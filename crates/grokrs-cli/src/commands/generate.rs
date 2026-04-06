//! CLI subcommands for image and video generation.
//!
//! `grokrs generate image '<prompt>' --output file.png` uses `ImagesClient::generate()`.
//! `grokrs generate video '<prompt>'` uses `VideosClient::generate()` + polling.
//!
//! These commands use the existing endpoint clients directly — they do NOT
//! need the ToolRegistry or AgentExecutor.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use clap::Subcommand;

use grokrs_api::client::GrokClient;
use grokrs_api::transport::policy_bridge::FnPolicyGate;
use grokrs_api::transport::policy_gate::{PolicyDecision, PolicyGate};
use grokrs_api::types::images::{
    AspectRatio, ImageEditRequest, ImageGenerationRequest, ImageResolution, ImageResponseFormat,
};
use grokrs_api::types::videos::{
    VideoDuration, VideoExtensionDuration, VideoExtensionRequest, VideoGenerationRequest,
};
use grokrs_cap::{WorkspacePath, WorkspaceRoot};
use grokrs_core::AppConfig;
use grokrs_policy::{Decision, Effect, PolicyEngine};

/// Generate subcommand group.
#[derive(Subcommand)]
#[command(after_help = "\
Examples:
  grokrs generate image 'a sunset over mountains' -o sunset.png
  grokrs generate image 'logo design' -o logo.png --aspect-ratio 1:1 --quality hd
  grokrs generate image 'enhance this' -o out.png --edit input.png
  grokrs generate video 'a cat walking' --duration 5
  grokrs generate video 'extend clip' --extend clip.mp4 --duration 3")]
pub enum GenerateCommand {
    /// Generate an image from a text prompt.
    Image {
        /// The text prompt describing the desired image.
        prompt: String,

        /// Output file path (required, workspace-relative).
        #[arg(long, short)]
        output: PathBuf,

        /// Model to use for generation.
        #[arg(long)]
        model: Option<String>,

        /// Aspect ratio (1:1, 16:9, 9:16, 4:3, 3:4, 3:2, 2:3).
        #[arg(long)]
        aspect_ratio: Option<String>,

        /// Quality level (e.g., "standard", "hd").
        #[arg(long)]
        quality: Option<String>,

        /// Resolution (1024x1024, 1024x1792, 1792x1024).
        #[arg(long)]
        resolution: Option<String>,

        /// Response format: "url" or "b64" (default: url).
        #[arg(long, default_value = "url")]
        format: String,

        /// Input image for editing (workspace-relative path or URL).
        #[arg(long)]
        edit: Option<String>,
    },

    /// Generate a video from a text prompt.
    Video {
        /// The text prompt describing the desired video.
        prompt: String,

        /// Model to use for generation.
        #[arg(long)]
        model: Option<String>,

        /// Aspect ratio (1:1, 16:9, 9:16, 4:3, 3:4, 3:2, 2:3).
        #[arg(long)]
        aspect_ratio: Option<String>,

        /// Duration of the generated video in seconds (1-15 for generation, 1-10 for extension).
        #[arg(long)]
        duration: Option<u32>,

        /// Resolution (e.g., "720p", "1080p").
        #[arg(long)]
        resolution: Option<String>,

        /// Extend an existing video (URL or workspace-relative path).
        #[arg(long)]
        extend: Option<String>,
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
             The generate command requires network access to communicate with the xAI API.\n\
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

/// Parse an aspect ratio string into an `AspectRatio` enum variant.
fn parse_aspect_ratio(s: &str) -> Result<AspectRatio> {
    match s {
        "1:1" => Ok(AspectRatio::Square),
        "16:9" => Ok(AspectRatio::Wide),
        "9:16" => Ok(AspectRatio::Tall),
        "4:3" => Ok(AspectRatio::Landscape),
        "3:4" => Ok(AspectRatio::Portrait),
        "3:2" => Ok(AspectRatio::ClassicLandscape),
        "2:3" => Ok(AspectRatio::ClassicPortrait),
        other => bail!(
            "unknown aspect ratio: '{other}'\n\
             Valid values: 1:1, 16:9, 9:16, 4:3, 3:4, 3:2, 2:3"
        ),
    }
}

/// Parse an image resolution string.
fn parse_image_resolution(s: &str) -> Result<ImageResolution> {
    match s {
        "1024x1024" => Ok(ImageResolution::Res1024x1024),
        "1024x1792" => Ok(ImageResolution::Res1024x1792),
        "1792x1024" => Ok(ImageResolution::Res1792x1024),
        other => bail!(
            "unknown image resolution: '{other}'\n\
             Valid values: 1024x1024, 1024x1792, 1792x1024"
        ),
    }
}

/// Parse image response format string.
fn parse_image_format(s: &str) -> Result<ImageResponseFormat> {
    match s {
        "url" => Ok(ImageResponseFormat::Url),
        "b64" | "b64_json" => Ok(ImageResponseFormat::B64Json),
        other => bail!(
            "unknown image format: '{other}'\n\
             Valid values: url, b64"
        ),
    }
}

/// Validate an output path through the WorkspacePath/policy system.
///
/// Returns the validated WorkspacePath. Ensures the output path is
/// workspace-relative and FsWrite is allowed by policy.
fn validate_output_path(
    output: &std::path::Path,
    engine: &PolicyEngine,
    approval_mode: &str,
) -> Result<WorkspacePath> {
    let path_str = output.to_str().context("output path must be valid UTF-8")?;

    let wp = WorkspacePath::new(path_str)
        .with_context(|| format!("invalid output path '{path_str}': must be workspace-relative"))?;

    let effect = Effect::FsWrite(wp.clone());
    let decision = engine.evaluate(&effect);

    match decision {
        Decision::Allow { .. } => Ok(wp),
        Decision::Ask { reason } => match approval_mode {
            "allow" => Ok(wp),
            _ => bail!(
                "writing to '{path_str}' requires approval: {reason}\n\
                 Set approval_mode = \"allow\" in [session] to bypass."
            ),
        },
        Decision::Deny { reason } => bail!(
            "writing to '{path_str}' denied by policy: {reason}\n\
             To enable workspace writes, set `allow_workspace_writes = true` in [policy]."
        ),
    }
}

/// Execute the `grokrs generate` command.
pub async fn run(command: &GenerateCommand, config: &AppConfig) -> Result<()> {
    check_network_allowed(config)?;

    let engine = PolicyEngine::new(config.policy.clone());
    let gate = build_policy_gate(engine.clone(), &config.session.approval_mode);
    let client =
        GrokClient::from_config(config, Some(gate)).context("failed to construct API client")?;

    match command {
        GenerateCommand::Image {
            prompt,
            output,
            model,
            aspect_ratio,
            quality,
            resolution,
            format,
            edit,
        } => {
            run_image(
                &client,
                &engine,
                config,
                prompt,
                output,
                model.as_deref(),
                aspect_ratio.as_deref(),
                quality.as_deref(),
                resolution.as_deref(),
                format,
                edit.as_deref(),
            )
            .await
        }
        GenerateCommand::Video {
            prompt,
            model,
            aspect_ratio,
            duration,
            resolution,
            extend,
        } => {
            run_video(
                &client,
                config,
                prompt,
                model.as_deref(),
                aspect_ratio.as_deref(),
                *duration,
                resolution.as_deref(),
                extend.as_deref(),
            )
            .await
        }
    }
}

/// Generate or edit an image.
#[allow(clippy::too_many_arguments)]
async fn run_image(
    client: &GrokClient,
    engine: &PolicyEngine,
    config: &AppConfig,
    prompt: &str,
    output: &std::path::Path,
    model: Option<&str>,
    aspect_ratio: Option<&str>,
    quality: Option<&str>,
    resolution: Option<&str>,
    format: &str,
    edit_input: Option<&str>,
) -> Result<()> {
    // Validate output path.
    let _wp = validate_output_path(output, engine, &config.session.approval_mode)?;

    let image_model = model.unwrap_or("grok-2-image");
    let response_format = parse_image_format(format)?;
    let ar = aspect_ratio.map(parse_aspect_ratio).transpose()?;
    let res = resolution.map(parse_image_resolution).transpose()?;

    let workspace_root = WorkspaceRoot::new(
        &std::env::current_dir().context("failed to resolve current directory")?,
    )
    .context("failed to construct workspace root")?;

    let response = if let Some(edit_source) = edit_input {
        // Image editing mode.
        // Validate the input path if it looks workspace-relative (not a URL).
        let image_source =
            if edit_source.starts_with("http://") || edit_source.starts_with("https://") {
                edit_source.to_string()
            } else {
                // Validate as workspace path with FsRead effect.
                let input_wp = WorkspacePath::new(edit_source).with_context(|| {
                    format!("invalid edit input path '{edit_source}': must be workspace-relative")
                })?;
                let read_effect = Effect::FsRead(input_wp);
                let decision = engine.evaluate(&read_effect);
                match decision {
                    Decision::Allow { .. } => {}
                    Decision::Ask { .. } if config.session.approval_mode == "allow" => {}
                    Decision::Ask { reason } => {
                        bail!("reading '{edit_source}' requires approval: {reason}")
                    }
                    Decision::Deny { reason } => {
                        bail!("reading '{edit_source}' denied by policy: {reason}")
                    }
                }

                // Read the file and encode as base64 URL.
                let abs_path = workspace_root.join(&WorkspacePath::new(edit_source)?);
                if !abs_path.exists() {
                    bail!("edit input file does not exist: {}", abs_path.display());
                }
                let bytes = std::fs::read(&abs_path)
                    .with_context(|| format!("failed to read {}", abs_path.display()))?;
                use base64::Engine as _;
                let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
                format!("data:image/png;base64,{b64}")
            };

        let request = ImageEditRequest {
            prompt: prompt.to_string(),
            model: image_model.to_string(),
            image: Some(image_source),
            images: None,
            mask: None,
        };

        client
            .images()
            .edit(&request)
            .await
            .context("image edit request failed")?
    } else {
        // Image generation mode.
        let request = ImageGenerationRequest {
            prompt: prompt.to_string(),
            model: image_model.to_string(),
            n: Some(1),
            aspect_ratio: ar,
            quality: quality.map(|s| s.to_string()),
            resolution: res,
            response_format: Some(response_format),
        };

        client
            .images()
            .generate(&request)
            .await
            .context("image generation request failed")?
    };

    if response.data.is_empty() {
        bail!("API returned no image data");
    }

    let image_data = &response.data[0];

    // Resolve output path relative to workspace.
    let output_abs = workspace_root.join(&WorkspacePath::new(
        output.to_str().context("output path must be valid UTF-8")?,
    )?);

    // Create parent directories if needed.
    if let Some(parent) = output_abs.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create directory {}", parent.display()))?;
    }

    if let Some(ref url) = image_data.url {
        // Download the image from the URL.
        eprintln!("[generate] downloading image from URL...");
        let bytes = reqwest::get(url)
            .await
            .context("failed to download image")?
            .bytes()
            .await
            .context("failed to read image bytes")?;

        if output_abs.exists() {
            eprintln!(
                "[generate] warning: overwriting existing file {}",
                output_abs.display()
            );
        }

        std::fs::write(&output_abs, &bytes)
            .with_context(|| format!("failed to write image to {}", output_abs.display()))?;

        eprintln!(
            "[generate] wrote {} bytes to {}",
            bytes.len(),
            output_abs.display()
        );
    } else if let Some(ref b64) = image_data.b64_json {
        // Decode base64 and write.
        use base64::Engine as _;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(b64)
            .context("failed to decode base64 image data")?;

        if output_abs.exists() {
            eprintln!(
                "[generate] warning: overwriting existing file {}",
                output_abs.display()
            );
        }

        std::fs::write(&output_abs, &bytes)
            .with_context(|| format!("failed to write image to {}", output_abs.display()))?;

        eprintln!(
            "[generate] wrote {} bytes to {}",
            bytes.len(),
            output_abs.display()
        );
    } else {
        bail!("API returned image data with neither URL nor base64 content");
    }

    if let Some(ref revised) = image_data.revised_prompt {
        eprintln!("[generate] revised prompt: {revised}");
    }

    Ok(())
}

/// Generate or extend a video.
#[allow(clippy::too_many_arguments)]
async fn run_video(
    client: &GrokClient,
    _config: &AppConfig,
    prompt: &str,
    model: Option<&str>,
    aspect_ratio: Option<&str>,
    duration: Option<u32>,
    resolution: Option<&str>,
    extend_source: Option<&str>,
) -> Result<()> {
    let ar = aspect_ratio.map(parse_aspect_ratio).transpose()?;

    let submit_response = if let Some(source) = extend_source {
        // Video extension mode.
        let ext_duration = duration
            .map(VideoExtensionDuration::new)
            .transpose()
            .map_err(|e| anyhow::anyhow!("{e}"))?;

        let request = VideoExtensionRequest {
            prompt: prompt.to_string(),
            video: source.to_string(),
            duration: ext_duration,
            model: model.map(|s| s.to_string()),
        };

        eprintln!("[generate] submitting video extension request...");
        client
            .videos()
            .extend(&request)
            .await
            .context("video extension request failed")?
    } else {
        // Video generation mode.
        let vid_duration = duration
            .map(VideoDuration::new)
            .transpose()
            .map_err(|e| anyhow::anyhow!("{e}"))?;

        let request = VideoGenerationRequest {
            prompt: prompt.to_string(),
            model: model.map(|s| s.to_string()),
            image: None,
            reference_images: None,
            duration: vid_duration,
            aspect_ratio: ar,
            resolution: resolution.map(|s| s.to_string()),
        };

        eprintln!("[generate] submitting video generation request...");
        client
            .videos()
            .generate(&request)
            .await
            .context("video generation request failed")?
    };

    let request_id = &submit_response.request_id;
    eprintln!("[generate] request_id={request_id}");
    eprintln!("[generate] polling for completion...");

    let start = Instant::now();
    let poll_interval = Duration::from_secs(5);
    let max_polls = 120; // 10 minutes at 5s intervals

    let result = client
        .videos()
        .poll_until_done(request_id, poll_interval, max_polls)
        .await;

    match result {
        Ok(response) => {
            let elapsed = start.elapsed();
            eprintln!("[generate] video ready in {:.1}s", elapsed.as_secs_f64());

            if let Some(video) = response.video {
                // Print the video URL to stdout (machine-readable).
                println!("{}", video.url);
            } else {
                bail!("video completed but no URL was returned");
            }
        }
        Err(e) => {
            let elapsed = start.elapsed();
            eprintln!("[generate] failed after {:.1}s", elapsed.as_secs_f64());
            bail!("video generation failed: {e}");
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use grokrs_core::PolicyConfig;

    fn test_engine(allow_writes: bool) -> PolicyEngine {
        PolicyEngine::new(PolicyConfig {
            allow_network: true,
            allow_shell: false,
            allow_workspace_writes: allow_writes,
            max_patch_bytes: 1024,
        })
    }

    // --- parse_aspect_ratio tests ---

    #[test]
    fn parse_aspect_ratio_square() {
        assert_eq!(parse_aspect_ratio("1:1").unwrap(), AspectRatio::Square);
    }

    #[test]
    fn parse_aspect_ratio_wide() {
        assert_eq!(parse_aspect_ratio("16:9").unwrap(), AspectRatio::Wide);
    }

    #[test]
    fn parse_aspect_ratio_tall() {
        assert_eq!(parse_aspect_ratio("9:16").unwrap(), AspectRatio::Tall);
    }

    #[test]
    fn parse_aspect_ratio_landscape() {
        assert_eq!(parse_aspect_ratio("4:3").unwrap(), AspectRatio::Landscape);
    }

    #[test]
    fn parse_aspect_ratio_portrait() {
        assert_eq!(parse_aspect_ratio("3:4").unwrap(), AspectRatio::Portrait);
    }

    #[test]
    fn parse_aspect_ratio_classic_landscape() {
        assert_eq!(
            parse_aspect_ratio("3:2").unwrap(),
            AspectRatio::ClassicLandscape
        );
    }

    #[test]
    fn parse_aspect_ratio_classic_portrait() {
        assert_eq!(
            parse_aspect_ratio("2:3").unwrap(),
            AspectRatio::ClassicPortrait
        );
    }

    #[test]
    fn parse_aspect_ratio_unknown_is_error() {
        let err = parse_aspect_ratio("5:4").unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("5:4"), "error should mention the value: {msg}");
    }

    // --- parse_image_resolution tests ---

    #[test]
    fn parse_resolution_1024x1024() {
        assert_eq!(
            parse_image_resolution("1024x1024").unwrap(),
            ImageResolution::Res1024x1024
        );
    }

    #[test]
    fn parse_resolution_1024x1792() {
        assert_eq!(
            parse_image_resolution("1024x1792").unwrap(),
            ImageResolution::Res1024x1792
        );
    }

    #[test]
    fn parse_resolution_1792x1024() {
        assert_eq!(
            parse_image_resolution("1792x1024").unwrap(),
            ImageResolution::Res1792x1024
        );
    }

    #[test]
    fn parse_resolution_unknown_is_error() {
        let err = parse_image_resolution("512x512").unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("512x512"));
    }

    // --- parse_image_format tests ---

    #[test]
    fn parse_format_url() {
        assert_eq!(parse_image_format("url").unwrap(), ImageResponseFormat::Url);
    }

    #[test]
    fn parse_format_b64() {
        assert_eq!(
            parse_image_format("b64").unwrap(),
            ImageResponseFormat::B64Json
        );
    }

    #[test]
    fn parse_format_b64_json() {
        assert_eq!(
            parse_image_format("b64_json").unwrap(),
            ImageResponseFormat::B64Json
        );
    }

    #[test]
    fn parse_format_unknown_is_error() {
        let err = parse_image_format("jpeg").unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("jpeg"));
    }

    // --- validate_output_path tests ---

    #[test]
    fn validate_output_path_allows_workspace_relative() {
        let engine = test_engine(true);
        let result =
            validate_output_path(std::path::Path::new("output/image.png"), &engine, "allow");
        assert!(result.is_ok());
    }

    #[test]
    fn validate_output_path_rejects_absolute() {
        let engine = test_engine(true);
        let result = validate_output_path(std::path::Path::new("/tmp/image.png"), &engine, "allow");
        assert!(result.is_err());
        let msg = format!("{}", result.unwrap_err());
        assert!(msg.contains("workspace-relative"), "msg: {msg}");
    }

    #[test]
    fn validate_output_path_rejects_traversal() {
        let engine = test_engine(true);
        let result = validate_output_path(
            std::path::Path::new("../escape/image.png"),
            &engine,
            "allow",
        );
        assert!(result.is_err());
    }

    #[test]
    fn validate_output_path_denied_by_policy() {
        let engine = test_engine(false);
        let result =
            validate_output_path(std::path::Path::new("output/image.png"), &engine, "deny");
        assert!(result.is_err());
        let msg = format!("{}", result.unwrap_err());
        assert!(
            msg.contains("denied") || msg.contains("approval"),
            "msg: {msg}"
        );
    }

    // --- check_network_allowed tests ---

    #[test]
    fn generate_check_network_denied() {
        let config = test_app_config(false);
        let err = check_network_allowed(&config).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("Network access is denied"));
    }

    #[test]
    fn generate_check_network_allowed() {
        let config = test_app_config(true);
        assert!(check_network_allowed(&config).is_ok());
    }

    fn test_app_config(allow_network: bool) -> AppConfig {
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
                allow_workspace_writes: true,
                max_patch_bytes: 1024,
            },
            session: SessionConfig {
                approval_mode: "allow".into(),
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
