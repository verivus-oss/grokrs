//! CLI subcommands for session management.
//!
//! `grokrs sessions list` — show all sessions with state, trust level, timestamps, transcript count.
//! `grokrs sessions show <id>` — session detail with transcript summary (prefix match).
//! `grokrs sessions transcript <id>` — full transcript for a session.
//! `grokrs sessions clean` — delete old closed/failed sessions with confirmation.

use std::fmt::Write as _;

use anyhow::{Context, Result, bail};
use clap::Subcommand;
use grokrs_core::AppConfig;
use grokrs_store::Store;
use grokrs_store::types::SessionRecord;
use std::io::{self, BufRead, Write};
use std::path::Path;

/// Sessions subcommands for browsing and managing stored sessions.
#[derive(Subcommand)]
#[command(after_help = "\
Examples:
  grokrs sessions list                   List all sessions
  grokrs sessions list --active          Show only active sessions
  grokrs sessions show abc123            Show session details (prefix match)
  grokrs sessions transcript abc123      Show full transcript
  grokrs sessions clean --older-than 7d  Delete old closed/failed sessions
  grokrs sessions clean --yes            Skip confirmation prompt

See also: grokrs chat --resume")]
pub enum SessionsCommand {
    /// Show all sessions with state, trust level, timestamps, transcript count
    List {
        /// Show only active (non-Closed, non-Failed) sessions
        #[arg(long)]
        active: bool,

        /// Filter by exact state (e.g. "Ready", "Closed", "RunningTurn")
        #[arg(long)]
        state: Option<String>,

        /// Maximum number of rows to display
        #[arg(long)]
        limit: Option<u32>,
    },

    /// Show full detail for a session (supports prefix match on ID)
    Show {
        /// Session ID (or unique prefix)
        id: String,
    },

    /// Show the full transcript for a session (supports prefix match on ID)
    Transcript {
        /// Session ID (or unique prefix)
        id: String,

        /// Maximum bytes to show for response/request bodies (default 500)
        #[arg(long, default_value = "500")]
        max_body: usize,
    },

    /// Delete old sessions in Closed or Failed state
    Clean {
        /// Delete sessions older than this duration (e.g. 7d, 24h, 30m). Default: 7d
        #[arg(long, default_value = "7d")]
        older_than: String,

        /// Skip confirmation prompt
        #[arg(long)]
        yes: bool,
    },
}

/// Execute a sessions subcommand.
pub fn run(command: &SessionsCommand, config: &AppConfig, workspace_root: &Path) -> Result<()> {
    match command {
        SessionsCommand::List {
            active,
            state,
            limit,
        } => run_list(config, workspace_root, *active, state.as_deref(), *limit),
        SessionsCommand::Show { id } => run_show(config, workspace_root, id),
        SessionsCommand::Transcript { id, max_body } => {
            run_transcript(config, workspace_root, id, *max_body)
        }
        SessionsCommand::Clean { older_than, yes } => {
            run_clean(config, workspace_root, older_than, *yes)
        }
    }
}

/// Open the store, returning a helpful error if it doesn't exist.
fn open_store(config: &AppConfig, workspace_root: &Path) -> Result<Store> {
    let store_path = config
        .store
        .as_ref()
        .map_or(".grokrs/state.db", |s| s.path.as_str());

    let db_full_path = workspace_root.join(store_path);

    if !db_full_path.exists() {
        bail!(
            "No store database found at {}.\n\
             Run an API command to create one, or check your configuration.",
            db_full_path.display()
        );
    }

    Store::open_with_path(workspace_root, store_path).context("failed to open store database")
}

/// Resolve a session ID, supporting prefix matching.
///
/// If the given `id_or_prefix` is a full UUID match, returns it directly.
/// Otherwise, uses `find_by_prefix` and requires exactly one match.
fn resolve_session_id(store: &Store, id_or_prefix: &str) -> Result<SessionRecord> {
    // First try exact match.
    if let Some(record) = store
        .sessions()
        .get(id_or_prefix)
        .context("failed to look up session")?
    {
        return Ok(record);
    }

    // Try prefix match.
    let matches = store
        .sessions()
        .find_by_prefix(id_or_prefix)
        .context("failed to search sessions by prefix")?;

    match matches.len() {
        0 => bail!("No session found matching '{id_or_prefix}'."),
        1 => Ok(matches.into_iter().next().unwrap()),
        n => {
            let mut msg =
                format!("Ambiguous session ID prefix '{id_or_prefix}' matches {n} sessions:\n");
            for s in &matches {
                write!(
                    msg,
                    "  {}  state={}  updated={}\n",
                    &s.id[..s.id.len().min(12)],
                    s.state,
                    s.updated_at,
                )
                .unwrap();
            }
            msg.push_str("\nPlease provide a longer prefix to uniquely identify the session.");
            bail!("{msg}");
        }
    }
}

/// Truncate ID to first 8 characters for display.
fn short_id(id: &str) -> &str {
    &id[..id.len().min(8)]
}

/// Truncate a string to at most `max_bytes` bytes, appending a note if truncated.
fn truncate_body(body: &str, max_bytes: usize) -> String {
    if body.len() <= max_bytes {
        body.to_owned()
    } else {
        // Find the last valid UTF-8 boundary at or before max_bytes.
        let mut end = max_bytes;
        while end > 0 && !body.is_char_boundary(end) {
            end -= 1;
        }
        format!(
            "{}... [truncated, {} total bytes]",
            &body[..end],
            body.len()
        )
    }
}

fn run_list(
    config: &AppConfig,
    workspace_root: &Path,
    active: bool,
    state: Option<&str>,
    limit: Option<u32>,
) -> Result<()> {
    let store = open_store(config, workspace_root)?;

    let sessions = if active {
        store
            .sessions()
            .list_active()
            .context("failed to list active sessions")?
    } else if let Some(st) = state {
        store
            .sessions()
            .list_by_state(st)
            .context("failed to list sessions by state")?
    } else {
        store
            .sessions()
            .list_all(limit)
            .context("failed to list all sessions")?
    };

    // Apply limit to active/state filters (list_all already has built-in limit).
    let sessions: Vec<_> = if let (true, Some(n)) = (active || state.is_some(), limit) {
        sessions.into_iter().take(n as usize).collect()
    } else {
        sessions
    };

    if sessions.is_empty() {
        if active {
            println!("No active sessions found.");
        } else if let Some(st) = state {
            println!("No sessions found with state '{st}'.");
        } else {
            println!("No sessions found. Run a chat or API command to create one.");
        }
        store.close().ok();
        return Ok(());
    }

    // Print table header.
    println!(
        "{:<10} {:<18} {:<15} {:<22} {:<22} {:>6}",
        "ID", "STATE", "TRUST", "CREATED", "UPDATED", "TURNS"
    );
    println!("{}", "-".repeat(95));

    for session in &sessions {
        let transcript_count = store.sessions().count_transcripts(&session.id).unwrap_or(0);

        // Truncate state for display (Failed: ... can be long).
        let state_display = if session.state.len() > 16 {
            format!("{}...", &session.state[..13])
        } else {
            session.state.clone()
        };

        println!(
            "{:<10} {:<18} {:<15} {:<22} {:<22} {:>6}",
            short_id(&session.id),
            state_display,
            session.trust_level,
            session.created_at,
            session.updated_at,
            transcript_count,
        );
    }

    println!("\n{} session(s) shown.", sessions.len());
    store.close().ok();
    Ok(())
}

fn run_show(config: &AppConfig, workspace_root: &Path, id: &str) -> Result<()> {
    let store = open_store(config, workspace_root)?;
    let session = resolve_session_id(&store, id)?;
    let transcript_count = store.sessions().count_transcripts(&session.id).unwrap_or(0);

    println!("Session detail:");
    println!("  id:          {}", session.id);
    println!("  state:       {}", session.state);
    println!("  trust_level: {}", session.trust_level);
    println!("  created_at:  {}", session.created_at);
    println!("  updated_at:  {}", session.updated_at);
    println!("  transcripts: {transcript_count}");

    // Show usage summary if there are transcripts.
    if transcript_count > 0
        && let Ok(summary) = store.usage().session_totals(&session.id)
    {
        println!();
        println!("Usage summary:");
        println!("  requests:         {}", summary.request_count);
        println!("  input_tokens:     {}", summary.total_input_tokens);
        println!("  output_tokens:    {}", summary.total_output_tokens);
        println!("  reasoning_tokens: {}", summary.total_reasoning_tokens);
        println!(
            "  total_cost:       ${:.6} ({} ticks)",
            ticks_to_usd(summary.total_cost_ticks),
            summary.total_cost_ticks
        );
    }

    store.close().ok();
    Ok(())
}

fn run_transcript(
    config: &AppConfig,
    workspace_root: &Path,
    id: &str,
    max_body: usize,
) -> Result<()> {
    let store = open_store(config, workspace_root)?;
    let session = resolve_session_id(&store, id)?;

    let transcripts = store
        .transcripts()
        .list_by_session(&session.id)
        .context("failed to list transcripts")?;

    if transcripts.is_empty() {
        println!(
            "No transcripts found for session {}.",
            short_id(&session.id)
        );
        store.close().ok();
        return Ok(());
    }

    println!(
        "Transcript for session {} ({} entries):",
        short_id(&session.id),
        transcripts.len()
    );
    println!();

    for (i, t) in transcripts.iter().enumerate() {
        println!("--- Turn {} ---", i + 1);
        println!("  request_at:  {}", t.request_at);
        println!("  endpoint:    {} {}", t.method, t.endpoint);

        if let Some(ref body) = t.request_body {
            println!("  request:     {}", truncate_body(body, max_body));
        }

        if let Some(ref response_at) = t.response_at {
            println!("  response_at: {response_at}");
        }

        if let Some(code) = t.status_code {
            println!("  status:      {code}");
        }

        if let Some(ref body) = t.response_body {
            println!("  response:    {}", truncate_body(body, max_body));
        }

        if let Some(ref err) = t.error {
            println!("  error:       {err}");
        }

        // Usage for this turn.
        let parts: Vec<String> = [
            t.input_tokens.map(|v| format!("input={v}")),
            t.output_tokens.map(|v| format!("output={v}")),
            t.reasoning_tokens.map(|v| format!("reasoning={v}")),
        ]
        .into_iter()
        .flatten()
        .collect();
        if !parts.is_empty() {
            println!("  usage:       {}", parts.join(", "));
        }

        if let Some(ref rid) = t.response_id {
            println!("  response_id: {rid}");
        }

        println!();
    }

    store.close().ok();
    Ok(())
}

fn run_clean(
    config: &AppConfig,
    workspace_root: &Path,
    older_than: &str,
    skip_confirm: bool,
) -> Result<()> {
    let store = open_store(config, workspace_root)?;

    // Parse the duration string.
    let before_ts = compute_cutoff_timestamp(older_than)?;

    // Count how many would be affected (preview).
    // We use the same query as delete_old but just to give a count first.
    // For simplicity, we'll query all Closed+Failed sessions older than cutoff.
    let all_sessions = store
        .sessions()
        .list_all(None)
        .context("failed to list sessions")?;

    let deletable_count = all_sessions
        .iter()
        .filter(|s| {
            (s.state == "Closed" || s.state.starts_with("Failed")) && s.updated_at < before_ts
        })
        .count();

    if deletable_count == 0 {
        println!("No sessions to clean (no Closed/Failed sessions older than {older_than}).");
        store.close().ok();
        return Ok(());
    }

    println!("Found {deletable_count} Closed/Failed session(s) older than {older_than} to delete.");
    println!("Associated transcripts will also be deleted (cascade).");

    if !skip_confirm {
        print!("Proceed? [y/N] ");
        io::stdout().flush().ok();
        let stdin = io::stdin();
        let mut line = String::new();
        stdin.lock().read_line(&mut line).ok();
        let answer = line.trim().to_ascii_lowercase();
        if answer != "y" && answer != "yes" {
            println!("Aborted.");
            store.close().ok();
            return Ok(());
        }
    }

    let deleted = store
        .sessions()
        .delete_old(&before_ts)
        .context("failed to delete old sessions")?;

    println!("Deleted {deleted} session(s) and their associated transcripts.");
    store.close().ok();
    Ok(())
}

/// Parse a human-readable duration string (e.g. "7d", "24h", "30m") and return
/// an RFC 3339 timestamp representing `now - duration`.
fn compute_cutoff_timestamp(duration_str: &str) -> Result<String> {
    let trimmed = duration_str.trim();
    if trimmed.is_empty() {
        bail!("Empty duration string");
    }

    let (num_str, unit) = if let Some(s) = trimmed.strip_suffix('d') {
        (s, 'd')
    } else if let Some(s) = trimmed.strip_suffix('h') {
        (s, 'h')
    } else if let Some(s) = trimmed.strip_suffix('m') {
        (s, 'm')
    } else {
        bail!(
            "Invalid duration format '{trimmed}'. Expected format: <number><unit> where unit is d (days), h (hours), or m (minutes). Examples: 7d, 24h, 30m"
        );
    };

    let num: u64 = num_str
        .parse()
        .with_context(|| format!("Invalid number in duration '{trimmed}'"))?;

    let seconds = match unit {
        'd' => num * 86400,
        'h' => num * 3600,
        'm' => num * 60,
        _ => unreachable!(),
    };

    // Compute the cutoff as now - seconds.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .expect("system clock before UNIX epoch")
        .as_secs();

    let cutoff_secs = now.saturating_sub(seconds);
    Ok(epoch_to_rfc3339(cutoff_secs))
}

/// Convert a UNIX epoch timestamp (seconds) to RFC 3339 format.
fn epoch_to_rfc3339(secs: u64) -> String {
    let days = secs / 86400;
    let time_of_day = secs % 86400;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;

    let (year, month, day) = days_to_ymd(days);

    format!("{year:04}-{month:02}-{day:02}T{hours:02}:{minutes:02}:{seconds:02}Z")
}

/// Convert days since UNIX epoch (1970-01-01) to (year, month, day).
/// Algorithm from Howard Hinnant's `civil_from_days`.
fn days_to_ymd(days: u64) -> (u64, u64, u64) {
    let z = days as i64 + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };
    (year as u64, m, d)
}

/// Convert cost ticks to USD for human-readable display.
fn ticks_to_usd(ticks: i64) -> f64 {
    ticks as f64 / 1_000_000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // Duration parsing
    // -----------------------------------------------------------------------

    #[test]
    fn parse_duration_days() {
        let ts = compute_cutoff_timestamp("7d").unwrap();
        assert!(ts.ends_with('Z'));
        assert_eq!(ts.len(), 20);
    }

    #[test]
    fn parse_duration_hours() {
        let ts = compute_cutoff_timestamp("24h").unwrap();
        assert!(ts.ends_with('Z'));
    }

    #[test]
    fn parse_duration_minutes() {
        let ts = compute_cutoff_timestamp("30m").unwrap();
        assert!(ts.ends_with('Z'));
    }

    #[test]
    fn parse_duration_invalid_format() {
        assert!(compute_cutoff_timestamp("7x").is_err());
        assert!(compute_cutoff_timestamp("").is_err());
        assert!(compute_cutoff_timestamp("abc").is_err());
    }

    #[test]
    fn parse_duration_invalid_number() {
        assert!(compute_cutoff_timestamp("xd").is_err());
    }

    // -----------------------------------------------------------------------
    // Epoch to RFC 3339
    // -----------------------------------------------------------------------

    #[test]
    fn epoch_zero_is_1970() {
        let ts = epoch_to_rfc3339(0);
        assert_eq!(ts, "1970-01-01T00:00:00Z");
    }

    #[test]
    fn epoch_known_date() {
        // 2024-01-01 00:00:00 UTC = 1704067200
        let ts = epoch_to_rfc3339(1_704_067_200);
        assert_eq!(ts, "2024-01-01T00:00:00Z");
    }

    // -----------------------------------------------------------------------
    // Body truncation
    // -----------------------------------------------------------------------

    #[test]
    fn truncate_short_body() {
        assert_eq!(truncate_body("hello", 100), "hello");
    }

    #[test]
    fn truncate_long_body() {
        let body = "a".repeat(1000);
        let result = truncate_body(&body, 100);
        assert!(result.len() < 1000);
        assert!(result.contains("truncated"));
        assert!(result.contains("1000 total bytes"));
    }

    #[test]
    fn truncate_empty_body() {
        assert_eq!(truncate_body("", 100), "");
    }

    // -----------------------------------------------------------------------
    // Short ID
    // -----------------------------------------------------------------------

    #[test]
    fn short_id_truncates_uuid() {
        let id = "a1b2c3d4-e5f6-7890-abcd-ef1234567890";
        assert_eq!(short_id(id), "a1b2c3d4");
    }

    #[test]
    fn short_id_short_string() {
        assert_eq!(short_id("abc"), "abc");
    }

    // -----------------------------------------------------------------------
    // Integration tests with temp store
    // -----------------------------------------------------------------------

    fn make_test_store() -> (tempfile::TempDir, Store) {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(tmp.path()).unwrap();
        (tmp, store)
    }

    #[test]
    fn resolve_exact_match() {
        let (_tmp, store) = make_test_store();
        store
            .sessions()
            .create("test-session-abc123", "Untrusted")
            .unwrap();

        let result = resolve_session_id(&store, "test-session-abc123").unwrap();
        assert_eq!(result.id, "test-session-abc123");
    }

    #[test]
    fn resolve_prefix_match() {
        let (_tmp, store) = make_test_store();
        store
            .sessions()
            .create("test-session-abc123", "Untrusted")
            .unwrap();

        let result = resolve_session_id(&store, "test-session").unwrap();
        assert_eq!(result.id, "test-session-abc123");
    }

    #[test]
    fn resolve_ambiguous_prefix() {
        let (_tmp, store) = make_test_store();
        store
            .sessions()
            .create("test-session-001", "Untrusted")
            .unwrap();
        store
            .sessions()
            .create("test-session-002", "Untrusted")
            .unwrap();

        let result = resolve_session_id(&store, "test-session");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Ambiguous"), "got: {err}");
    }

    #[test]
    fn resolve_no_match() {
        let (_tmp, store) = make_test_store();
        let result = resolve_session_id(&store, "nonexistent");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("No session found"), "got: {err}");
    }

    #[test]
    fn list_sessions_empty_store() {
        let (tmp, store) = make_test_store();
        store.close().ok();

        let config = make_test_config();
        let result = run_list(&config, tmp.path(), false, None, None);
        assert!(result.is_ok());
    }

    #[test]
    fn list_sessions_with_data() {
        let (tmp, store) = make_test_store();
        store.sessions().create("session-001", "Untrusted").unwrap();
        store
            .sessions()
            .create("session-002", "AdminTrusted")
            .unwrap();
        store.close().ok();

        let config = make_test_config();
        let result = run_list(&config, tmp.path(), false, None, None);
        assert!(result.is_ok());
    }

    #[test]
    fn show_session_with_transcripts() {
        let (tmp, store) = make_test_store();
        store
            .sessions()
            .create("session-show-test", "Untrusted")
            .unwrap();
        store
            .transcripts()
            .log_request("session-show-test", "/v1/responses", "POST", Some("hello"))
            .unwrap();
        store.close().ok();

        let config = make_test_config();
        let result = run_show(&config, tmp.path(), "session-show");
        assert!(result.is_ok());
    }

    #[test]
    fn transcript_display() {
        let (tmp, store) = make_test_store();
        store
            .sessions()
            .create("session-tx-test", "Untrusted")
            .unwrap();
        let tid = store
            .transcripts()
            .log_request(
                "session-tx-test",
                "/v1/responses",
                "POST",
                Some("{\"prompt\":\"hi\"}"),
            )
            .unwrap();
        let usage = grokrs_store::types::TranscriptUsage {
            cost_in_usd_ticks: None,
            input_tokens: Some(10),
            output_tokens: Some(20),
            reasoning_tokens: None,
        };
        store
            .transcripts()
            .log_response(tid, 200, Some("Hello!"), &usage, Some("resp_abc"))
            .unwrap();
        store.close().ok();

        let config = make_test_config();
        let result = run_transcript(&config, tmp.path(), "session-tx", 500);
        assert!(result.is_ok());
    }

    #[test]
    fn clean_no_sessions() {
        let (tmp, store) = make_test_store();
        store.close().ok();

        let config = make_test_config();
        // With --yes to skip interactive prompt.
        let result = run_clean(&config, tmp.path(), "7d", true);
        assert!(result.is_ok());
    }

    #[test]
    fn clean_skips_active_sessions() {
        let (tmp, store) = make_test_store();
        store
            .sessions()
            .create("active-session", "Untrusted")
            .unwrap();
        store
            .sessions()
            .transition("active-session", "Ready")
            .unwrap();
        store.close().ok();

        let config = make_test_config();
        let result = run_clean(&config, tmp.path(), "0m", true);
        assert!(result.is_ok());
        // Verify session still exists.
        let store2 = Store::open(tmp.path()).unwrap();
        let s = store2.sessions().get("active-session").unwrap();
        assert!(s.is_some(), "active session should not be deleted");
    }

    fn make_test_config() -> AppConfig {
        use grokrs_core::{ModelConfig, PolicyConfig, SessionConfig, WorkspaceConfig};
        AppConfig {
            workspace: WorkspaceConfig {
                name: "test".into(),
                root: ".".into(),
            },
            model: ModelConfig {
                provider: "xai".into(),
                default_model: "grok-4".into(),
            },
            policy: PolicyConfig {
                allow_network: false,
                allow_shell: false,
                allow_workspace_writes: true,
                max_patch_bytes: 1024,
            },
            session: SessionConfig {
                approval_mode: "interactive".into(),
                transcript_dir: ".grokrs/sessions".into(),
            },
            api: None,
            management_api: None,
            store: None,
            agent: None,
            chat: None,
            mcp: None,
        }
    }
}
