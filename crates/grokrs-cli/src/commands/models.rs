//! CLI subcommands for model discovery.
//!
//! `grokrs models list` lists all language models with pricing, modalities, and
//! context window. `grokrs models info <model_id>` shows detailed information
//! for a single model. `grokrs models pricing` shows a pricing comparison
//! sorted by cost.
//!
//! These commands use `ModelsClient` directly — no `ToolRegistry` or `AgentExecutor`
//! needed. All model data is fetched from the API at runtime; nothing is hardcoded.

use std::sync::Arc;

use anyhow::{Context, Result, bail};
use clap::Subcommand;

use grokrs_api::client::GrokClient;
use grokrs_api::transport::policy_bridge::FnPolicyGate;
use grokrs_api::transport::policy_gate::{PolicyDecision, PolicyGate};
use grokrs_core::AppConfig;
use grokrs_policy::{Decision, Effect, PolicyEngine};

/// Models subcommand group.
#[derive(Subcommand)]
#[command(after_help = "\
Examples:
  grokrs models list                     List language models
  grokrs models list --type image        List image generation models
  grokrs models list --json              Output as JSON
  grokrs models info grok-4              Show details for a specific model
  grokrs models pricing                  Compare pricing (cheapest first)
  grokrs models pricing --json           Pricing as JSON")]
pub enum ModelsCommand {
    /// List models (language by default, or --type image/video).
    List {
        /// Model type: language, image, or video.
        #[arg(long, default_value = "language")]
        r#type: String,

        /// Output as JSON instead of a table.
        #[arg(long)]
        json: bool,
    },

    /// Show detailed information for a single model.
    Info {
        /// The model ID to look up.
        model_id: String,

        /// Output as JSON instead of formatted text.
        #[arg(long)]
        json: bool,
    },

    /// Show a pricing comparison table sorted by cost (cheapest first).
    Pricing {
        /// Output as JSON instead of a table.
        #[arg(long)]
        json: bool,
    },
}

/// Build a policy gate from config (same pattern as api.rs).
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

/// Check that network access is allowed.
fn check_network_allowed(config: &AppConfig) -> Result<()> {
    if !config.policy.allow_network {
        bail!(
            "Network access is denied by policy.\n\
             \n\
             The models command requires network access to query the xAI API.\n\
             To enable, set `allow_network = true` in your config file:\n\
             \n\
             [policy]\n\
             allow_network = true\n\
             \n\
             Config file location: use --config <path> or the default configs/grokrs.example.toml"
        );
    }
    Ok(())
}

/// Check if stdout is connected to a TTY.
fn is_tty() -> bool {
    #[cfg(unix)]
    {
        // SAFETY: STDOUT_FILENO (1) is always valid on Unix.
        unsafe { libc::isatty(libc::STDOUT_FILENO) != 0 }
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// ANSI escape helper: bold text.
fn bold(text: &str, use_color: bool) -> String {
    if use_color {
        format!("\x1b[1m{text}\x1b[0m")
    } else {
        text.to_string()
    }
}

/// ANSI escape helper: color text based on price tier.
///
/// green = cheap (< 300), yellow = moderate (300-1000), red = expensive (> 1000).
fn price_color(price: i64, text: &str, use_color: bool) -> String {
    if !use_color {
        return text.to_string();
    }
    if price < 300 {
        format!("\x1b[32m{text}\x1b[0m") // green
    } else if price <= 1000 {
        format!("\x1b[33m{text}\x1b[0m") // yellow
    } else {
        format!("\x1b[31m{text}\x1b[0m") // red
    }
}

/// Format a price value or "-" for missing.
fn fmt_price(price: Option<i64>) -> String {
    price.map_or_else(|| "-".into(), |p| format!("{p}"))
}

/// Execute the `grokrs models` command.
///
/// # Errors
///
/// Returns an error if the network policy check, client construction, or
/// API call fails.
pub async fn run(command: &ModelsCommand, config: &AppConfig) -> Result<()> {
    check_network_allowed(config)?;

    let engine = PolicyEngine::new(config.policy.clone());
    let gate = build_policy_gate(engine, &config.session.approval_mode);
    let client =
        GrokClient::from_config(config, Some(gate)).context("failed to construct API client")?;

    match command {
        ModelsCommand::List { r#type, json } => run_list(&client, r#type, *json).await,
        ModelsCommand::Info { model_id, json } => run_info(&client, model_id, *json).await,
        ModelsCommand::Pricing { json } => run_pricing(&client, *json).await,
    }
}

/// Format modalities as "input->output" or "-" if both empty.
fn format_modalities(input: &[String], output: &[String]) -> String {
    if input.is_empty() && output.is_empty() {
        "-".to_string()
    } else {
        let i = input.join(",");
        let o = output.join(",");
        format!("{i}->{o}")
    }
}

/// Format a price with optional color, returning the display string.
fn fmt_price_colored(price: Option<i64>, use_color: bool) -> String {
    let s = fmt_price(price);
    price.map(|p| price_color(p, &s, use_color)).unwrap_or(s)
}

/// Print JSON-serialized output and return early if `json_output` is true.
/// Returns `true` if JSON was printed (caller should return `Ok(())`).
fn print_json_if_requested<T: serde::Serialize>(data: &T, json_output: bool) -> Result<bool> {
    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(data).context("failed to serialize model list")?
        );
        Ok(true)
    } else {
        Ok(false)
    }
}

/// List language models with pricing table.
fn print_language_models(models: &[grokrs_api::types::model::LanguageModel], use_color: bool) {
    println!(
        "{:<30} {:<12} {:<12} {:<12} {:<25} {:<15}",
        bold("MODEL ID", use_color),
        "PROMPT",
        "COMPLETION",
        "CACHED",
        "MODALITIES",
        "CONTEXT"
    );
    println!("{}", "-".repeat(106));

    for m in models {
        let modalities = format_modalities(&m.input_modalities, &m.output_modalities);
        let context = m
            .max_prompt_length
            .map_or_else(|| "-".into(), |l| format!("{l}"));
        println!(
            "{:<30} {:<12} {:<12} {:<12} {:<25} {:<15}",
            bold(&m.id, use_color),
            fmt_price_colored(m.prompt_text_token_price, use_color),
            fmt_price_colored(m.completion_text_token_price, use_color),
            fmt_price_colored(m.cached_prompt_text_token_price, use_color),
            modalities,
            context,
        );
    }
    println!("\nPrices are in integer ticks (cents per 100M tokens).");
    println!("Total: {} language models", models.len());
}

/// List media models (image or video) with a single price column.
fn print_media_models(
    models: &[(String, String, Vec<String>, Vec<String>, Option<i64>)],
    price_header: &str,
    kind: &str,
    use_color: bool,
) {
    println!(
        "{:<30} {:<15} {:<25} {:<15}",
        bold("MODEL ID", use_color),
        price_header,
        "MODALITIES",
        "OWNED BY"
    );
    println!("{}", "-".repeat(85));

    for (id, owned_by, input_mod, output_mod, price) in models {
        let modalities = format_modalities(input_mod, output_mod);
        println!(
            "{:<30} {:<15} {:<25} {:<15}",
            bold(id, use_color),
            fmt_price_colored(*price, use_color),
            modalities,
            owned_by,
        );
    }
    println!("\nTotal: {} {kind} models", models.len());
}

/// List models by type.
async fn run_list(client: &GrokClient, model_type: &str, json_output: bool) -> Result<()> {
    let use_color = is_tty() && !json_output;

    match model_type {
        "language" => {
            let list = client
                .models()
                .list_language_models()
                .await
                .context("failed to list language models")?;
            if print_json_if_requested(&list, json_output)? {
                return Ok(());
            }
            if list.models.is_empty() {
                println!("No language models available.");
                return Ok(());
            }
            print_language_models(&list.models, use_color);
        }
        "image" => {
            let list = client
                .models()
                .list_image_models()
                .await
                .context("failed to list image models")?;
            if print_json_if_requested(&list, json_output)? {
                return Ok(());
            }
            if list.models.is_empty() {
                println!("No image generation models available.");
                return Ok(());
            }
            let rows: Vec<_> = list
                .models
                .iter()
                .map(|m| {
                    (
                        m.id.clone(),
                        m.owned_by.clone(),
                        m.input_modalities.clone(),
                        m.output_modalities.clone(),
                        m.per_image_price,
                    )
                })
                .collect();
            print_media_models(&rows, "PER IMAGE", "image generation", use_color);
        }
        "video" => {
            let list = client
                .models()
                .list_video_models()
                .await
                .context("failed to list video models")?;
            if print_json_if_requested(&list, json_output)? {
                return Ok(());
            }
            if list.models.is_empty() {
                println!("No video generation models available.");
                return Ok(());
            }
            let rows: Vec<_> = list
                .models
                .iter()
                .map(|m| {
                    (
                        m.id.clone(),
                        m.owned_by.clone(),
                        m.input_modalities.clone(),
                        m.output_modalities.clone(),
                        m.per_second_price,
                    )
                })
                .collect();
            print_media_models(&rows, "PER SECOND", "video generation", use_color);
        }
        other => bail!(
            "unknown model type: '{other}'\n\
             Valid values: language, image, video"
        ),
    }

    Ok(())
}

/// Print common model metadata fields (ID, `owned_by`, created, modalities, etc.).
fn print_model_common(
    title: &str,
    id: &str,
    owned_by: &str,
    created: i64,
    input_modalities: &[String],
    output_modalities: &[String],
    version: Option<&str>,
    fingerprint: Option<&str>,
    use_color: bool,
) {
    println!("{}", bold(title, use_color));
    println!("{}", "-".repeat(50));
    println!("ID:             {}", bold(id, use_color));
    println!("Owned By:       {owned_by}");
    println!("Created:        {created}");
    if !input_modalities.is_empty() {
        println!("Input:          {}", input_modalities.join(", "));
    }
    if !output_modalities.is_empty() {
        println!("Output:         {}", output_modalities.join(", "));
    }
    if let Some(v) = version {
        println!("Version:        {v}");
    }
    if let Some(fp) = fingerprint {
        println!("Fingerprint:    {fp}");
    }
}

/// Show detailed information for a single model.
async fn run_info(client: &GrokClient, model_id: &str, json_output: bool) -> Result<()> {
    let use_color = is_tty() && !json_output;

    // Try language model first, then image, then video.
    if let Ok(model) = client.models().get_language_model(model_id).await {
        if print_json_if_requested(&model, json_output)? {
            return Ok(());
        }

        print_model_common(
            "Language Model",
            &model.id,
            &model.owned_by,
            model.created,
            &model.input_modalities,
            &model.output_modalities,
            model.version.as_deref(),
            model.fingerprint.as_deref(),
            use_color,
        );
        if !model.aliases.is_empty() {
            println!("Aliases:        {}", model.aliases.join(", "));
        }
        if let Some(max) = model.max_prompt_length {
            println!("Context Window: {max} tokens");
        }

        println!();
        println!("{}", bold("Pricing (cents per 100M tokens)", use_color));
        println!("{}", "-".repeat(50));
        for (label, price) in [
            ("Prompt Text", model.prompt_text_token_price),
            ("Completion Text", model.completion_text_token_price),
            ("Cached Prompt", model.cached_prompt_text_token_price),
            ("Prompt Image", model.prompt_image_token_price),
            ("Search", model.search_price),
            ("Image Gen", model.image_price),
        ] {
            println!("  {label:<20} {}", fmt_price_colored(price, use_color));
        }
        return Ok(());
    }

    if let Ok(model) = client.models().get_image_model(model_id).await {
        if print_json_if_requested(&model, json_output)? {
            return Ok(());
        }
        print_model_common(
            "Image Generation Model",
            &model.id,
            &model.owned_by,
            model.created,
            &model.input_modalities,
            &model.output_modalities,
            model.version.as_deref(),
            model.fingerprint.as_deref(),
            use_color,
        );
        println!("Per Image:      {}", fmt_price(model.per_image_price));
        return Ok(());
    }

    if let Ok(model) = client.models().get_video_model(model_id).await {
        if print_json_if_requested(&model, json_output)? {
            return Ok(());
        }
        print_model_common(
            "Video Generation Model",
            &model.id,
            &model.owned_by,
            model.created,
            &model.input_modalities,
            &model.output_modalities,
            model.version.as_deref(),
            model.fingerprint.as_deref(),
            use_color,
        );
        println!("Per Second:     {}", fmt_price(model.per_second_price));
        return Ok(());
    }

    bail!(
        "model '{model_id}' not found.\n\
         Use 'grokrs models list' to see available models."
    );
}

/// Show pricing comparison sorted by cost.
async fn run_pricing(client: &GrokClient, json_output: bool) -> Result<()> {
    let use_color = is_tty() && !json_output;

    let list = client
        .models()
        .list_language_models()
        .await
        .context("failed to list language models")?;

    if json_output {
        // Build a pricing-only view.
        let pricing: Vec<serde_json::Value> = list
            .models
            .iter()
            .map(|m| {
                serde_json::json!({
                    "id": m.id,
                    "prompt_text_token_price": m.prompt_text_token_price,
                    "completion_text_token_price": m.completion_text_token_price,
                    "cached_prompt_text_token_price": m.cached_prompt_text_token_price,
                })
            })
            .collect();

        println!(
            "{}",
            serde_json::to_string_pretty(&pricing).context("failed to serialize pricing")?
        );
        return Ok(());
    }

    if list.models.is_empty() {
        println!("No language models available.");
        return Ok(());
    }

    // Sort by prompt price (cheapest first). Models without pricing go last.
    let mut models = list.models;
    models.sort_by_key(|m| m.prompt_text_token_price.unwrap_or(i64::MAX));

    println!(
        "{:<30} {:<12} {:<12} {:<12}",
        bold("MODEL ID", use_color),
        "PROMPT",
        "COMPLETION",
        "CACHED"
    );
    println!("{}", "-".repeat(66));

    for model in &models {
        let prompt_str = fmt_price(model.prompt_text_token_price);
        let completion_str = fmt_price(model.completion_text_token_price);
        let cached_str = fmt_price(model.cached_prompt_text_token_price);

        let prompt_display = model
            .prompt_text_token_price
            .map(|p| price_color(p, &prompt_str, use_color))
            .unwrap_or(prompt_str);
        let completion_display = model
            .completion_text_token_price
            .map(|p| price_color(p, &completion_str, use_color))
            .unwrap_or(completion_str);
        let cached_display = model
            .cached_prompt_text_token_price
            .map(|p| price_color(p, &cached_str, use_color))
            .unwrap_or(cached_str);

        println!(
            "{:<30} {:<12} {:<12} {:<12}",
            bold(&model.id, use_color),
            prompt_display,
            completion_display,
            cached_display
        );
    }

    println!("\nPrices are in integer ticks (cents per 100M tokens).");
    println!("Sorted: cheapest prompt price first.");
    println!("Total: {} language models", models.len());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- Formatting helper tests ---

    #[test]
    fn fmt_price_with_value() {
        assert_eq!(fmt_price(Some(500)), "500");
    }

    #[test]
    fn fmt_price_without_value() {
        assert_eq!(fmt_price(None), "-");
    }

    #[test]
    fn bold_with_color() {
        let result = bold("test", true);
        assert!(result.contains("\x1b[1m"));
        assert!(result.contains("test"));
        assert!(result.contains("\x1b[0m"));
    }

    #[test]
    fn bold_without_color() {
        let result = bold("test", false);
        assert_eq!(result, "test");
        assert!(!result.contains("\x1b["));
    }

    #[test]
    fn price_color_green_for_cheap() {
        let result = price_color(100, "100", true);
        assert!(result.contains("\x1b[32m")); // green
    }

    #[test]
    fn price_color_yellow_for_moderate() {
        let result = price_color(500, "500", true);
        assert!(result.contains("\x1b[33m")); // yellow
    }

    #[test]
    fn price_color_red_for_expensive() {
        let result = price_color(2000, "2000", true);
        assert!(result.contains("\x1b[31m")); // red
    }

    #[test]
    fn price_color_no_ansi_when_disabled() {
        let result = price_color(100, "100", false);
        assert_eq!(result, "100");
        assert!(!result.contains("\x1b["));
    }

    // --- check_network_allowed tests ---

    #[test]
    fn models_check_network_denied() {
        let config = test_config(false);
        let err = check_network_allowed(&config).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("Network access is denied"));
    }

    #[test]
    fn models_check_network_allowed() {
        let config = test_config(true);
        assert!(check_network_allowed(&config).is_ok());
    }

    fn test_config(allow_network: bool) -> AppConfig {
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
                allow_network,
                allow_shell: false,
                allow_workspace_writes: false,
                max_patch_bytes: 0,
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
