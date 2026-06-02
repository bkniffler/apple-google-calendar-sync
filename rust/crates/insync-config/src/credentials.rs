use crate::{ConfigError, SecretStoreKind, ServiceConfig, save_config};
use keyring::{Entry, Error as KeyringError};
use std::path::Path;

const SERVICE: &str = "insync";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedCredentials {
    pub google: ResolvedGoogleCredentials,
    pub icloud: ResolvedIcloudCredentials,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedGoogleCredentials {
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
    pub refresh_token: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedIcloudCredentials {
    pub username: Option<String>,
    pub app_specific_password: Option<String>,
    pub caldav_url: String,
}

pub fn resolve_credentials(
    config: &mut ServiceConfig,
    config_path: impl AsRef<Path>,
) -> Result<ResolvedCredentials, ConfigError> {
    match config.secret_store {
        SecretStoreKind::None => Ok(inline_credentials(config)),
        SecretStoreKind::Os => resolve_os_credentials(config, config_path.as_ref()),
    }
}

pub fn store_google_refresh_token(
    config: &mut ServiceConfig,
    config_path: impl AsRef<Path>,
    refresh_token: &str,
) -> Result<(), ConfigError> {
    match config.secret_store {
        SecretStoreKind::None => {
            config.google.refresh_token = Some(refresh_token.to_string());
        }
        SecretStoreKind::Os => {
            set_secret(
                &google_refresh_token_key(&config.google.account_label),
                refresh_token,
            )?;
            config.google.refresh_token = None;
        }
    }

    save_config(config_path, config)
}

pub fn google_client_secret_key(account_label: &str) -> String {
    format!("google:{account_label}:client-secret")
}

pub fn google_refresh_token_key(account_label: &str) -> String {
    format!("google:{account_label}:refresh-token")
}

pub fn icloud_app_password_key(account_label: &str) -> String {
    format!("icloud:{account_label}:app-specific-password")
}

fn inline_credentials(config: &ServiceConfig) -> ResolvedCredentials {
    ResolvedCredentials {
        google: ResolvedGoogleCredentials {
            client_id: config.google.client_id.clone(),
            client_secret: config.google.client_secret.clone(),
            refresh_token: config.google.refresh_token.clone(),
        },
        icloud: ResolvedIcloudCredentials {
            username: config.icloud.username.clone(),
            app_specific_password: config.icloud.app_specific_password.clone(),
            caldav_url: config.icloud.caldav_url.clone(),
        },
    }
}

fn resolve_os_credentials(
    config: &mut ServiceConfig,
    config_path: &Path,
) -> Result<ResolvedCredentials, ConfigError> {
    let mut changed = false;

    if let Some(client_secret) = config.google.client_secret.take() {
        set_secret(
            &google_client_secret_key(&config.google.account_label),
            &client_secret,
        )?;
        changed = true;
    }

    if let Some(refresh_token) = config.google.refresh_token.take() {
        set_secret(
            &google_refresh_token_key(&config.google.account_label),
            &refresh_token,
        )?;
        changed = true;
    }

    if let Some(app_password) = config.icloud.app_specific_password.take() {
        set_secret(
            &icloud_app_password_key(&config.icloud.account_label),
            &app_password,
        )?;
        changed = true;
    }

    if changed {
        save_config(config_path, config)?;
    }

    Ok(ResolvedCredentials {
        google: ResolvedGoogleCredentials {
            client_id: config.google.client_id.clone(),
            client_secret: get_secret(&google_client_secret_key(&config.google.account_label))?,
            refresh_token: get_secret(&google_refresh_token_key(&config.google.account_label))?,
        },
        icloud: ResolvedIcloudCredentials {
            username: config.icloud.username.clone(),
            app_specific_password: get_secret(&icloud_app_password_key(
                &config.icloud.account_label,
            ))?,
            caldav_url: config.icloud.caldav_url.clone(),
        },
    })
}

fn set_secret(key: &str, value: &str) -> Result<(), ConfigError> {
    Entry::new(SERVICE, key)?.set_password(value)?;
    Ok(())
}

fn get_secret(key: &str) -> Result<Option<String>, ConfigError> {
    match Entry::new(SERVICE, key)?.get_password() {
        Ok(value) => Ok(Some(value)),
        Err(KeyringError::NoEntry) => Ok(None),
        Err(error) => Err(ConfigError::SecretStore(error)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{GoogleConfig, IcloudConfig, ServiceConfig};

    #[test]
    fn resolves_inline_credentials_when_secret_store_is_none() {
        let mut config = config();
        config.secret_store = SecretStoreKind::None;

        let credentials = resolve_credentials(&mut config, "unused.json").unwrap();

        assert_eq!(credentials.google.client_id.as_deref(), Some("client-id"));
        assert_eq!(
            credentials.google.client_secret.as_deref(),
            Some("client-secret")
        );
        assert_eq!(
            credentials.google.refresh_token.as_deref(),
            Some("refresh-token")
        );
        assert_eq!(
            credentials.icloud.app_specific_password.as_deref(),
            Some("app-password")
        );
    }

    #[test]
    fn stores_google_refresh_token_inline_when_secret_store_is_none() {
        let path =
            std::env::temp_dir().join(format!("insync-config-{}-refresh.json", std::process::id()));
        let mut config = config();
        config.secret_store = SecretStoreKind::None;

        store_google_refresh_token(&mut config, &path, "new-token").unwrap();
        let saved = crate::load_config(&path).unwrap();
        std::fs::remove_file(&path).unwrap();

        assert_eq!(saved.google.refresh_token.as_deref(), Some("new-token"));
    }

    #[test]
    fn secret_keys_are_stable_and_account_scoped() {
        assert_eq!(
            google_client_secret_key("personal"),
            "google:personal:client-secret"
        );
        assert_eq!(
            google_refresh_token_key("personal"),
            "google:personal:refresh-token"
        );
        assert_eq!(
            icloud_app_password_key("personal"),
            "icloud:personal:app-specific-password"
        );
    }

    fn config() -> ServiceConfig {
        ServiceConfig {
            google: GoogleConfig {
                account_label: "personal".to_string(),
                client_id: Some("client-id".to_string()),
                client_secret: Some("client-secret".to_string()),
                refresh_token: Some("refresh-token".to_string()),
            },
            icloud: IcloudConfig {
                account_label: "personal".to_string(),
                username: Some("me@example.com".to_string()),
                app_specific_password: Some("app-password".to_string()),
                ..IcloudConfig::default()
            },
            ..serde_json::from_str(
                r#"{
                  "google": {},
                  "icloud": {}
                }"#,
            )
            .unwrap()
        }
    }
}
