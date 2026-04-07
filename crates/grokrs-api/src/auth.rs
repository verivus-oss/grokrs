use std::env;
use std::future::Future;

use azure_identity::DeveloperToolsCredential;
use azure_security_keyvault_secrets::SecretClient;
use grokrs_core::{
    ApiAuthConfig, ApiAuthProvider, ApiConfig, AzureKeyVaultAuthConfig, ManagementApiConfig,
};

use crate::transport::auth::ApiKeySecret;
use crate::transport::error::TransportError;

const DEFAULT_API_KEY_ENV: &str = "XAI_API_KEY";
const DEFAULT_MANAGEMENT_API_KEY_ENV: &str = "XAI_MANAGEMENT_API_KEY";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApiKeySource {
    Environment { env_var: String },
    AzureKeyVault {
        vault_name: String,
        secret_name: String,
    },
}

#[derive(Debug, Clone)]
pub struct ResolvedApiKey {
    pub secret: ApiKeySecret,
    pub source: ApiKeySource,
}

impl ApiKeySource {
    #[must_use]
    pub fn summary(&self) -> String {
        match self {
            Self::Environment { env_var } => format!("env:{env_var}"),
            Self::AzureKeyVault {
                vault_name,
                secret_name,
            } => {
                format!("azure_key_vault:{vault_name}/{secret_name}")
            }
        }
    }
}

pub fn configured_api_key_env(api_config: Option<&ApiConfig>) -> &str {
    api_config
        .and_then(|a| a.api_key_env.as_deref())
        .unwrap_or(DEFAULT_API_KEY_ENV)
}

pub fn configured_api_key_source(api_config: Option<&ApiConfig>) -> String {
    configured_source_from_parts(
        configured_api_key_env(api_config),
        api_config.and_then(|api| api.auth.as_ref()),
    )
}

pub fn resolve_api_key_with_config(
    api_config: Option<&ApiConfig>,
) -> Result<ResolvedApiKey, TransportError> {
    let env_var = configured_api_key_env(api_config).to_owned();
    match env::var(&env_var) {
        Ok(val) if val.is_empty() => {
            return Err(TransportError::Auth {
                message: format!("environment variable '{env_var}' is set but empty"),
            });
        }
        Ok(val) => {
            return Ok(ResolvedApiKey {
                secret: ApiKeySecret::new(val),
                source: ApiKeySource::Environment { env_var },
            });
        }
        Err(env::VarError::NotPresent) => {}
        Err(env::VarError::NotUnicode(_)) => {
            return Err(TransportError::Auth {
                message: format!("environment variable '{env_var}' is not valid UTF-8"),
            });
        }
    }

    match api_config
        .and_then(|api| api.auth.as_ref())
        .and_then(|auth| auth.provider)
    {
        Some(ApiAuthProvider::AzureKeyVault) => {
            let kv = api_config
                .and_then(|api| api.auth.as_ref())
                .and_then(|auth| auth.azure_key_vault.as_ref())
                .ok_or_else(|| TransportError::Auth {
                    message:
                        "api.auth.provider is 'azure_key_vault' but [api.auth.azure_key_vault] is missing"
                            .to_owned(),
                })?;
            resolve_azure_key_vault_secret(kv)
        }
        Some(ApiAuthProvider::Env) | None => Err(TransportError::Auth {
            message: format!("environment variable '{env_var}' is not set"),
        }),
    }
}

pub fn configured_management_api_key_env(
    mgmt_config: Option<&ManagementApiConfig>,
) -> &str {
    mgmt_config
        .and_then(|m| m.management_key_env.as_deref())
        .unwrap_or(DEFAULT_MANAGEMENT_API_KEY_ENV)
}

pub fn configured_management_api_key_source(
    mgmt_config: Option<&ManagementApiConfig>,
) -> String {
    configured_source_from_parts(
        configured_management_api_key_env(mgmt_config),
        mgmt_config.and_then(|mgmt| mgmt.auth.as_ref()),
    )
}

pub fn resolve_management_api_key_with_config(
    mgmt_config: Option<&ManagementApiConfig>,
) -> Result<ResolvedApiKey, TransportError> {
    let env_var = configured_management_api_key_env(mgmt_config).to_owned();
    resolve_from_env_then_provider(&env_var, mgmt_config.and_then(|mgmt| mgmt.auth.as_ref()))
}

fn configured_source_from_parts(env_var: &str, auth: Option<&ApiAuthConfig>) -> String {
    match auth.and_then(|auth| auth.provider) {
        Some(ApiAuthProvider::AzureKeyVault) => {
            if let Some(kv) = auth.and_then(|auth| auth.azure_key_vault.as_ref()) {
                return ApiKeySource::AzureKeyVault {
                    vault_name: kv.vault_name.clone(),
                    secret_name: kv.secret_name.clone(),
                }
                .summary();
            }
            "azure_key_vault:unconfigured".to_owned()
        }
        Some(ApiAuthProvider::Env) | None => ApiKeySource::Environment {
            env_var: env_var.to_owned(),
        }
        .summary(),
    }
}

fn resolve_from_env_then_provider(
    env_var: &str,
    auth: Option<&ApiAuthConfig>,
) -> Result<ResolvedApiKey, TransportError> {
    match env::var(env_var) {
        Ok(val) if val.is_empty() => {
            return Err(TransportError::Auth {
                message: format!("environment variable '{env_var}' is set but empty"),
            });
        }
        Ok(val) => {
            return Ok(ResolvedApiKey {
                secret: ApiKeySecret::new(val),
                source: ApiKeySource::Environment {
                    env_var: env_var.to_owned(),
                },
            });
        }
        Err(env::VarError::NotPresent) => {}
        Err(env::VarError::NotUnicode(_)) => {
            return Err(TransportError::Auth {
                message: format!("environment variable '{env_var}' is not valid UTF-8"),
            });
        }
    }

    match auth.and_then(|auth| auth.provider) {
        Some(ApiAuthProvider::AzureKeyVault) => {
            let kv = auth
                .and_then(|auth| auth.azure_key_vault.as_ref())
                .ok_or_else(|| TransportError::Auth {
                    message:
                        "auth.provider is 'azure_key_vault' but [*.auth.azure_key_vault] is missing"
                            .to_owned(),
                })?;
            resolve_azure_key_vault_secret(kv)
        }
        Some(ApiAuthProvider::Env) | None => Err(TransportError::Auth {
            message: format!("environment variable '{env_var}' is not set"),
        }),
    }
}

fn resolve_azure_key_vault_secret(
    config: &AzureKeyVaultAuthConfig,
) -> Result<ResolvedApiKey, TransportError> {
    let value = block_on_auth_future(fetch_azure_key_vault_secret(config))?;
    if value.is_empty() {
        return Err(TransportError::Auth {
            message: format!(
                "Azure Key Vault secret '{}/{}' resolved but was empty",
                config.vault_name, config.secret_name
            ),
        });
    }
    Ok(ResolvedApiKey {
        secret: ApiKeySecret::new(value),
        source: ApiKeySource::AzureKeyVault {
            vault_name: config.vault_name.clone(),
            secret_name: config.secret_name.clone(),
        },
    })
}

async fn fetch_azure_key_vault_secret(
    config: &AzureKeyVaultAuthConfig,
) -> Result<String, TransportError> {
    let credential = DeveloperToolsCredential::new(None).map_err(|e| TransportError::Auth {
        message: format!("failed to initialize Azure developer credential chain: {e}"),
    })?;
    let vault_url = format!("https://{}.vault.azure.net/", config.vault_name);
    let client =
        SecretClient::new(&vault_url, credential, None).map_err(|e| TransportError::Auth {
            message: format!("failed to initialize Azure Key Vault client: {e}"),
        })?;

    let response = client
        .get_secret(&config.secret_name, None)
        .await
        .map_err(|e| TransportError::Auth {
            message: classify_azure_error(&e.to_string(), config),
        })?;
    let secret = response.into_model().map_err(|e| TransportError::Auth {
        message: format!("failed to decode Azure Key Vault secret response: {e}"),
    })?;
    let value = secret.value.ok_or_else(|| TransportError::Auth {
        message: format!(
            "Azure Key Vault secret '{}/{}' resolved without a secret value",
            config.vault_name, config.secret_name
        ),
    })?;
    Ok(value)
}

fn classify_azure_error(error_text: &str, config: &AzureKeyVaultAuthConfig) -> String {
    let normalized = error_text.to_ascii_lowercase();
    if normalized.contains("secretdisabled") || normalized.contains("disabled") {
        return format!(
            "Azure Key Vault secret '{}/{}' is disabled",
            config.vault_name, config.secret_name
        );
    }
    if normalized.contains("secretnotfound") || normalized.contains("not found") {
        return format!(
            "Azure Key Vault secret '{}/{}' was not found",
            config.vault_name, config.secret_name
        );
    }
    if normalized.contains("vault") && normalized.contains("not found") {
        return format!("Azure Key Vault '{}' was not found", config.vault_name);
    }
    if normalized.contains("please run 'az login'")
        || normalized.contains("az login")
        || normalized.contains("authentication")
        || normalized.contains("unauthorized")
    {
        return "Azure authentication failed for Key Vault access. Ensure Azure CLI or developer credentials are active".to_owned();
    }
    format!(
        "Azure Key Vault lookup failed for '{}/{}': {}",
        config.vault_name,
        config.secret_name,
        error_text.trim()
    )
}

fn block_on_auth_future<F, T>(future: F) -> Result<T, TransportError>
where
    F: Future<Output = Result<T, TransportError>>,
{
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => tokio::task::block_in_place(|| handle.block_on(future)),
        Err(_) => {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| TransportError::Auth {
                    message: format!("failed to build tokio runtime for auth provider: {e}"),
                })?;
            rt.block_on(future)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use grokrs_core::{ApiAuthConfig, ApiConfig};
    use serial_test::serial;
    #[test]
    #[serial]
    fn resolve_api_key_with_config_uses_env_first() {
        let var_name = "GROKRS_TEST_API_KEY_PROVIDER_ENV";
        unsafe { env::set_var(var_name, "env-key-value") };
        let config = ApiConfig {
            api_key_env: Some(var_name.to_owned()),
            base_url: None,
            timeout_secs: None,
            max_retries: None,
            auth: Some(ApiAuthConfig {
                provider: Some(ApiAuthProvider::AzureKeyVault),
                azure_key_vault: Some(AzureKeyVaultAuthConfig {
                    vault_name: "v".to_owned(),
                    secret_name: "s".to_owned(),
                }),
            }),
        };
        let resolved = resolve_api_key_with_config(Some(&config)).unwrap();
        unsafe { env::remove_var(var_name) };
        assert_eq!(resolved.secret.expose(), "env-key-value");
        assert_eq!(
            resolved.source,
            ApiKeySource::Environment {
                env_var: var_name.to_owned()
            }
        );
    }

    #[test]
    #[serial]
    fn resolve_api_key_with_config_empty_env_is_error() {
        let var_name = "GROKRS_TEST_API_KEY_PROVIDER_EMPTY";
        unsafe { env::set_var(var_name, "") };
        let config = ApiConfig {
            api_key_env: Some(var_name.to_owned()),
            base_url: None,
            timeout_secs: None,
            max_retries: None,
            auth: None,
        };
        let err = resolve_api_key_with_config(Some(&config)).unwrap_err();
        unsafe { env::remove_var(var_name) };
        match err {
            TransportError::Auth { message } => assert!(message.contains("empty")),
            other => panic!("expected auth error, got: {other}"),
        }
    }

    #[test]
    fn configured_api_key_source_reports_azure_key_vault() {
        let config = ApiConfig {
            api_key_env: Some("XAI_API_KEY".to_owned()),
            base_url: None,
            timeout_secs: None,
            max_retries: None,
            auth: Some(ApiAuthConfig {
                provider: Some(ApiAuthProvider::AzureKeyVault),
                azure_key_vault: Some(AzureKeyVaultAuthConfig {
                    vault_name: "vault".to_owned(),
                    secret_name: "secret".to_owned(),
                }),
            }),
        };
        assert_eq!(
            configured_api_key_source(Some(&config)),
            "azure_key_vault:vault/secret"
        );
    }

    #[test]
    fn configured_management_api_key_source_reports_azure_key_vault() {
        let config = ManagementApiConfig {
            management_key_env: Some("XAI_MANAGEMENT_API_KEY".to_owned()),
            base_url: None,
            timeout_secs: None,
            max_retries: None,
            auth: Some(ApiAuthConfig {
                provider: Some(ApiAuthProvider::AzureKeyVault),
                azure_key_vault: Some(AzureKeyVaultAuthConfig {
                    vault_name: "vault".to_owned(),
                    secret_name: "secret".to_owned(),
                }),
            }),
        };
        assert_eq!(
            configured_management_api_key_source(Some(&config)),
            "azure_key_vault:vault/secret"
        );
    }

    #[test]
    fn classify_azure_error_reports_not_found() {
        let cfg = AzureKeyVaultAuthConfig {
            vault_name: "vault".to_owned(),
            secret_name: "secret".to_owned(),
        };
        let message = classify_azure_error("SecretNotFound", &cfg);
        assert!(message.contains("was not found"));
    }

    #[test]
    fn classify_azure_error_reports_disabled() {
        let cfg = AzureKeyVaultAuthConfig {
            vault_name: "vault".to_owned(),
            secret_name: "secret".to_owned(),
        };
        let message = classify_azure_error("SecretDisabled", &cfg);
        assert!(message.contains("is disabled"));
    }
}
