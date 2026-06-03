use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct GoogleCalendarListEntry {
    pub id: String,
    pub summary: Option<String>,
    pub time_zone: Option<String>,
    pub background_color: Option<String>,
    pub access_role: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct GoogleCalendarListResponse {
    pub next_page_token: Option<String>,
    pub items: Option<Vec<GoogleCalendarListEntry>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct GoogleEventsResponse {
    pub next_page_token: Option<String>,
    pub next_sync_token: Option<String>,
    pub items: Option<Vec<GoogleEvent>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct GoogleEvent {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub etag: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<GoogleEventStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub html_link: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub creator: Option<GooglePerson>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub organizer: Option<GooglePerson>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start: Option<GoogleEventDateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<GoogleEventDateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recurrence: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recurring_event_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub original_start_time: Option<GoogleEventDateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transparency: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visibility: Option<GoogleVisibility>,
    #[serde(rename = "iCalUID", skip_serializing_if = "Option::is_none")]
    pub i_cal_uid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sequence: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attendees: Option<Vec<GoogleAttendee>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reminders: Option<GoogleReminders>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extended_properties: Option<GoogleExtendedProperties>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GooglePerson {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub self_: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoogleEventStatus {
    Confirmed,
    Tentative,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoogleVisibility {
    Default,
    Public,
    Private,
    Confidential,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum GoogleEventDateTime {
    Date {
        date: String,
        #[serde(rename = "timeZone", skip_serializing_if = "Option::is_none")]
        time_zone: Option<String>,
    },
    DateTime {
        #[serde(rename = "dateTime")]
        date_time: String,
        #[serde(rename = "timeZone", skip_serializing_if = "Option::is_none")]
        time_zone: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoogleAttendee {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub optional: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_status: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoogleReminders {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub use_default: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overrides: Option<Vec<GoogleReminder>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoogleReminder {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub minutes: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoogleExtendedProperties {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub private: Option<BTreeMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shared: Option<BTreeMap<String, String>>,
}
