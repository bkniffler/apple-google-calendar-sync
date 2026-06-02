pub mod credentials;

use directories::ProjectDirs;
use insync_core::{
    ConflictPolicies, ConflictPolicy, DeleteConflictPolicy, SyncDirection, UidCollisionPolicy,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    env,
    fmt::{self, Display},
    fs,
    path::{Path, PathBuf},
};
use thiserror::Error;

pub const INSYNC_CONFIG_ENV: &str = "INSYNC_CONFIG";
pub const LOCAL_CONFIG_FILE: &str = "insync.local.json";
pub const APP_CONFIG_FILE: &str = "insync.json";

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("config file not found: {0}")]
    Missing(PathBuf),
    #[error("failed to read config {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to parse config {path}: {source}")]
    Parse {
        path: PathBuf,
        source: serde_json::Error,
    },
    #[error("failed to serialize config: {0}")]
    Serialize(#[from] serde_json::Error),
    #[error("failed to write config {path}: {source}")]
    Write {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("secret store error: {0}")]
    SecretStore(#[from] keyring::Error),
    #[error("could not resolve an app config directory")]
    AppConfigDir,
    #[error("{0}")]
    Validation(#[from] ConfigValidationError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigValidationError {
    errors: Vec<String>,
}

impl ConfigValidationError {
    pub fn errors(&self) -> &[String] {
        &self.errors
    }
}

impl Display for ConfigValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "config validation failed")?;
        for error in &self.errors {
            write!(f, "\n- {error}")?;
        }
        Ok(())
    }
}

impl std::error::Error for ConfigValidationError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretStoreKind {
    None,
    Os,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceConfig {
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default)]
    pub secret_store: SecretStoreKind,
    #[serde(default = "default_db_path")]
    pub db_path: PathBuf,
    #[serde(default = "default_log_level")]
    pub log_level: String,
    pub google: GoogleConfig,
    pub icloud: IcloudConfig,
    #[serde(default)]
    pub sync: SyncConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoogleConfig {
    #[serde(default = "default_account_label")]
    pub account_label: String,
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
    pub refresh_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IcloudConfig {
    #[serde(default = "default_account_label")]
    pub account_label: String,
    pub username: Option<String>,
    pub app_specific_password: Option<String>,
    #[serde(default = "default_caldav_url")]
    pub caldav_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncConfig {
    #[serde(default = "default_poll_interval_seconds")]
    pub poll_interval_seconds: u64,
    #[serde(default)]
    pub conflict_policy: ConflictPolicy,
    #[serde(default)]
    pub conflicts: ConflictConfig,
    #[serde(default)]
    pub pairs: Vec<SyncPairConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConflictConfig {
    #[serde(default)]
    pub default: ConflictPolicy,
    #[serde(default)]
    pub both_sides_changed: ConflictPolicy,
    #[serde(default)]
    pub unlinked_same_uid: ConflictPolicy,
    #[serde(default = "default_delete_vs_update")]
    pub delete_vs_update: DeleteConflictPolicy,
    #[serde(default = "default_icloud_uid_collision")]
    pub icloud_uid_collision: UidCollisionPolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncPairConfig {
    pub id: String,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub direction: SyncDirection,
    pub google_calendar_id: String,
    pub icloud_calendar_id: String,
}

impl Default for SecretStoreKind {
    fn default() -> Self {
        Self::None
    }
}

impl Default for GoogleConfig {
    fn default() -> Self {
        Self {
            account_label: default_account_label(),
            client_id: None,
            client_secret: None,
            refresh_token: None,
        }
    }
}

impl Default for IcloudConfig {
    fn default() -> Self {
        Self {
            account_label: default_account_label(),
            username: None,
            app_specific_password: None,
            caldav_url: default_caldav_url(),
        }
    }
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            poll_interval_seconds: default_poll_interval_seconds(),
            conflict_policy: ConflictPolicy::Manual,
            conflicts: ConflictConfig::default(),
            pairs: Vec::new(),
        }
    }
}

impl Default for ServiceConfig {
    fn default() -> Self {
        Self {
            version: default_version(),
            secret_store: SecretStoreKind::None,
            db_path: default_db_path(),
            log_level: default_log_level(),
            google: GoogleConfig::default(),
            icloud: IcloudConfig::default(),
            sync: SyncConfig::default(),
        }
    }
}

impl Default for ConflictConfig {
    fn default() -> Self {
        Self {
            default: ConflictPolicy::Manual,
            both_sides_changed: ConflictPolicy::Manual,
            unlinked_same_uid: ConflictPolicy::Manual,
            delete_vs_update: DeleteConflictPolicy::UpdateWins,
            icloud_uid_collision: UidCollisionPolicy::IgnoreKnown,
        }
    }
}

impl From<ConflictConfig> for ConflictPolicies {
    fn from(value: ConflictConfig) -> Self {
        Self {
            default: value.default,
            both_sides_changed: value.both_sides_changed,
            unlinked_same_uid: value.unlinked_same_uid,
            delete_vs_update: value.delete_vs_update,
            icloud_uid_collision: value.icloud_uid_collision,
        }
    }
}

pub fn load_config(path: impl AsRef<Path>) -> Result<ServiceConfig, ConfigError> {
    let path = path.as_ref();
    if !path.exists() {
        return Err(ConfigError::Missing(path.to_path_buf()));
    }

    let body = fs::read_to_string(path).map_err(|source| ConfigError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    serde_json::from_str(&body).map_err(|source| ConfigError::Parse {
        path: path.to_path_buf(),
        source,
    })
}

pub fn save_config(path: impl AsRef<Path>, config: &ServiceConfig) -> Result<(), ConfigError> {
    let path = path.as_ref();
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(|source| ConfigError::Write {
            path: parent.to_path_buf(),
            source,
        })?;
    }

    let body = serde_json::to_string_pretty(config)?;
    fs::write(path, format!("{body}\n")).map_err(|source| ConfigError::Write {
        path: path.to_path_buf(),
        source,
    })
}

pub fn validate_config(config: &ServiceConfig) -> Result<(), ConfigValidationError> {
    let mut errors = Vec::new();

    if config.version != default_version() {
        errors.push(format!(
            "version must be {}; got {}",
            default_version(),
            config.version
        ));
    }

    if config.db_path.as_os_str().is_empty() {
        errors.push("dbPath must not be empty".to_string());
    }

    if config.log_level.trim().is_empty() {
        errors.push("logLevel must not be empty".to_string());
    }

    if config.google.account_label.trim().is_empty() {
        errors.push("google.accountLabel must not be empty".to_string());
    }

    if config.icloud.account_label.trim().is_empty() {
        errors.push("icloud.accountLabel must not be empty".to_string());
    }

    let caldav_url = config.icloud.caldav_url.trim();
    if caldav_url.is_empty() {
        errors.push("icloud.caldavUrl must not be empty".to_string());
    } else if !caldav_url.starts_with("https://") && !caldav_url.starts_with("http://") {
        errors.push("icloud.caldavUrl must start with https:// or http://".to_string());
    }

    if config.sync.poll_interval_seconds == 0 {
        errors.push("sync.pollIntervalSeconds must be greater than 0".to_string());
    }

    let mut pair_ids = BTreeSet::new();
    for (index, pair) in config.sync.pairs.iter().enumerate() {
        let prefix = format!("sync.pairs[{index}]");
        if pair.id.trim().is_empty() {
            errors.push(format!("{prefix}.id must not be empty"));
        } else if !pair_ids.insert(pair.id.as_str()) {
            errors.push(format!(
                "{prefix}.id duplicates an earlier pair id: {}",
                pair.id
            ));
        }

        if pair.google_calendar_id.trim().is_empty() {
            errors.push(format!("{prefix}.googleCalendarId must not be empty"));
        }

        if pair.icloud_calendar_id.trim().is_empty() {
            errors.push(format!("{prefix}.icloudCalendarId must not be empty"));
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(ConfigValidationError { errors })
    }
}

pub fn resolve_config_path(explicit: Option<PathBuf>) -> Result<PathBuf, ConfigError> {
    let env_path = env::var_os(INSYNC_CONFIG_ENV).map(PathBuf::from);
    resolve_config_path_from(explicit, env_path)
}

pub fn default_config_path() -> Result<PathBuf, ConfigError> {
    let local = PathBuf::from(LOCAL_CONFIG_FILE);
    if local.exists() {
        return Ok(local);
    }

    app_config_path()
}

pub fn config_search_paths() -> Result<Vec<PathBuf>, ConfigError> {
    Ok(vec![PathBuf::from(LOCAL_CONFIG_FILE), app_config_path()?])
}

pub fn app_config_path() -> Result<PathBuf, ConfigError> {
    ProjectDirs::from("dev", "bkniffler", "insync")
        .map(|dirs| dirs.config_dir().join(APP_CONFIG_FILE))
        .ok_or(ConfigError::AppConfigDir)
}

fn resolve_config_path_from(
    explicit: Option<PathBuf>,
    env_path: Option<PathBuf>,
) -> Result<PathBuf, ConfigError> {
    if let Some(path) = explicit {
        return Ok(path);
    }

    if let Some(path) = env_path {
        return Ok(path);
    }

    default_config_path()
}

fn default_version() -> u32 {
    1
}

fn default_db_path() -> PathBuf {
    PathBuf::from(".insync/insync.db")
}

fn default_log_level() -> String {
    "info".to_string()
}

fn default_account_label() -> String {
    "personal".to_string()
}

fn default_caldav_url() -> String {
    "https://caldav.icloud.com".to_string()
}

fn default_poll_interval_seconds() -> u64 {
    300
}

fn default_enabled() -> bool {
    true
}

fn default_delete_vs_update() -> DeleteConflictPolicy {
    DeleteConflictPolicy::UpdateWins
}

fn default_icloud_uid_collision() -> UidCollisionPolicy {
    UidCollisionPolicy::IgnoreKnown
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_current_json_shape() {
        let config: ServiceConfig = serde_json::from_str(
            r#"{
              "secretStore": "none",
              "google": { "accountLabel": "personal" },
              "icloud": { "accountLabel": "personal" },
              "sync": {
                "pairs": [{
                  "id": "personal",
                  "googleCalendarId": "primary",
                  "icloudCalendarId": "https://caldav.icloud.com/cal"
                }]
              }
            }"#,
        )
        .unwrap();

        assert_eq!(config.secret_store, SecretStoreKind::None);
        assert_eq!(config.sync.pairs[0].direction, SyncDirection::TwoWay);
    }

    #[test]
    fn writes_pretty_config_json() {
        let path = std::env::temp_dir().join(format!(
            "insync-config-{}-{}.json",
            std::process::id(),
            "write"
        ));
        let config: ServiceConfig = serde_json::from_str(
            r#"{
              "google": { "accountLabel": "personal" },
              "icloud": { "accountLabel": "personal" }
            }"#,
        )
        .unwrap();

        save_config(&path, &config).unwrap();
        let roundtrip = load_config(&path).unwrap();
        std::fs::remove_file(&path).unwrap();

        assert_eq!(roundtrip.version, 1);
        assert_eq!(roundtrip.google.account_label, "personal");
    }

    #[test]
    fn validates_minimal_non_secret_config() {
        let config: ServiceConfig = serde_json::from_str(
            r#"{
              "google": { "accountLabel": "personal" },
              "icloud": { "accountLabel": "personal" },
              "sync": { "pairs": [] }
            }"#,
        )
        .unwrap();

        validate_config(&config).unwrap();
    }

    #[test]
    fn default_config_is_valid_and_uses_stable_defaults() {
        let config = ServiceConfig::default();

        validate_config(&config).unwrap();
        assert_eq!(config.secret_store, SecretStoreKind::None);
        assert_eq!(config.google.account_label, "personal");
        assert_eq!(config.icloud.caldav_url, "https://caldav.icloud.com");
        assert_eq!(config.sync.poll_interval_seconds, 300);
        assert!(config.sync.pairs.is_empty());
    }

    #[test]
    fn validates_non_secret_shape_with_clear_paths() {
        let config: ServiceConfig = serde_json::from_str(
            r#"{
              "version": 2,
              "dbPath": "",
              "logLevel": " ",
              "google": { "accountLabel": "" },
              "icloud": {
                "accountLabel": "",
                "caldavUrl": "caldav.icloud.com"
              },
              "sync": {
                "pollIntervalSeconds": 0,
                "pairs": [
                  {
                    "id": "personal",
                    "googleCalendarId": "",
                    "icloudCalendarId": ""
                  },
                  {
                    "id": "personal",
                    "googleCalendarId": "primary",
                    "icloudCalendarId": "https://caldav.icloud.com/cal"
                  }
                ]
              }
            }"#,
        )
        .unwrap();

        let error = validate_config(&config).unwrap_err();

        assert!(
            error
                .errors()
                .contains(&"version must be 1; got 2".to_string())
        );
        assert!(
            error
                .errors()
                .contains(&"sync.pairs[0].googleCalendarId must not be empty".to_string())
        );
        assert!(
            error
                .errors()
                .contains(&"sync.pairs[1].id duplicates an earlier pair id: personal".to_string())
        );
        assert!(
            error
                .to_string()
                .starts_with("config validation failed\n- ")
        );
    }

    #[test]
    fn resolves_explicit_config_before_env_config() {
        let path = resolve_config_path_from(
            Some(PathBuf::from("/explicit/insync.json")),
            Some(PathBuf::from("/env/insync.json")),
        )
        .unwrap();

        assert_eq!(path, PathBuf::from("/explicit/insync.json"));
    }

    #[test]
    fn resolves_env_config_before_defaults() {
        let path = resolve_config_path_from(None, Some(PathBuf::from("/env/insync.json"))).unwrap();

        assert_eq!(path, PathBuf::from("/env/insync.json"));
    }

    #[test]
    fn exposes_config_search_paths_in_default_order() {
        let paths = config_search_paths().unwrap();

        assert_eq!(paths.first().unwrap(), &PathBuf::from(LOCAL_CONFIG_FILE));
        assert_eq!(paths.last().unwrap().file_name().unwrap(), APP_CONFIG_FILE);
    }
}
