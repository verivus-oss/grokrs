//! CLI subcommands for collection management via the Management API.
//!
//! Each subcommand loads config, constructs a `PolicyEngine`, wires it into
//! `ManagementClient` via `FnPolicyGate`, and delegates to the appropriate
//! endpoint client. The management API key is never stored or logged.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use clap::Subcommand;

use grokrs_api::management::client::ManagementClient;
use grokrs_api::transport::policy_bridge::FnPolicyGate;
use grokrs_api::transport::policy_gate::{PolicyDecision, PolicyGate};
use grokrs_api::types::collection_documents::AddDocumentRequest;
use grokrs_api::types::collections::CreateCollectionRequest;
use grokrs_core::AppConfig;
use grokrs_policy::{Decision, Effect, PolicyEngine};

/// Collections Management API subcommands.
#[derive(Subcommand)]
pub enum CollectionsCommand {
    /// Create a new collection
    Create {
        /// Name for the new collection
        name: String,
        /// Embedding model to use (e.g., grok-embedding-small)
        #[arg(long, default_value = "grok-embedding-small")]
        model: String,
    },
    /// List all collections
    List,
    /// Add a document (by file ID) to a collection
    AddDoc {
        /// Collection ID
        collection_id: String,
        /// File ID (from the inference Files API)
        file_id: String,
        /// Optional human-readable name for the document
        #[arg(long)]
        name: Option<String>,
    },
    /// List documents in a collection
    ListDocs {
        /// Collection ID
        collection_id: String,
        /// Filter expression (e.g., status filter)
        #[arg(long)]
        filter: Option<String>,
    },
    /// Delete a collection
    Delete {
        /// Collection ID
        collection_id: String,
        /// Skip confirmation prompt (use with caution)
        #[arg(long)]
        yes: bool,
    },
}

/// Check that the config has a `[management_api]` section and that network
/// access is allowed by policy. Returns a helpful error if either is missing.
fn check_management_config(config: &AppConfig) -> Result<()> {
    if config.management_api.is_none() {
        bail!(
            "Management API is not configured.\n\
             \n\
             Add a [management_api] section to your config file:\n\
             \n\
             [management_api]\n\
             management_key_env = \"XAI_MANAGEMENT_API_KEY\"\n\
             base_url = \"https://management-api.x.ai\"\n\
             timeout_secs = 120\n\
             max_retries = 3\n\
             \n\
             Then set the XAI_MANAGEMENT_API_KEY environment variable to your\n\
             Management API key (this is separate from the inference API key\n\
             set via XAI_API_KEY).\n\
             \n\
             Config file location: use --config <path> or the default configs/grokrs.example.toml"
        );
    }
    if !config.policy.allow_network {
        bail!(
            "Network access is denied by policy.\n\
             \n\
             To use the Collections Management API, set `allow_network = true`\n\
             in your config file:\n\
             \n\
             [policy]\n\
             allow_network = true\n\
             \n\
             Config file location: use --config <path> or the default configs/grokrs.example.toml"
        );
    }
    Ok(())
}

/// Build a policy gate that bridges `PolicyEngine` into the `PolicyGate` trait.
///
/// Resolves `Decision::Ask` based on the configured `approval_mode`:
///
/// - `"allow"` — map `Ask` to `Allow` (bypass approval; use with caution)
/// - `"deny"` — map `Ask` to `Deny` (fail-closed; no interactive approval)
/// - `"interactive"` (or any other value) — keep `Ask` as `Ask` (requires
///   the approval broker, which is not yet implemented; effectively a denial)
fn build_policy_gate(engine: PolicyEngine, approval_mode: &str) -> Arc<dyn PolicyGate> {
    let approval_mode = approval_mode.to_owned();
    Arc::new(FnPolicyGate::new(move |host: &str| {
        let effect = Effect::NetworkConnect {
            host: host.to_owned(),
        };
        match engine.evaluate(&effect) {
            Decision::Allow { .. } => PolicyDecision::Allow,
            Decision::Ask { reason } => match approval_mode.as_str() {
                "allow" => PolicyDecision::Allow,
                "deny" => PolicyDecision::Deny {
                    reason: reason.to_owned(),
                },
                _ => PolicyDecision::Ask,
            },
            Decision::Deny { reason } => PolicyDecision::Deny {
                reason: reason.to_owned(),
            },
        }
    }))
}

/// Build a `ManagementClient` from config with policy enforcement.
fn build_management_client(config: &AppConfig) -> Result<ManagementClient> {
    let engine = PolicyEngine::new(config.policy.clone());
    let gate = build_policy_gate(engine, &config.session.approval_mode);
    ManagementClient::from_config(config, Some(gate))
        .context("failed to construct Management API client")
}

/// Execute a collections subcommand.
///
/// # Errors
///
/// Returns an error if client construction or the Management API call fails.
pub async fn run(command: &CollectionsCommand, config: &AppConfig) -> Result<()> {
    check_management_config(config)?;
    let client = build_management_client(config)?;

    match command {
        CollectionsCommand::Create { name, model } => run_create(&client, name, model).await,
        CollectionsCommand::List => run_list(&client).await,
        CollectionsCommand::AddDoc {
            collection_id,
            file_id,
            name,
        } => run_add_doc(&client, collection_id, file_id, name.as_deref()).await,
        CollectionsCommand::ListDocs {
            collection_id,
            filter,
        } => run_list_docs(&client, collection_id, filter.as_deref()).await,
        CollectionsCommand::Delete { collection_id, yes } => {
            run_delete(&client, collection_id, *yes).await
        }
    }
}

/// Create a new collection.
async fn run_create(client: &ManagementClient, name: &str, model: &str) -> Result<()> {
    let request = CreateCollectionRequest {
        name: name.to_owned(),
        description: None,
        embedding_model: model.to_owned(),
        chunk_configuration: None,
        index_configuration: None,
        field_definitions: vec![],
    };

    let collection = client
        .collections()
        .create(&request)
        .await
        .context("failed to create collection")?;

    println!("Created collection:");
    println!("  ID:    {}", collection.id);
    println!("  Name:  {}", collection.name);
    println!("  Model: {}", collection.embedding_model);
    if let Some(ref created) = collection.created_at {
        println!("  Created: {created}");
    }

    Ok(())
}

/// List all collections.
async fn run_list(client: &ManagementClient) -> Result<()> {
    let list = client
        .collections()
        .list()
        .await
        .context("failed to list collections")?;

    if list.collections.is_empty() {
        println!("No collections found.");
        return Ok(());
    }

    println!("{:<30} {:<30} {:<25} CREATED", "ID", "NAME", "MODEL");
    println!("{}", "-".repeat(100));

    for col in &list.collections {
        let created = col.created_at.as_deref().unwrap_or("-");
        println!(
            "{:<30} {:<30} {:<25} {}",
            col.id, col.name, col.embedding_model, created
        );
    }

    println!("\nTotal: {} collections", list.collections.len());
    Ok(())
}

/// Add a document to a collection.
async fn run_add_doc(
    client: &ManagementClient,
    collection_id: &str,
    file_id: &str,
    name: Option<&str>,
) -> Result<()> {
    let request = AddDocumentRequest {
        name: name.map(String::from),
        fields: HashMap::new(),
    };

    let doc = client
        .collections()
        .documents(collection_id)
        .add(file_id, &request)
        .await
        .context("failed to add document to collection")?;

    println!("Added document:");
    println!("  File ID: {}", doc.file_id);
    println!("  Status:  {:?}", doc.status);
    if let Some(ref n) = doc.name {
        println!("  Name:    {n}");
    }

    Ok(())
}

/// List documents in a collection.
async fn run_list_docs(
    client: &ManagementClient,
    collection_id: &str,
    filter: Option<&str>,
) -> Result<()> {
    let list = client
        .collections()
        .documents(collection_id)
        .list(filter, None, None)
        .await
        .context("failed to list documents")?;

    if list.documents.is_empty() {
        println!("No documents found in collection {collection_id}.");
        return Ok(());
    }

    println!("{:<30} {:<12} {:<30} CREATED", "FILE ID", "STATUS", "NAME");
    println!("{}", "-".repeat(90));

    for doc in &list.documents {
        let name = doc.name.as_deref().unwrap_or("-");
        let created = doc.created_at.as_deref().unwrap_or("-");
        println!(
            "{:<30} {:<12} {:<30} {}",
            doc.file_id,
            format!("{:?}", doc.status),
            name,
            created
        );
    }

    println!("\nTotal: {} documents", list.documents.len());
    Ok(())
}

/// Delete a collection (with confirmation).
async fn run_delete(client: &ManagementClient, collection_id: &str, yes: bool) -> Result<()> {
    if !yes {
        eprintln!(
            "WARNING: This will permanently delete collection '{collection_id}' and all its documents."
        );
        eprintln!("To confirm, re-run with --yes flag:");
        eprintln!("  grokrs collections delete {collection_id} --yes");
        bail!("Deletion cancelled. Use --yes to confirm.");
    }

    client
        .collections()
        .delete(collection_id)
        .await
        .context("failed to delete collection")?;

    println!("Deleted collection: {collection_id}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use grokrs_core::{
        AppConfig, ManagementApiConfig, ModelConfig, PolicyConfig, SessionConfig, WorkspaceConfig,
    };

    fn base_config(allow_network: bool) -> AppConfig {
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
                allow_network,
                allow_shell: false,
                allow_workspace_writes: true,
                max_patch_bytes: 1024,
            },
            session: SessionConfig {
                approval_mode: "interactive".into(),
                transcript_dir: ".grokrs/sessions".into(),
            },
            api: None,
            management_api: Some(ManagementApiConfig {
                management_key_env: Some("XAI_MANAGEMENT_API_KEY".into()),
                base_url: Some("https://management-api.x.ai".into()),
                timeout_secs: Some(120),
                max_retries: Some(3),
            }),
            store: None,
            agent: None,
            chat: None,
            mcp: None,
        }
    }

    fn engine(allow_network: bool) -> PolicyEngine {
        PolicyEngine::new(PolicyConfig {
            allow_network,
            allow_shell: false,
            allow_workspace_writes: false,
            max_patch_bytes: 0,
        })
    }

    #[test]
    fn check_management_config_fails_without_section() {
        let mut config = base_config(true);
        config.management_api = None;
        let result = check_management_config(&config);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("Management API is not configured"),
            "error should explain management_api is missing: {err_msg}"
        );
        assert!(
            err_msg.contains("[management_api]"),
            "error should show config example: {err_msg}"
        );
        assert!(
            err_msg.contains("XAI_MANAGEMENT_API_KEY"),
            "error should mention management key env var: {err_msg}"
        );
    }

    #[test]
    fn check_management_config_fails_when_network_denied() {
        let config = base_config(false);
        let result = check_management_config(&config);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("Network access is denied"),
            "error should explain network is denied: {err_msg}"
        );
    }

    #[test]
    fn check_management_config_ok_when_present_and_network_allowed() {
        let config = base_config(true);
        let result = check_management_config(&config);
        assert!(result.is_ok());
    }

    // --- approval_mode tests ---

    #[test]
    fn approval_mode_allow_maps_ask_to_allow() {
        let gate = build_policy_gate(engine(true), "allow");
        let decision = gate.evaluate_network("management-api.x.ai");
        assert!(
            matches!(decision, PolicyDecision::Allow),
            "expected Allow when approval_mode='allow', got {decision:?}"
        );
    }

    #[test]
    fn approval_mode_deny_maps_ask_to_deny() {
        let gate = build_policy_gate(engine(true), "deny");
        let decision = gate.evaluate_network("management-api.x.ai");
        assert!(
            matches!(decision, PolicyDecision::Deny { .. }),
            "expected Deny when approval_mode='deny', got {decision:?}"
        );
    }

    #[test]
    fn approval_mode_interactive_keeps_ask() {
        let gate = build_policy_gate(engine(true), "interactive");
        let decision = gate.evaluate_network("management-api.x.ai");
        assert!(
            matches!(decision, PolicyDecision::Ask),
            "expected Ask when approval_mode='interactive', got {decision:?}"
        );
    }

    #[test]
    fn approval_mode_unknown_treated_as_interactive() {
        let gate = build_policy_gate(engine(true), "something-else");
        let decision = gate.evaluate_network("management-api.x.ai");
        assert!(
            matches!(decision, PolicyDecision::Ask),
            "expected Ask for unknown approval_mode, got {decision:?}"
        );
    }

    // --- Allow decisions are never downgraded ---

    #[test]
    fn allow_decision_never_downgraded_by_deny_mode() {
        // With allow_network=false, engine returns Deny — verify it stays Deny,
        // not that an Allow gets downgraded (engine never returns Allow for
        // NetworkConnect today). This proves the bridge only touches Ask.
        let gate = build_policy_gate(engine(false), "deny");
        let decision = gate.evaluate_network("management-api.x.ai");
        assert!(
            matches!(decision, PolicyDecision::Deny { .. }),
            "Deny from engine must stay Deny regardless of approval_mode"
        );
    }

    // --- Deny decisions are never upgraded ---

    #[test]
    fn deny_decision_never_upgraded_by_allow_mode() {
        // allow_network=false produces Deny; approval_mode="allow" must NOT
        // upgrade it to Allow. Only Ask decisions are affected.
        let gate = build_policy_gate(engine(false), "allow");
        let decision = gate.evaluate_network("management-api.x.ai");
        assert!(
            matches!(decision, PolicyDecision::Deny { .. }),
            "Deny from engine must never be upgraded to Allow, got {decision:?}"
        );
    }

    #[test]
    fn deny_decision_never_upgraded_by_interactive_mode() {
        let gate = build_policy_gate(engine(false), "interactive");
        let decision = gate.evaluate_network("management-api.x.ai");
        assert!(
            matches!(decision, PolicyDecision::Deny { .. }),
            "Deny from engine must never be upgraded to Ask, got {decision:?}"
        );
    }

    // --- Legacy behaviour preserved ---

    #[test]
    fn policy_bridge_maps_deny() {
        let gate = build_policy_gate(engine(false), "interactive");
        let decision = gate.evaluate_network("management-api.x.ai");
        assert!(
            matches!(decision, PolicyDecision::Deny { .. }),
            "expected Deny, got {decision:?}"
        );
    }

    #[test]
    fn policy_bridge_maps_ask() {
        let gate = build_policy_gate(engine(true), "interactive");
        let decision = gate.evaluate_network("management-api.x.ai");
        assert!(
            matches!(decision, PolicyDecision::Ask),
            "expected Ask, got {decision:?}"
        );
    }
}
