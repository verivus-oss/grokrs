//! CLI subcommands for inspecting store state.
//!
//! `grokrs store status` — database path, schema version, WAL mode, session count, total cost.
//! `grokrs store usage [--session <id>]` — token usage and cost summary.
//! `grokrs store cost` — cost breakdowns by model, day, session, or endpoint.

use anyhow::{Context, Result};
use clap::{Subcommand, ValueEnum};
use grokrs_core::AppConfig;
use grokrs_store::cost::{self, CostFilter, CostGroupBy};
use grokrs_store::Store;
use std::path::Path;

/// Store subcommands for inspecting persistence state.
#[derive(Subcommand)]
pub enum StoreCommand {
    /// Print database path, schema version, WAL mode, session count, total cost
    Status,
    /// Print token usage and cost summary
    Usage {
        /// Filter usage to a specific session ID
        #[arg(long)]
        session: Option<String>,
    },
    /// Cost breakdowns by model, day, session, or endpoint
    Cost {
        /// Grouping dimension for the cost report
        #[arg(long, value_enum, default_value_t = CostGroupByArg::Model)]
        group_by: CostGroupByArg,

        /// Output format
        #[arg(long, value_enum, default_value_t = OutputFormat::Table)]
        format: OutputFormat,

        /// Only include transcripts at or after this date (YYYY-MM-DD or RFC 3339)
        #[arg(long)]
        since: Option<String>,

        /// Only include transcripts at or before this date (YYYY-MM-DD or RFC 3339)
        #[arg(long)]
        until: Option<String>,

        /// Filter to a single session ID
        #[arg(long)]
        session: Option<String>,
    },
}

/// CLI argument mapping for cost grouping dimension.
#[derive(Clone, Copy, ValueEnum)]
pub enum CostGroupByArg {
    /// Group by model name (from request body JSON)
    Model,
    /// Group by date (YYYY-MM-DD)
    Day,
    /// Group by session ID
    Session,
    /// Group by API endpoint
    Endpoint,
}

impl From<CostGroupByArg> for CostGroupBy {
    fn from(arg: CostGroupByArg) -> Self {
        match arg {
            CostGroupByArg::Model => CostGroupBy::Model,
            CostGroupByArg::Day => CostGroupBy::Day,
            CostGroupByArg::Session => CostGroupBy::Session,
            CostGroupByArg::Endpoint => CostGroupBy::Endpoint,
        }
    }
}

/// Output format for cost reports.
#[derive(Clone, Copy, ValueEnum)]
pub enum OutputFormat {
    /// Human-readable fixed-width table
    Table,
    /// JSON object with rows and summary
    Json,
    /// Comma-separated values with header
    Csv,
}

/// Execute a store subcommand.
pub fn run(command: &StoreCommand, config: &AppConfig, workspace_root: &Path) -> Result<()> {
    match command {
        StoreCommand::Status => run_status(config, workspace_root),
        StoreCommand::Usage { session } => run_usage(config, workspace_root, session.as_deref()),
        StoreCommand::Cost {
            group_by,
            format,
            since,
            until,
            session,
        } => run_cost(
            config,
            workspace_root,
            (*group_by).into(),
            *format,
            since.as_deref(),
            until.as_deref(),
            session.as_deref(),
        ),
    }
}

/// Open the store from config, returning None if the database does not exist yet.
fn open_store(config: &AppConfig, workspace_root: &Path) -> Result<Option<Store>> {
    let store_path = config
        .store
        .as_ref()
        .map(|s| s.path.as_str())
        .unwrap_or(".grokrs/state.db");

    let db_full_path = workspace_root.join(store_path);

    if !db_full_path.exists() {
        return Ok(None);
    }

    let store = Store::open_with_path(workspace_root, store_path)
        .context("failed to open store database")?;
    Ok(Some(store))
}

fn run_status(config: &AppConfig, workspace_root: &Path) -> Result<()> {
    let store_path = config
        .store
        .as_ref()
        .map(|s| s.path.as_str())
        .unwrap_or(".grokrs/state.db");

    match open_store(config, workspace_root)? {
        Some(store) => {
            let version = store
                .schema_version()
                .context("failed to read schema version")?;
            let total_sessions = store
                .sessions()
                .count_total()
                .context("failed to count total sessions")?;
            let active = store
                .sessions()
                .list_active()
                .context("failed to list active sessions")?;
            let totals = store
                .usage()
                .all_totals()
                .context("failed to compute usage totals")?;

            println!("Store status:");
            println!("  database:       {}", store.db_path().display());
            println!("  schema_version: {version}");
            println!("  journal_mode:   wal");
            println!(
                "  sessions:       {total_sessions} total, {} active",
                active.len()
            );
            println!(
                "  total_cost:     ${:.6} ({} ticks)",
                ticks_to_usd(totals.total_cost_ticks),
                totals.total_cost_ticks
            );
            println!("  total_requests: {}", totals.request_count);

            store.close().ok();
        }
        None => {
            println!("Store status:");
            println!(
                "  database: {} (not created yet)",
                workspace_root.join(store_path).display()
            );
            println!("  No store database exists. Run an API command to create one.");
        }
    }
    Ok(())
}

fn run_usage(config: &AppConfig, workspace_root: &Path, session_id: Option<&str>) -> Result<()> {
    match open_store(config, workspace_root)? {
        Some(store) => {
            let summary = match session_id {
                Some(sid) => store
                    .usage()
                    .session_totals(sid)
                    .context("failed to compute session usage")?,
                None => store
                    .usage()
                    .all_totals()
                    .context("failed to compute total usage")?,
            };

            if let Some(sid) = session_id {
                println!("Usage for session: {sid}");
            } else {
                println!("Usage across all sessions:");
            }
            println!("  requests:         {}", summary.request_count);
            println!("  input_tokens:     {}", summary.total_input_tokens);
            println!("  output_tokens:    {}", summary.total_output_tokens);
            println!("  reasoning_tokens: {}", summary.total_reasoning_tokens);
            println!(
                "  total_cost:       ${:.6} ({} ticks)",
                ticks_to_usd(summary.total_cost_ticks),
                summary.total_cost_ticks
            );

            store.close().ok();
        }
        None => {
            println!("No store database exists. Run an API command to create one.");
        }
    }
    Ok(())
}

fn run_cost(
    config: &AppConfig,
    workspace_root: &Path,
    group_by: CostGroupBy,
    format: OutputFormat,
    since: Option<&str>,
    until: Option<&str>,
    session_id: Option<&str>,
) -> Result<()> {
    match open_store(config, workspace_root)? {
        Some(store) => {
            let filter = CostFilter {
                since: since.map(|s| s.to_owned()),
                until: until.map(|s| s.to_owned()),
                session_id: session_id.map(|s| s.to_owned()),
            };

            let rows = store
                .cost()
                .aggregate(group_by, &filter)
                .context("failed to aggregate cost data")?;

            let summary = store
                .cost()
                .summary(&filter)
                .context("failed to compute cost summary")?;

            let output = match format {
                OutputFormat::Table => cost::format_table(group_by, &rows, &summary),
                OutputFormat::Json => cost::format_json(&rows, &summary)
                    .context("failed to format cost data as JSON")?,
                OutputFormat::Csv => cost::format_csv(group_by, &rows),
            };

            println!("{output}");

            store.close().ok();
        }
        None => {
            println!("No usage data found.");
        }
    }
    Ok(())
}

/// Convert cost ticks to USD for human-readable display.
///
/// The xAI API convention: ticks are cents per 100M tokens. This is a display
/// helper; the exact conversion semantics depend on the API pricing model.
fn ticks_to_usd(ticks: i64) -> f64 {
    ticks as f64 / 1_000_000.0
}

/// Report store status for the `grokrs doctor` command.
///
/// Opens the store read-only (or handles the case where it does not exist
/// gracefully). Prints database path, schema version, session count, and
/// total cost.
pub fn doctor_report(config: &AppConfig, workspace_root: &Path) {
    let store_path = config
        .store
        .as_ref()
        .map(|s| s.path.as_str())
        .unwrap_or(".grokrs/state.db");

    let db_full_path = workspace_root.join(store_path);

    if !db_full_path.exists() {
        println!("store=not_created path={}", db_full_path.display());
        return;
    }

    match Store::open_with_path(workspace_root, store_path) {
        Ok(store) => {
            let version = store.schema_version().unwrap_or(0);
            let total_count = store.sessions().count_total().unwrap_or(0);
            let active_count = store.sessions().list_active().map(|v| v.len()).unwrap_or(0);
            let totals = store.usage().all_totals().ok();
            let cost_ticks = totals.as_ref().map(|t| t.total_cost_ticks).unwrap_or(0);

            println!(
                "store=ok path={} schema_version={version} total_sessions={total_count} active_sessions={active_count} total_cost_ticks={cost_ticks}",
                store.db_path().display()
            );
            store.close().ok();
        }
        Err(e) => {
            println!("store=error path={} error={e}", db_full_path.display());
        }
    }
}
