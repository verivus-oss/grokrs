use anyhow::Result;
use clap::Subcommand;
use grokrs_api::auth::{
    configured_api_key_source, configured_management_api_key_source,
    resolve_api_key_with_config, resolve_management_api_key_with_config,
};
use grokrs_core::AppConfig;

#[derive(Debug, Subcommand)]
pub enum AuthCommand {
    /// Validate configured auth resolution and report the source safely.
    Doctor,
    /// Show the configured auth source without resolving the secret.
    ShowSource,
    /// Resolve the configured auth source and report success or failure.
    Test,
}

pub fn run(command: &AuthCommand, config: &AppConfig) -> Result<()> {
    match command {
        AuthCommand::Doctor => {
            println!("auth_source={}", configured_api_key_source(config.api.as_ref()));
            match resolve_api_key_with_config(config.api.as_ref()) {
                Ok(resolved) => {
                    println!("auth_status=ready");
                    println!("auth_resolved_from={}", resolved.source.summary());
                }
                Err(err) => {
                    println!("auth_status=blocked");
                    println!("auth_error={err}");
                }
            }
            if config.management_api.is_some() {
                println!(
                    "management_auth_source={}",
                    configured_management_api_key_source(config.management_api.as_ref())
                );
                match resolve_management_api_key_with_config(config.management_api.as_ref()) {
                    Ok(resolved) => {
                        println!("management_auth_status=ready");
                        println!("management_auth_resolved_from={}", resolved.source.summary());
                    }
                    Err(err) => {
                        println!("management_auth_status=blocked");
                        println!("management_auth_error={err}");
                    }
                }
            }
        }
        AuthCommand::ShowSource => {
            println!("{}", configured_api_key_source(config.api.as_ref()));
            if config.management_api.is_some() {
                println!(
                    "{}",
                    configured_management_api_key_source(config.management_api.as_ref())
                );
            }
        }
        AuthCommand::Test => {
            match resolve_api_key_with_config(config.api.as_ref()) {
                Ok(resolved) => {
                    println!("ok: {}", resolved.source.summary());
                }
                Err(err) => {
                    println!("error: {err}");
                }
            }
            if config.management_api.is_some() {
                match resolve_management_api_key_with_config(config.management_api.as_ref()) {
                    Ok(resolved) => {
                        println!("management ok: {}", resolved.source.summary());
                    }
                    Err(err) => {
                        println!("management error: {err}");
                    }
                }
            }
        }
    }
    Ok(())
}
