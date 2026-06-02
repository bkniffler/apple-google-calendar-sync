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
    #[error("{0} sync token expired")]
    SyncTokenExpired(ProviderName),
    #[error("{0} precondition failed")]
    PreconditionFailed(ProviderName),
    #[error("{provider} request failed with {status}: {body}")]
    Http {
        provider: ProviderName,
        status: u16,
        body: String,
    },
    #[error("provider request failed: {0}")]
    Request(String),
    #[error("mapping failed: {0}")]
    Mapping(String),
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
