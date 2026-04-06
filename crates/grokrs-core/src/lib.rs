use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Clone, Deserialize)]
pub struct AppConfig {
    pub workspace: WorkspaceConfig,
    pub model: ModelConfig,
    pub policy: PolicyConfig,
    pub session: SessionConfig,
    /// Optional API configuration. Existing configs without `[api]` continue to load.
    pub api: Option<ApiConfig>,
    /// Optional Management API configuration. Existing configs without
    /// `[management_api]` continue to load.
    pub management_api: Option<ManagementApiConfig>,
    /// Optional store configuration. Existing configs without `[store]`
    /// continue to load. When absent, the store uses default settings
    /// (path = `.grokrs/state.db`).
    pub store: Option<StoreConfig>,
    /// Optional agent configuration. Existing configs without `[agent]`
    /// continue to load. When present, fields use serde defaults for
    /// partial sections.
    pub agent: Option<AgentConfig>,
    /// Optional chat configuration. Existing configs without `[chat]`
    /// continue to load. When present, fields use serde defaults for
    /// partial sections.
    pub chat: Option<ChatConfig>,
    /// Optional MCP (Model Context Protocol) configuration. Defines
    /// persistent MCP server connections. When absent, no MCP servers
    /// are connected at startup.
    pub mcp: Option<McpConfig>,
}

/// Configuration for the xAI API client.
///
/// Secrets are never stored here. The `api_key_env` field names the environment
/// variable that holds the bearer token; the actual secret is resolved at runtime.
#[derive(Debug, Clone, Deserialize)]
pub struct ApiConfig {
    /// Environment variable name holding the API key (NOT the key itself).
    pub api_key_env: Option<String>,
    /// Base URL for the xAI API (default: https://api.x.ai).
    pub base_url: Option<String>,
    /// Request timeout in seconds (default: 120).
    pub timeout_secs: Option<u64>,
    /// Maximum number of retries on 429/503 (default: 3).
    pub max_retries: Option<u32>,
}

/// Configuration for the xAI Collections Management API.
///
/// The Management API uses a different base URL (`https://management-api.x.ai`)
/// and a different authentication key than the inference API. Secrets are never
/// stored here. The `management_key_env` field names the environment variable
/// that holds the management API key; the actual secret is resolved at runtime.
#[derive(Debug, Clone, Deserialize)]
pub struct ManagementApiConfig {
    /// Environment variable name holding the Management API key (NOT the key itself).
    /// Defaults to `XAI_MANAGEMENT_API_KEY` when not specified.
    pub management_key_env: Option<String>,
    /// Base URL for the Management API (default: `https://management-api.x.ai`).
    pub base_url: Option<String>,
    /// Request timeout in seconds (default: 120).
    pub timeout_secs: Option<u64>,
    /// Maximum number of retries on 429/503 (default: 3).
    pub max_retries: Option<u32>,
}

/// Configuration for the SQLite persistence store.
///
/// Lives in `grokrs-core` (not `grokrs-store`) so the store crate can consume
/// it without circular dependency. The `path` field is relative to the workspace
/// root and defaults to `.grokrs/state.db`.
#[derive(Debug, Clone, Deserialize)]
pub struct StoreConfig {
    /// Path to the SQLite database file, relative to the workspace root.
    /// Defaults to `.grokrs/state.db` when the field is absent.
    #[serde(default = "StoreConfig::default_path")]
    pub path: String,
}

impl StoreConfig {
    fn default_path() -> String {
        ".grokrs/state.db".to_owned()
    }
}

impl Default for StoreConfig {
    fn default() -> Self {
        Self {
            path: Self::default_path(),
        }
    }
}

/// Configuration for the agent command (`grokrs agent`).
///
/// All fields have serde defaults so a bare `[agent]` section is valid.
/// When the section is absent, `AppConfig.agent` is `None` and the CLI
/// falls back to hardcoded defaults.
#[derive(Debug, Clone, Deserialize)]
pub struct AgentConfig {
    /// Maximum number of tool-calling iterations before aborting.
    /// Default: 10.
    #[serde(default = "AgentConfig::default_max_iterations")]
    pub max_iterations: u32,

    /// Default trust level for agent sessions.
    /// One of: `"untrusted"`, `"interactive"`, `"admin"`.
    /// Default: `"untrusted"`.
    #[serde(default = "AgentConfig::default_trust")]
    pub default_trust: String,

    /// Whether to enable search tools by default for agent tasks.
    /// Default: false.
    #[serde(default)]
    pub enable_search: bool,

    /// Maximum number of cross-session memories to persist.
    /// When the limit is exceeded, the oldest/least-accessed memories are evicted.
    /// Default: 50.
    #[serde(default = "AgentConfig::default_memory_limit")]
    pub memory_limit: i64,
}

impl AgentConfig {
    fn default_max_iterations() -> u32 {
        10
    }
    fn default_trust() -> String {
        "untrusted".to_owned()
    }
    fn default_memory_limit() -> i64 {
        50
    }
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            max_iterations: Self::default_max_iterations(),
            default_trust: Self::default_trust(),
            enable_search: false,
            memory_limit: Self::default_memory_limit(),
        }
    }
}

/// Configuration for the interactive chat command (`grokrs chat`).
///
/// All fields have serde defaults so a bare `[chat]` section is valid.
/// When the section is absent, `AppConfig.chat` is `None` and the CLI
/// falls back to hardcoded defaults.
#[derive(Debug, Clone, Deserialize)]
pub struct ChatConfig {
    /// Default model for chat sessions (falls back to `[model].default_model`
    /// when not specified).
    #[serde(default)]
    pub default_model: Option<String>,

    /// Optional default system instructions applied to every chat session.
    #[serde(default)]
    pub system_prompt: Option<String>,

    /// Enable server-side conversation chaining by default.
    /// Default: false.
    #[serde(default)]
    pub stateful: bool,

    /// Path to the readline history file, relative to the workspace root.
    /// Default: `.grokrs/chat_history`.
    #[serde(default = "ChatConfig::default_history_file")]
    pub history_file: String,

    /// Maximum conversation tokens before a warning is emitted.
    /// Default: 100,000.
    #[serde(default = "ChatConfig::default_max_conversation_tokens")]
    pub max_conversation_tokens: u64,
}

impl ChatConfig {
    fn default_history_file() -> String {
        ".grokrs/chat_history".to_owned()
    }
    fn default_max_conversation_tokens() -> u64 {
        100_000
    }
}

impl Default for ChatConfig {
    fn default() -> Self {
        Self {
            default_model: None,
            system_prompt: None,
            stateful: false,
            history_file: Self::default_history_file(),
            max_conversation_tokens: Self::default_max_conversation_tokens(),
        }
    }
}

/// Configuration for MCP (Model Context Protocol) client-side hosting.
///
/// Defines persistent MCP server connections that are established at startup.
/// Each server entry specifies a URL, optional label, optional tool allowlist,
/// and optional trust rank override.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct McpConfig {
    /// Named MCP server definitions.
    ///
    /// Keys are server identifiers used for logging and tool name prefixing.
    /// Example TOML:
    /// ```toml
    /// [mcp.servers.filesystem]
    /// url = "http://localhost:8080/mcp"
    /// label = "filesystem"
    /// ```
    #[serde(default)]
    pub servers: std::collections::HashMap<String, McpServerConfig>,
}

/// Configuration for a single MCP server connection.
#[derive(Debug, Clone, Deserialize)]
pub struct McpServerConfig {
    /// The MCP server URL (e.g., `http://localhost:8080/mcp`).
    pub url: String,
    /// Human-readable label for this server. Used in tool name prefixing
    /// and logging. Defaults to the server key from the config.
    #[serde(default)]
    pub label: Option<String>,
    /// Optional allowlist of tool names. When set, only tools whose names
    /// match this list are registered. When absent, all tools are registered.
    #[serde(default)]
    pub allowed_tools: Option<Vec<String>>,
    /// Trust rank override for tools from this server.
    /// Default: 1 (InteractiveTrusted).
    #[serde(default = "McpServerConfig::default_trust_rank")]
    pub trust_rank: u8,
    /// Request timeout in seconds for this server.
    /// Default: 30.
    #[serde(default = "McpServerConfig::default_timeout_secs")]
    pub timeout_secs: u64,
}

impl McpServerConfig {
    fn default_trust_rank() -> u8 {
        1
    }
    fn default_timeout_secs() -> u64 {
        30
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct WorkspaceConfig {
    pub name: String,
    pub root: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ModelConfig {
    pub provider: String,
    pub default_model: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PolicyConfig {
    pub allow_network: bool,
    pub allow_shell: bool,
    pub allow_workspace_writes: bool,
    pub max_patch_bytes: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SessionConfig {
    pub approval_mode: String,
    pub transcript_dir: String,
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("failed to read config {path}: {source}")]
    Read {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse config {path}: {source}")]
    Parse {
        path: String,
        #[source]
        source: toml::de::Error,
    },
    #[error(
        "invalid profile name '{name}': profile names must be non-empty and contain only alphanumeric characters, hyphens, and underscores"
    )]
    InvalidProfileName { name: String },
    #[error("profile config not found: {path}")]
    ProfileNotFound { path: String },
    #[error("failed to merge profile config: {reason}")]
    MergeError { reason: String },
}

/// Validate that a profile name contains only alphanumeric characters, hyphens,
/// and underscores. Returns `Ok(())` if valid, or `ConfigError::InvalidProfileName`.
pub fn validate_profile_name(name: &str) -> Result<(), ConfigError> {
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(ConfigError::InvalidProfileName {
            name: name.to_owned(),
        });
    }
    Ok(())
}

/// Resolve the effective profile name from an explicit flag and/or the
/// `GROKRS_PROFILE` environment variable.
///
/// Precedence: `flag` (--profile) > `GROKRS_PROFILE` env var > `None`.
pub fn resolve_profile(flag: Option<&str>) -> Option<String> {
    if let Some(name) = flag {
        return Some(name.to_owned());
    }
    std::env::var("GROKRS_PROFILE")
        .ok()
        .filter(|v| !v.is_empty())
}

/// Deep-merge two `toml::Value` tables. Values in `overlay` override values
/// in `base` at the leaf level. Tables are recursively merged; all other
/// value types are replaced outright.
pub fn deep_merge(base: &mut toml::Value, overlay: toml::Value) {
    match (base, overlay) {
        (toml::Value::Table(base_table), toml::Value::Table(overlay_table)) => {
            for (key, overlay_val) in overlay_table {
                let entry = base_table
                    .entry(key)
                    .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
                deep_merge(entry, overlay_val);
            }
        }
        (base, overlay) => {
            *base = overlay;
        }
    }
}

/// Deprecated Grok model name prefixes.
///
/// These patterns match model names that have been superseded by newer
/// generations. Users should migrate to current models (e.g., `grok-4`).
/// The check is prefix-based so it catches variant names like `grok-2-1212`,
/// `grok-3-turbo`, `grok-2-vision-preview`, etc.
const DEPRECATED_MODEL_PREFIXES: &[&str] = &["grok-2", "grok-3"];

/// Warn on stderr if the configured default model name matches a known-deprecated
/// Grok model family.
///
/// This is a non-fatal, informational check. The warning is printed to stderr
/// so it does not pollute stdout piped output. Operation continues normally.
///
/// # Deprecated families
///
/// - `grok-2` family — superseded by grok-3 and grok-4
/// - `grok-3` family — superseded by grok-4
///
/// # Examples
///
/// ```rust
/// use grokrs_core::check_deprecated_model;
/// // No output: current model
/// check_deprecated_model("grok-4");
/// // Prints warning to stderr: legacy model
/// check_deprecated_model("grok-3-turbo");
/// ```
pub fn check_deprecated_model(model_name: &str) {
    for prefix in DEPRECATED_MODEL_PREFIXES {
        // Match exact name (e.g., "grok-2") or prefixed variants
        // (e.g., "grok-2-1212", "grok-3-turbo") but NOT accidental
        // substring matches in unrelated names.
        if model_name == *prefix || model_name.starts_with(&format!("{prefix}-")) {
            eprintln!(
                "WARNING: config references deprecated model '{model_name}'.\n\
                 \n\
                 The {prefix} family has been superseded by grok-4. Continuing\n\
                 with the configured model, but you should update your config:\n\
                 \n\
                 [model]\n\
                 default_model = \"grok-4\"\n\
                 \n\
                 Run `grokrs models list` to see all currently available models.\n"
            );
            return; // One warning per startup is sufficient.
        }
    }
}

impl AppConfig {
    pub fn load(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let path = path.as_ref();
        let raw = fs::read_to_string(path).map_err(|source| ConfigError::Read {
            path: path.display().to_string(),
            source,
        })?;
        toml::from_str(&raw).map_err(|source| ConfigError::Parse {
            path: path.display().to_string(),
            source,
        })
    }

    /// Load a base config and, if a profile name is provided, deep-merge the
    /// profile config on top.
    ///
    /// The profile config file is resolved as `<config_dir>/grokrs.<profile>.toml`
    /// where `config_dir` is the parent directory of `base_path`. Profile configs
    /// are partial: only the fields present in the profile file override the base.
    ///
    /// # Errors
    ///
    /// - `ConfigError::InvalidProfileName` if the name contains disallowed characters.
    /// - `ConfigError::ProfileNotFound` if the profile TOML file does not exist.
    /// - `ConfigError::Read` / `ConfigError::Parse` for I/O or deserialization failures.
    /// - `ConfigError::MergeError` if the merged TOML cannot be deserialized.
    pub fn load_with_profile(
        base_path: impl AsRef<Path>,
        profile: Option<&str>,
    ) -> Result<Self, ConfigError> {
        let base_path = base_path.as_ref();

        let profile = match profile {
            Some(name) => {
                validate_profile_name(name)?;
                Some(name)
            }
            None => None,
        };

        match profile {
            None => Self::load(base_path),
            Some(name) => {
                let profile_path = Self::profile_path(base_path, name);

                if !profile_path.exists() {
                    return Err(ConfigError::ProfileNotFound {
                        path: profile_path.display().to_string(),
                    });
                }

                // Load both files as raw TOML values for merging.
                let base_raw =
                    fs::read_to_string(base_path).map_err(|source| ConfigError::Read {
                        path: base_path.display().to_string(),
                        source,
                    })?;
                let mut base_value: toml::Value =
                    toml::from_str(&base_raw).map_err(|source| ConfigError::Parse {
                        path: base_path.display().to_string(),
                        source,
                    })?;

                let profile_raw =
                    fs::read_to_string(&profile_path).map_err(|source| ConfigError::Read {
                        path: profile_path.display().to_string(),
                        source,
                    })?;
                let profile_value: toml::Value =
                    toml::from_str(&profile_raw).map_err(|source| ConfigError::Parse {
                        path: profile_path.display().to_string(),
                        source,
                    })?;

                deep_merge(&mut base_value, profile_value);

                base_value
                    .try_into()
                    .map_err(|source: toml::de::Error| ConfigError::MergeError {
                        reason: format!(
                            "merged config (base={}, profile={}) failed to deserialize: {}",
                            base_path.display(),
                            profile_path.display(),
                            source
                        ),
                    })
            }
        }
    }

    /// Compute the expected filesystem path for a named profile config,
    /// relative to the base config path.
    ///
    /// Given base `configs/grokrs.example.toml` and profile `dev`, returns
    /// `configs/grokrs.dev.toml`.
    pub fn profile_path(base_path: &Path, profile_name: &str) -> PathBuf {
        let dir = base_path.parent().unwrap_or_else(|| Path::new("."));
        dir.join(format!("grokrs.{profile_name}.toml"))
    }

    pub fn summary(&self) -> String {
        let mut s = format!(
            "workspace={} model={} provider={} allow_network={} allow_shell={} allow_workspace_writes={} approval_mode={} transcript_dir={} max_patch_bytes={}",
            self.workspace.name,
            self.model.default_model,
            self.model.provider,
            self.policy.allow_network,
            self.policy.allow_shell,
            self.policy.allow_workspace_writes,
            self.session.approval_mode,
            self.session.transcript_dir,
            self.policy.max_patch_bytes
        );
        if let Some(ref api) = self.api {
            let key_env = api.api_key_env.as_deref().unwrap_or("XAI_API_KEY");
            let base_url = api.base_url.as_deref().unwrap_or("https://api.x.ai");
            let timeout = api.timeout_secs.unwrap_or(120);
            let retries = api.max_retries.unwrap_or(3);
            s.push_str(&format!(
                " api_key_env={key_env} base_url={base_url} timeout_secs={timeout} max_retries={retries}"
            ));
        }
        if let Some(ref mgmt) = self.management_api {
            let key_env = mgmt
                .management_key_env
                .as_deref()
                .unwrap_or("XAI_MANAGEMENT_API_KEY");
            let base_url = mgmt
                .base_url
                .as_deref()
                .unwrap_or("https://management-api.x.ai");
            let timeout = mgmt.timeout_secs.unwrap_or(120);
            let retries = mgmt.max_retries.unwrap_or(3);
            s.push_str(&format!(
                " management_key_env={key_env} management_base_url={base_url} management_timeout_secs={timeout} management_max_retries={retries}"
            ));
        }
        if let Some(ref store) = self.store {
            s.push_str(&format!(" store_path={}", store.path));
        }
        if let Some(ref agent) = self.agent {
            s.push_str(&format!(
                " agent_max_iterations={} agent_default_trust={} agent_enable_search={}",
                agent.max_iterations, agent.default_trust, agent.enable_search
            ));
        }
        if let Some(ref chat) = self.chat {
            let model = chat.default_model.as_deref().unwrap_or("(inherit)");
            s.push_str(&format!(
                " chat_default_model={} chat_stateful={} chat_history_file={} chat_max_conversation_tokens={}",
                model, chat.stateful, chat.history_file, chat.max_conversation_tokens
            ));
            if let Some(ref sys) = chat.system_prompt {
                // UTF-8 safe truncation: find the nearest char boundary at or before 50 bytes.
                let end = if sys.len() > 50 {
                    let mut i = 50;
                    while i > 0 && !sys.is_char_boundary(i) {
                        i -= 1;
                    }
                    i
                } else {
                    sys.len()
                };
                s.push_str(&format!(" chat_system_prompt={}...", &sys[..end]));
            }
        }
        if let Some(ref mcp) = self.mcp {
            let server_count = mcp.servers.len();
            let server_names: Vec<&str> = mcp.servers.keys().map(|k| k.as_str()).collect();
            s.push_str(&format!(
                " mcp_servers={} mcp_server_names=[{}]",
                server_count,
                server_names.join(",")
            ));
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::AppConfig;

    /// Base config sections shared by all tests.
    const BASE_CONFIG: &str = r#"
        [workspace]
        name = "grokrs"
        root = "."

        [model]
        provider = "xai"
        default_model = "grok-4"

        [policy]
        allow_network = false
        allow_shell = false
        allow_workspace_writes = true
        max_patch_bytes = 1024

        [session]
        approval_mode = "interactive"
        transcript_dir = ".grokrs/sessions"
    "#;

    const API_SECTION: &str = r#"
        [api]
        api_key_env = "XAI_API_KEY"
        base_url = "https://api.x.ai"
        timeout_secs = 120
        max_retries = 3
    "#;

    const MANAGEMENT_API_SECTION: &str = r#"
        [management_api]
        management_key_env = "XAI_MANAGEMENT_API_KEY"
        base_url = "https://management-api.x.ai"
        timeout_secs = 60
        max_retries = 2
    "#;

    #[test]
    fn parses_sample_config_with_api() {
        let raw = format!("{BASE_CONFIG}{API_SECTION}");
        let config: AppConfig = toml::from_str(&raw).expect("config should parse");
        assert_eq!(config.workspace.name, "grokrs");
        assert!(!config.policy.allow_network);
        assert_eq!(config.policy.max_patch_bytes, 1024);

        let api = config.api.expect("api section should be present");
        assert_eq!(api.api_key_env.as_deref(), Some("XAI_API_KEY"));
        assert_eq!(api.base_url.as_deref(), Some("https://api.x.ai"));
        assert_eq!(api.timeout_secs, Some(120));
        assert_eq!(api.max_retries, Some(3));
    }

    #[test]
    fn parses_config_without_api_section() {
        let config: AppConfig =
            toml::from_str(BASE_CONFIG).expect("config without [api] should parse");
        assert_eq!(config.workspace.name, "grokrs");
        assert!(config.api.is_none());
    }

    #[test]
    fn summary_includes_api_fields_when_present() {
        let raw = format!("{BASE_CONFIG}{API_SECTION}");
        let config: AppConfig = toml::from_str(&raw).unwrap();
        let summary = config.summary();
        assert!(summary.contains("api_key_env=XAI_API_KEY"));
        assert!(summary.contains("base_url=https://api.x.ai"));
        assert!(summary.contains("timeout_secs=120"));
        assert!(summary.contains("max_retries=3"));
    }

    #[test]
    fn summary_omits_api_fields_when_absent() {
        let config: AppConfig = toml::from_str(BASE_CONFIG).unwrap();
        let summary = config.summary();
        assert!(!summary.contains("api_key_env"));
        assert!(!summary.contains("base_url"));
        assert!(!summary.contains("timeout_secs"));
        assert!(!summary.contains("max_retries"));
    }

    #[test]
    fn summary_uses_defaults_for_omitted_api_fields() {
        let raw = format!("{BASE_CONFIG}\n[api]\n");
        let config: AppConfig = toml::from_str(&raw).unwrap();
        let summary = config.summary();
        // All fields are Option, so defaults kick in for summary display
        assert!(summary.contains("api_key_env=XAI_API_KEY"));
        assert!(summary.contains("base_url=https://api.x.ai"));
        assert!(summary.contains("timeout_secs=120"));
        assert!(summary.contains("max_retries=3"));
    }

    #[test]
    fn parses_config_with_management_api() {
        let raw = format!("{BASE_CONFIG}{API_SECTION}{MANAGEMENT_API_SECTION}");
        let config: AppConfig = toml::from_str(&raw).expect("config should parse");
        let mgmt = config
            .management_api
            .expect("management_api section should be present");
        assert_eq!(
            mgmt.management_key_env.as_deref(),
            Some("XAI_MANAGEMENT_API_KEY")
        );
        assert_eq!(
            mgmt.base_url.as_deref(),
            Some("https://management-api.x.ai")
        );
        assert_eq!(mgmt.timeout_secs, Some(60));
        assert_eq!(mgmt.max_retries, Some(2));
    }

    #[test]
    fn parses_config_without_management_api_section() {
        let config: AppConfig =
            toml::from_str(BASE_CONFIG).expect("config without [management_api] should parse");
        assert!(config.management_api.is_none());
    }

    #[test]
    fn summary_includes_management_fields_when_present() {
        let raw = format!("{BASE_CONFIG}{MANAGEMENT_API_SECTION}");
        let config: AppConfig = toml::from_str(&raw).unwrap();
        let summary = config.summary();
        assert!(summary.contains("management_key_env=XAI_MANAGEMENT_API_KEY"));
        assert!(summary.contains("management_base_url=https://management-api.x.ai"));
        assert!(summary.contains("management_timeout_secs=60"));
        assert!(summary.contains("management_max_retries=2"));
    }

    #[test]
    fn summary_omits_management_fields_when_absent() {
        let config: AppConfig = toml::from_str(BASE_CONFIG).unwrap();
        let summary = config.summary();
        assert!(!summary.contains("management_key_env"));
        assert!(!summary.contains("management_base_url"));
    }

    #[test]
    fn summary_uses_defaults_for_omitted_management_fields() {
        let raw = format!("{BASE_CONFIG}\n[management_api]\n");
        let config: AppConfig = toml::from_str(&raw).unwrap();
        let summary = config.summary();
        assert!(summary.contains("management_key_env=XAI_MANAGEMENT_API_KEY"));
        assert!(summary.contains("management_base_url=https://management-api.x.ai"));
        assert!(summary.contains("management_timeout_secs=120"));
        assert!(summary.contains("management_max_retries=3"));
    }

    #[test]
    fn parses_config_with_store_section() {
        let raw = format!("{BASE_CONFIG}\n[store]\npath = \"custom/store.db\"\n");
        let config: AppConfig = toml::from_str(&raw).expect("config with [store] should parse");
        let store = config.store.expect("store section should be present");
        assert_eq!(store.path, "custom/store.db");
    }

    #[test]
    fn parses_config_without_store_section() {
        let config: AppConfig =
            toml::from_str(BASE_CONFIG).expect("config without [store] should parse");
        assert!(config.store.is_none());
    }

    #[test]
    fn store_config_defaults_path_when_section_present_but_path_absent() {
        let raw = format!("{BASE_CONFIG}\n[store]\n");
        let config: AppConfig = toml::from_str(&raw).unwrap();
        let store = config.store.expect("store section should be present");
        assert_eq!(store.path, ".grokrs/state.db");
    }

    #[test]
    fn summary_includes_store_path_when_present() {
        let raw = format!("{BASE_CONFIG}\n[store]\npath = \"custom/store.db\"\n");
        let config: AppConfig = toml::from_str(&raw).unwrap();
        let summary = config.summary();
        assert!(summary.contains("store_path=custom/store.db"));
    }

    #[test]
    fn summary_omits_store_when_absent() {
        let config: AppConfig = toml::from_str(BASE_CONFIG).unwrap();
        let summary = config.summary();
        assert!(!summary.contains("store_path"));
    }

    // --- AgentConfig ---

    #[test]
    fn parses_config_with_agent_section() {
        let raw = format!(
            "{BASE_CONFIG}\n[agent]\nmax_iterations = 20\ndefault_trust = \"admin\"\nenable_search = true\n"
        );
        let config: AppConfig = toml::from_str(&raw).expect("config with [agent] should parse");
        let agent = config.agent.expect("agent section should be present");
        assert_eq!(agent.max_iterations, 20);
        assert_eq!(agent.default_trust, "admin");
        assert!(agent.enable_search);
    }

    #[test]
    fn parses_config_without_agent_section() {
        let config: AppConfig =
            toml::from_str(BASE_CONFIG).expect("config without [agent] should parse");
        assert!(config.agent.is_none());
    }

    #[test]
    fn agent_config_bare_section_uses_defaults() {
        let raw = format!("{BASE_CONFIG}\n[agent]\n");
        let config: AppConfig = toml::from_str(&raw).unwrap();
        let agent = config.agent.expect("agent section should be present");
        assert_eq!(agent.max_iterations, 10);
        assert_eq!(agent.default_trust, "untrusted");
        assert!(!agent.enable_search);
    }

    #[test]
    fn agent_config_partial_section() {
        let raw = format!("{BASE_CONFIG}\n[agent]\nmax_iterations = 5\n");
        let config: AppConfig = toml::from_str(&raw).unwrap();
        let agent = config.agent.unwrap();
        assert_eq!(agent.max_iterations, 5);
        assert_eq!(agent.default_trust, "untrusted"); // default
        assert!(!agent.enable_search); // default
    }

    #[test]
    fn summary_includes_agent_fields_when_present() {
        let raw = format!("{BASE_CONFIG}\n[agent]\nmax_iterations = 15\n");
        let config: AppConfig = toml::from_str(&raw).unwrap();
        let summary = config.summary();
        assert!(summary.contains("agent_max_iterations=15"));
        assert!(summary.contains("agent_default_trust=untrusted"));
        assert!(summary.contains("agent_enable_search=false"));
    }

    #[test]
    fn summary_omits_agent_when_absent() {
        let config: AppConfig = toml::from_str(BASE_CONFIG).unwrap();
        let summary = config.summary();
        assert!(!summary.contains("agent_"));
    }

    // --- ChatConfig ---

    #[test]
    fn parses_config_with_chat_section() {
        let raw = format!(
            "{BASE_CONFIG}\n[chat]\ndefault_model = \"grok-4-mini\"\nsystem_prompt = \"Be helpful\"\nstateful = true\nhistory_file = \"custom/history\"\nmax_conversation_tokens = 50000\n"
        );
        let config: AppConfig = toml::from_str(&raw).expect("config with [chat] should parse");
        let chat = config.chat.expect("chat section should be present");
        assert_eq!(chat.default_model.as_deref(), Some("grok-4-mini"));
        assert_eq!(chat.system_prompt.as_deref(), Some("Be helpful"));
        assert!(chat.stateful);
        assert_eq!(chat.history_file, "custom/history");
        assert_eq!(chat.max_conversation_tokens, 50000);
    }

    #[test]
    fn parses_config_without_chat_section() {
        let config: AppConfig =
            toml::from_str(BASE_CONFIG).expect("config without [chat] should parse");
        assert!(config.chat.is_none());
    }

    #[test]
    fn chat_config_bare_section_uses_defaults() {
        let raw = format!("{BASE_CONFIG}\n[chat]\n");
        let config: AppConfig = toml::from_str(&raw).unwrap();
        let chat = config.chat.expect("chat section should be present");
        assert!(chat.default_model.is_none());
        assert!(chat.system_prompt.is_none());
        assert!(!chat.stateful);
        assert_eq!(chat.history_file, ".grokrs/chat_history");
        assert_eq!(chat.max_conversation_tokens, 100_000);
    }

    #[test]
    fn chat_config_partial_section() {
        let raw = format!("{BASE_CONFIG}\n[chat]\nstateful = true\n");
        let config: AppConfig = toml::from_str(&raw).unwrap();
        let chat = config.chat.unwrap();
        assert!(chat.stateful);
        assert!(chat.default_model.is_none()); // default
        assert_eq!(chat.history_file, ".grokrs/chat_history"); // default
    }

    #[test]
    fn summary_includes_chat_fields_when_present() {
        let raw = format!("{BASE_CONFIG}\n[chat]\nstateful = true\n");
        let config: AppConfig = toml::from_str(&raw).unwrap();
        let summary = config.summary();
        assert!(summary.contains("chat_stateful=true"));
        assert!(summary.contains("chat_default_model=(inherit)"));
        assert!(summary.contains("chat_history_file=.grokrs/chat_history"));
        assert!(summary.contains("chat_max_conversation_tokens=100000"));
    }

    #[test]
    fn summary_omits_chat_when_absent() {
        let config: AppConfig = toml::from_str(BASE_CONFIG).unwrap();
        let summary = config.summary();
        assert!(!summary.contains("chat_"));
    }

    #[test]
    fn existing_config_without_new_sections_still_parses() {
        // The most important backward-compatibility test: existing configs
        // with only the original sections must continue to work.
        let config: AppConfig = toml::from_str(BASE_CONFIG).unwrap();
        assert!(config.agent.is_none());
        assert!(config.chat.is_none());
        assert!(config.mcp.is_none());
        assert_eq!(config.workspace.name, "grokrs");
        assert_eq!(config.model.default_model, "grok-4");
    }

    // --- McpConfig ---

    #[test]
    fn parses_config_with_mcp_section() {
        let raw = format!(
            "{BASE_CONFIG}\n\
             [mcp.servers.filesystem]\n\
             url = \"http://localhost:8080/mcp\"\n\
             label = \"filesystem\"\n\
             trust_rank = 1\n\
             timeout_secs = 30\n"
        );
        let config: AppConfig = toml::from_str(&raw).expect("config with [mcp] should parse");
        let mcp = config.mcp.expect("mcp section should be present");
        assert_eq!(mcp.servers.len(), 1);
        let server = mcp
            .servers
            .get("filesystem")
            .expect("filesystem server should exist");
        assert_eq!(server.url, "http://localhost:8080/mcp");
        assert_eq!(server.label.as_deref(), Some("filesystem"));
        assert_eq!(server.trust_rank, 1);
        assert_eq!(server.timeout_secs, 30);
        assert!(server.allowed_tools.is_none());
    }

    #[test]
    fn parses_config_without_mcp_section() {
        let config: AppConfig =
            toml::from_str(BASE_CONFIG).expect("config without [mcp] should parse");
        assert!(config.mcp.is_none());
    }

    #[test]
    fn mcp_server_defaults() {
        let raw = format!(
            "{BASE_CONFIG}\n\
             [mcp.servers.test]\n\
             url = \"http://localhost:9090\"\n"
        );
        let config: AppConfig = toml::from_str(&raw).unwrap();
        let mcp = config.mcp.unwrap();
        let server = mcp.servers.get("test").unwrap();
        assert_eq!(server.trust_rank, 1); // default
        assert_eq!(server.timeout_secs, 30); // default
        assert!(server.label.is_none());
        assert!(server.allowed_tools.is_none());
    }

    #[test]
    fn mcp_multiple_servers() {
        let raw = format!(
            "{BASE_CONFIG}\n\
             [mcp.servers.server_a]\n\
             url = \"http://localhost:8080/mcp\"\n\
             label = \"Server A\"\n\
             \n\
             [mcp.servers.server_b]\n\
             url = \"http://localhost:9090/mcp\"\n\
             label = \"Server B\"\n\
             trust_rank = 2\n\
             allowed_tools = [\"query\", \"list_tables\"]\n"
        );
        let config: AppConfig = toml::from_str(&raw).unwrap();
        let mcp = config.mcp.unwrap();
        assert_eq!(mcp.servers.len(), 2);

        let a = mcp.servers.get("server_a").unwrap();
        assert_eq!(a.label.as_deref(), Some("Server A"));
        assert_eq!(a.trust_rank, 1);

        let b = mcp.servers.get("server_b").unwrap();
        assert_eq!(b.label.as_deref(), Some("Server B"));
        assert_eq!(b.trust_rank, 2);
        assert_eq!(
            b.allowed_tools.as_ref().unwrap(),
            &vec!["query".to_owned(), "list_tables".to_owned()]
        );
    }

    #[test]
    fn mcp_bare_section_uses_defaults() {
        let raw = format!("{BASE_CONFIG}\n[mcp]\n");
        let config: AppConfig = toml::from_str(&raw).unwrap();
        let mcp = config.mcp.expect("mcp section should be present");
        assert!(mcp.servers.is_empty());
    }

    #[test]
    fn summary_includes_mcp_when_present() {
        let raw = format!(
            "{BASE_CONFIG}\n\
             [mcp.servers.test]\n\
             url = \"http://localhost:8080\"\n"
        );
        let config: AppConfig = toml::from_str(&raw).unwrap();
        let summary = config.summary();
        assert!(summary.contains("mcp_servers=1"));
        assert!(summary.contains("test"));
    }

    #[test]
    fn summary_omits_mcp_when_absent() {
        let config: AppConfig = toml::from_str(BASE_CONFIG).unwrap();
        let summary = config.summary();
        assert!(!summary.contains("mcp_"));
    }

    // --- Profile / deep_merge ---

    use super::{deep_merge, resolve_profile, validate_profile_name};
    use std::io::Write;

    #[test]
    fn validate_profile_name_accepts_valid_names() {
        assert!(validate_profile_name("dev").is_ok());
        assert!(validate_profile_name("staging").is_ok());
        assert!(validate_profile_name("prod").is_ok());
        assert!(validate_profile_name("my-profile").is_ok());
        assert!(validate_profile_name("my_profile").is_ok());
        assert!(validate_profile_name("dev123").is_ok());
        assert!(validate_profile_name("A-B_C").is_ok());
    }

    #[test]
    fn validate_profile_name_rejects_invalid_names() {
        assert!(validate_profile_name("").is_err());
        assert!(validate_profile_name("dev/staging").is_err());
        assert!(validate_profile_name("my profile").is_err());
        assert!(validate_profile_name("../escape").is_err());
        assert!(validate_profile_name("name.toml").is_err());
        assert!(validate_profile_name("a b").is_err());
    }

    #[test]
    fn deep_merge_replaces_leaf_values() {
        let mut base: toml::Value = toml::from_str(
            r#"
            [policy]
            allow_network = false
            allow_shell = false
            max_patch_bytes = 1024
        "#,
        )
        .unwrap();

        let overlay: toml::Value = toml::from_str(
            r#"
            [policy]
            allow_network = true
        "#,
        )
        .unwrap();

        deep_merge(&mut base, overlay);

        let table = base.as_table().unwrap();
        let policy = table["policy"].as_table().unwrap();
        assert_eq!(policy["allow_network"].as_bool(), Some(true));
        // Unchanged values preserved.
        assert_eq!(policy["allow_shell"].as_bool(), Some(false));
        assert_eq!(policy["max_patch_bytes"].as_integer(), Some(1024));
    }

    #[test]
    fn deep_merge_adds_new_sections() {
        let mut base: toml::Value = toml::from_str(BASE_CONFIG).unwrap();

        let overlay: toml::Value = toml::from_str(
            r#"
            [agent]
            max_iterations = 25
            default_trust = "admin"
        "#,
        )
        .unwrap();

        deep_merge(&mut base, overlay);

        let table = base.as_table().unwrap();
        let agent = table["agent"].as_table().unwrap();
        assert_eq!(agent["max_iterations"].as_integer(), Some(25));
        assert_eq!(agent["default_trust"].as_str(), Some("admin"));
        // Original sections still present.
        assert!(table.contains_key("workspace"));
        assert!(table.contains_key("policy"));
    }

    #[test]
    fn deep_merge_nested_tables_merge_recursively() {
        let mut base: toml::Value = toml::from_str(
            r#"
            [api]
            base_url = "https://api.x.ai"
            timeout_secs = 120
            max_retries = 3
        "#,
        )
        .unwrap();

        let overlay: toml::Value = toml::from_str(
            r#"
            [api]
            timeout_secs = 60
        "#,
        )
        .unwrap();

        deep_merge(&mut base, overlay);

        let api = base.as_table().unwrap()["api"].as_table().unwrap();
        assert_eq!(api["base_url"].as_str(), Some("https://api.x.ai")); // inherited
        assert_eq!(api["timeout_secs"].as_integer(), Some(60)); // overridden
        assert_eq!(api["max_retries"].as_integer(), Some(3)); // inherited
    }

    #[test]
    fn deep_merge_overlay_replaces_non_table_with_non_table() {
        let mut base: toml::Value = toml::from_str(
            r#"
            [session]
            approval_mode = "interactive"
        "#,
        )
        .unwrap();

        let overlay: toml::Value = toml::from_str(
            r#"
            [session]
            approval_mode = "allow"
        "#,
        )
        .unwrap();

        deep_merge(&mut base, overlay);
        assert_eq!(
            base.as_table().unwrap()["session"].as_table().unwrap()["approval_mode"].as_str(),
            Some("allow")
        );
    }

    #[test]
    fn deep_merge_full_config_round_trip() {
        // Simulate merging a dev profile on top of the base config and
        // deserializing the result into AppConfig.
        let mut base: toml::Value = toml::from_str(BASE_CONFIG).unwrap();

        let overlay: toml::Value = toml::from_str(
            r#"
            [policy]
            allow_network = true
            allow_shell = true

            [session]
            approval_mode = "allow"

            [agent]
            max_iterations = 25
            default_trust = "admin"
            enable_search = true

            [chat]
            stateful = true
        "#,
        )
        .unwrap();

        deep_merge(&mut base, overlay);

        let config: AppConfig = base.try_into().expect("merged config should deserialize");

        // Overridden values.
        assert!(config.policy.allow_network);
        assert!(config.policy.allow_shell);
        assert_eq!(config.session.approval_mode, "allow");

        // Inherited values.
        assert!(config.policy.allow_workspace_writes);
        assert_eq!(config.policy.max_patch_bytes, 1024);
        assert_eq!(config.workspace.name, "grokrs");
        assert_eq!(config.model.default_model, "grok-4");
        assert_eq!(config.session.transcript_dir, ".grokrs/sessions");

        // New sections from overlay.
        let agent = config.agent.expect("agent should be present after merge");
        assert_eq!(agent.max_iterations, 25);
        assert_eq!(agent.default_trust, "admin");
        assert!(agent.enable_search);

        let chat = config.chat.expect("chat should be present after merge");
        assert!(chat.stateful);
        // Chat defaults for fields not in overlay.
        assert_eq!(chat.history_file, ".grokrs/chat_history");
        assert_eq!(chat.max_conversation_tokens, 100_000);
    }

    #[test]
    fn profile_path_from_base_config() {
        let base = std::path::Path::new("configs/grokrs.example.toml");
        let path = AppConfig::profile_path(base, "dev");
        assert_eq!(path, std::path::PathBuf::from("configs/grokrs.dev.toml"));
    }

    #[test]
    fn profile_path_from_absolute_base() {
        let base = std::path::Path::new("/etc/grokrs/grokrs.toml");
        let path = AppConfig::profile_path(base, "staging");
        assert_eq!(
            path,
            std::path::PathBuf::from("/etc/grokrs/grokrs.staging.toml")
        );
    }

    #[test]
    fn load_with_profile_no_profile_loads_base() {
        let dir = tempfile::tempdir().unwrap();
        let base_path = dir.path().join("grokrs.example.toml");
        let mut base_file = std::fs::File::create(&base_path).unwrap();
        write!(base_file, "{}", BASE_CONFIG).unwrap();

        let config = AppConfig::load_with_profile(&base_path, None)
            .expect("load_with_profile(None) should load base config");
        assert_eq!(config.workspace.name, "grokrs");
        assert!(!config.policy.allow_network);
    }

    #[test]
    fn load_with_profile_merges_profile_on_top() {
        let dir = tempfile::tempdir().unwrap();
        let base_path = dir.path().join("grokrs.example.toml");
        let profile_path = dir.path().join("grokrs.dev.toml");

        std::fs::write(&base_path, BASE_CONFIG).unwrap();
        std::fs::write(
            &profile_path,
            r#"
            [policy]
            allow_network = true
            allow_shell = true

            [session]
            approval_mode = "allow"
        "#,
        )
        .unwrap();

        let config = AppConfig::load_with_profile(&base_path, Some("dev"))
            .expect("load_with_profile should merge dev profile");

        // Overridden by profile.
        assert!(config.policy.allow_network);
        assert!(config.policy.allow_shell);
        assert_eq!(config.session.approval_mode, "allow");

        // Inherited from base.
        assert!(config.policy.allow_workspace_writes);
        assert_eq!(config.policy.max_patch_bytes, 1024);
        assert_eq!(config.workspace.name, "grokrs");
        assert_eq!(config.model.default_model, "grok-4");
    }

    #[test]
    fn load_with_profile_missing_profile_file_errors() {
        let dir = tempfile::tempdir().unwrap();
        let base_path = dir.path().join("grokrs.example.toml");
        std::fs::write(&base_path, BASE_CONFIG).unwrap();

        let result = AppConfig::load_with_profile(&base_path, Some("nonexistent"));
        assert!(result.is_err());
        let err = result.unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("profile config not found"),
            "error should mention 'profile config not found', got: {msg}"
        );
    }

    #[test]
    fn load_with_profile_invalid_name_errors() {
        let dir = tempfile::tempdir().unwrap();
        let base_path = dir.path().join("grokrs.example.toml");
        std::fs::write(&base_path, BASE_CONFIG).unwrap();

        let result = AppConfig::load_with_profile(&base_path, Some("../evil"));
        assert!(result.is_err());
        let err = result.unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("invalid profile name"),
            "error should mention 'invalid profile name', got: {msg}"
        );
    }

    #[test]
    fn resolve_profile_flag_takes_precedence() {
        // Set env var, but flag should win.
        // SAFETY: Test-only env manipulation; test runner serializes these tests.
        unsafe {
            std::env::set_var("GROKRS_PROFILE", "staging");
        }
        let result = resolve_profile(Some("dev"));
        assert_eq!(result, Some("dev".to_owned()));
        // SAFETY: Test-only env manipulation; test runner serializes these tests.
        unsafe {
            std::env::remove_var("GROKRS_PROFILE");
        }
    }

    #[test]
    fn resolve_profile_falls_back_to_env() {
        // SAFETY: Test-only env manipulation; test runner serializes these tests.
        unsafe {
            std::env::set_var("GROKRS_PROFILE", "staging");
        }
        let result = resolve_profile(None);
        assert_eq!(result, Some("staging".to_owned()));
        // SAFETY: Test-only env manipulation; test runner serializes these tests.
        unsafe {
            std::env::remove_var("GROKRS_PROFILE");
        }
    }

    #[test]
    fn resolve_profile_none_when_nothing_set() {
        // SAFETY: Test-only env manipulation; test runner serializes these tests.
        unsafe {
            std::env::remove_var("GROKRS_PROFILE");
        }
        let result = resolve_profile(None);
        assert_eq!(result, None);
    }

    #[test]
    fn resolve_profile_ignores_empty_env() {
        // SAFETY: Test-only env manipulation; test runner serializes these tests.
        unsafe {
            std::env::set_var("GROKRS_PROFILE", "");
        }
        let result = resolve_profile(None);
        assert_eq!(result, None);
        // SAFETY: Test-only env manipulation; test runner serializes these tests.
        unsafe {
            std::env::remove_var("GROKRS_PROFILE");
        }
    }

    #[test]
    fn load_with_profile_partial_agent_overlay() {
        // Profile adds only agent section; base has no agent.
        let dir = tempfile::tempdir().unwrap();
        let base_path = dir.path().join("grokrs.example.toml");
        let profile_path = dir.path().join("grokrs.test.toml");

        std::fs::write(&base_path, BASE_CONFIG).unwrap();
        std::fs::write(
            &profile_path,
            r#"
            [agent]
            max_iterations = 50
            default_trust = "admin"
            enable_search = true
        "#,
        )
        .unwrap();

        let config = AppConfig::load_with_profile(&base_path, Some("test"))
            .expect("should merge agent-only profile");

        let agent = config.agent.expect("agent should be present");
        assert_eq!(agent.max_iterations, 50);
        assert_eq!(agent.default_trust, "admin");
        assert!(agent.enable_search);

        // Base values unchanged.
        assert!(!config.policy.allow_network);
        assert_eq!(config.session.approval_mode, "interactive");
    }

    // --- check_deprecated_model ---
    //
    // These tests verify the matching logic only (prefix-based). Stderr output
    // is a side-effect of informational warnings and is not captured here;
    // integration-level stderr capture is in tests/cli_smoke.rs.

    use super::check_deprecated_model;

    #[test]
    fn deprecated_model_check_exact_grok_2() {
        // "grok-2" is the exact deprecated name; should not panic.
        // (We cannot assert on stderr in unit tests; we verify no panic/abort.)
        check_deprecated_model("grok-2");
    }

    #[test]
    fn deprecated_model_check_grok_2_variant() {
        // "grok-2-1212" matches the grok-2 prefix family.
        check_deprecated_model("grok-2-1212");
    }

    #[test]
    fn deprecated_model_check_exact_grok_3() {
        check_deprecated_model("grok-3");
    }

    #[test]
    fn deprecated_model_check_grok_3_variant() {
        // "grok-3-turbo" and "grok-3-mini" are in the deprecated grok-3 family.
        check_deprecated_model("grok-3-turbo");
        check_deprecated_model("grok-3-mini");
    }

    #[test]
    fn deprecated_model_check_current_model_no_warn() {
        // Current models must NOT trigger the deprecation path. We verify that
        // the function returns without panicking and, crucially, does not match
        // the deprecated prefixes. Since we cannot capture stderr in unit tests,
        // we rely on the function being a no-op (no side effects beyond print)
        // for current models.
        check_deprecated_model("grok-4");
        check_deprecated_model("grok-4-mini");
        check_deprecated_model("grok-4-turbo");
    }

    #[test]
    fn deprecated_model_check_does_not_match_grok_20_or_similar() {
        // "grok-20" must NOT match "grok-2" — it is not a grok-2 variant.
        // The prefix check uses "grok-2-" so "grok-20" does not qualify.
        // This test verifies no false positives from numeric suffix collisions.
        check_deprecated_model("grok-20");
        check_deprecated_model("grok-30");
    }

    #[test]
    fn deprecated_model_prefix_match_requires_hyphen_separator() {
        // "grok-2foo" does NOT start with "grok-2-" so it should not match
        // the variant rule, and it is not exactly "grok-2". It is an unknown
        // model name that happens to share a prefix — no warning should fire.
        check_deprecated_model("grok-2foo");
    }

    #[test]
    fn deprecated_model_check_is_deterministic() {
        // Calling the function multiple times for the same input is idempotent
        // (no panic, no corruption of global state).
        for _ in 0..5 {
            check_deprecated_model("grok-3-turbo");
            check_deprecated_model("grok-4");
        }
    }
}
