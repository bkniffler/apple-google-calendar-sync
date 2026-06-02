pub mod google;
pub mod icloud;

use async_trait::async_trait;
use insync_core::{CanonicalEvent, ProviderEventMeta, ProviderName};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProviderError {
    #[error("provider is not configured: {0}")]
    NotConfigured(ProviderName),
    #[error("{provider} authentication failed with {status}: {body}")]
    Auth {
        provider: ProviderName,
        status: u16,
        body: String,
    },
    #[error("{0} sync token expired")]
    SyncTokenExpired(ProviderName),
    #[error("{0} precondition failed")]
    PreconditionFailed(ProviderName),
    #[error("{provider} rate limit failed with {status}: {body}")]
    RateLimited {
        provider: ProviderName,
        status: u16,
        body: String,
    },
    #[error("{provider} UID collision for {uid} in {calendar_id}")]
    UidCollision {
        provider: ProviderName,
        calendar_id: String,
        uid: String,
    },
    #[error("{provider} request failed with {status}: {body}")]
    Http {
        provider: ProviderName,
        status: u16,
        body: String,
    },
    #[error("{provider} network request failed: {message}")]
    Network {
        provider: ProviderName,
        message: String,
    },
    #[error("provider request failed: {0}")]
    Request(String),
    #[error("mapping failed: {0}")]
    Mapping(String),
}

impl ProviderError {
    pub fn http(provider: ProviderName, status: u16, body: impl Into<String>) -> Self {
        let body = body.into();
        if status == 401 || is_auth_response(provider, status, &body) {
            Self::Auth {
                provider,
                status,
                body,
            }
        } else if is_rate_limit_response(provider, status, &body) {
            Self::RateLimited {
                provider,
                status,
                body,
            }
        } else {
            Self::Http {
                provider,
                status,
                body,
            }
        }
    }

    pub fn network(provider: ProviderName, error: impl std::fmt::Display) -> Self {
        Self::Network {
            provider,
            message: error.to_string(),
        }
    }

    pub fn is_rate_limited(&self) -> bool {
        matches!(self, Self::RateLimited { .. })
    }
}

fn is_auth_response(provider: ProviderName, status: u16, body: &str) -> bool {
    let body = body.to_ascii_lowercase();
    match provider {
        ProviderName::Google => {
            (status == 400
                && (body.contains("invalid_grant")
                    || body.contains("invalid_client")
                    || body.contains("unauthorized_client")))
                || (status == 403
                    && (body.contains("autherror")
                        || body.contains("insufficientpermissions")
                        || body.contains("insufficient permissions")))
        }
        ProviderName::Icloud => {
            status == 403
                && (body.contains("forbidden")
                    || body.contains("unauthorized")
                    || body.contains("authentication"))
        }
    }
}

fn is_rate_limit_response(provider: ProviderName, status: u16, body: &str) -> bool {
    let body = body.to_ascii_lowercase();
    match provider {
        ProviderName::Google => {
            status == 429
                || (status == 403
                    && (body.contains("ratelimitexceeded")
                        || body.contains("userratelimitexceeded")))
        }
        ProviderName::Icloud => status == 429 || status == 503,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderCalendar {
    pub id: String,
    pub name: String,
    pub timezone: Option<String>,
    pub writable: bool,
    pub raw: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderSyncCursor {
    pub sync_token: Option<String>,
    pub full_sync: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderChangeSet {
    pub provider: ProviderName,
    pub calendar_id: String,
    pub sync_token: Option<String>,
    pub events: Vec<CanonicalEvent>,
}

#[async_trait]
pub trait CalendarProvider: Send + Sync {
    fn name(&self) -> ProviderName;

    async fn list_calendars(&self) -> Result<Vec<ProviderCalendar>, ProviderError>;

    async fn get_changes(
        &self,
        calendar_id: &str,
        cursor: ProviderSyncCursor,
    ) -> Result<ProviderChangeSet, ProviderError>;

    async fn create_event(
        &self,
        calendar_id: &str,
        event: &CanonicalEvent,
    ) -> Result<ProviderEventMeta, ProviderError>;

    async fn update_event(
        &self,
        calendar_id: &str,
        remote_event_id: &str,
        event: &CanonicalEvent,
        etag: Option<&str>,
    ) -> Result<ProviderEventMeta, ProviderError>;

    async fn delete_event(
        &self,
        calendar_id: &str,
        remote_event_id: &str,
        etag: Option<&str>,
    ) -> Result<(), ProviderError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_google_auth_failures() {
        let error = ProviderError::http(ProviderName::Google, 400, r#"{"error":"invalid_grant"}"#);

        assert!(matches!(
            error,
            ProviderError::Auth {
                provider: ProviderName::Google,
                status: 400,
                ..
            }
        ));
    }

    #[test]
    fn classifies_google_rate_limits() {
        let error = ProviderError::http(
            ProviderName::Google,
            403,
            r#"{"reason":"userRateLimitExceeded"}"#,
        );

        assert!(matches!(
            error,
            ProviderError::RateLimited {
                provider: ProviderName::Google,
                status: 403,
                ..
            }
        ));
    }

    #[test]
    fn classifies_icloud_auth_and_throttling() {
        assert!(matches!(
            ProviderError::http(ProviderName::Icloud, 401, ""),
            ProviderError::Auth {
                provider: ProviderName::Icloud,
                ..
            }
        ));
        assert!(matches!(
            ProviderError::http(ProviderName::Icloud, 503, "temporarily unavailable"),
            ProviderError::RateLimited {
                provider: ProviderName::Icloud,
                status: 503,
                ..
            }
        ));
    }
}
