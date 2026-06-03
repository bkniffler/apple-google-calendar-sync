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
    pub id: Option<String>,
    pub etag: Option<String>,
    pub status: Option<GoogleEventStatus>,
    pub html_link: Option<String>,
    pub created: Option<String>,
    pub updated: Option<String>,
    pub summary: Option<String>,
    pub description: Option<String>,
    pub location: Option<String>,
    pub color_id: Option<String>,
    pub creator: Option<GooglePerson>,
    pub organizer: Option<GooglePerson>,
    pub start: Option<GoogleEventDateTime>,
    pub end: Option<GoogleEventDateTime>,
    pub recurrence: Option<Vec<String>>,
    pub recurring_event_id: Option<String>,
    pub original_start_time: Option<GoogleEventDateTime>,
    pub transparency: Option<String>,
    pub visibility: Option<GoogleVisibility>,
    #[serde(rename = "iCalUID")]
    pub i_cal_uid: Option<String>,
    pub sequence: Option<i64>,
    pub attendees: Option<Vec<GoogleAttendee>>,
    pub reminders: Option<GoogleReminders>,
    pub extended_properties: Option<GoogleExtendedProperties>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GooglePerson {
    pub email: Option<String>,
    pub display_name: Option<String>,
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
        #[serde(rename = "timeZone")]
        time_zone: Option<String>,
    },
    DateTime {
        #[serde(rename = "dateTime")]
        date_time: String,
        #[serde(rename = "timeZone")]
        time_zone: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoogleAttendee {
    pub email: Option<String>,
    pub display_name: Option<String>,
    pub optional: Option<bool>,
    pub response_status: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoogleReminders {
    pub use_default: Option<bool>,
    pub overrides: Option<Vec<GoogleReminder>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoogleReminder {
    pub method: Option<String>,
    pub minutes: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoogleExtendedProperties {
    pub private: Option<BTreeMap<String, String>>,
    pub shared: Option<BTreeMap<String, String>>,
}
