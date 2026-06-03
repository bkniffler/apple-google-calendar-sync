use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderName {
    Google,
    Icloud,
}

impl fmt::Display for ProviderName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Google => formatter.write_str("google"),
            Self::Icloud => formatter.write_str("icloud"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventStatus {
    Confirmed,
    Tentative,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventVisibility {
    Default,
    Public,
    Private,
    Confidential,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum EventDateTime {
    DateTime {
        value: DateTime<Utc>,
        timezone: Option<String>,
    },
    Date {
        value: NaiveDate,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RecurrenceData {
    pub rrule: Option<String>,
    pub exdates: Vec<String>,
    pub recurrence_id: Option<String>,
    pub sequence: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventAttendee {
    pub email: String,
    pub name: Option<String>,
    pub response_status: Option<String>,
    pub optional: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventReminder {
    pub method: String,
    pub minutes_before_start: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderEventMeta {
    pub provider: ProviderName,
    pub calendar_id: String,
    pub event_id: Option<String>,
    pub href: Option<String>,
    pub etag: Option<String>,
    #[serde(rename = "iCalUid")]
    pub ical_uid: Option<String>,
    pub updated_at: Option<DateTime<Utc>>,
    pub deleted: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CanonicalEvent {
    pub canonical_uid: String,
    pub title: String,
    pub description: Option<String>,
    pub location: Option<String>,
    pub status: EventStatus,
    pub visibility: EventVisibility,
    pub start: EventDateTime,
    pub end: EventDateTime,
    pub recurrence: Option<RecurrenceData>,
    pub attendees: Vec<EventAttendee>,
    pub reminders: Vec<EventReminder>,
    pub provider_meta: ProviderEventMeta,
    pub raw: serde_json::Value,
}
