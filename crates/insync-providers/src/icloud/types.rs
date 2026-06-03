use crate::ProviderCalendar;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct IcloudCalendar {
    pub url: String,
    pub display_name: Option<String>,
    pub timezone: Option<String>,
    pub sync_token: Option<String>,
}

pub fn icloud_calendar_to_provider(calendar: IcloudCalendar) -> ProviderCalendar {
    ProviderCalendar {
        id: calendar.url.clone(),
        name: calendar
            .display_name
            .clone()
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| calendar.url.clone()),
        timezone: calendar.timezone.clone(),
        writable: true,
        raw: serde_json::to_value(calendar).unwrap_or_else(|_| serde_json::json!({})),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icloud_calendar_maps_to_provider_calendar() {
        let calendar = icloud_calendar_to_provider(IcloudCalendar {
            url: "https://caldav.example/cal".to_string(),
            display_name: Some("Personal".to_string()),
            timezone: Some("Europe/Berlin".to_string()),
            sync_token: Some("sync-token".to_string()),
        });

        assert_eq!(calendar.id, "https://caldav.example/cal");
        assert_eq!(calendar.name, "Personal");
        assert_eq!(calendar.timezone.as_deref(), Some("Europe/Berlin"));
        assert!(calendar.writable);
    }
}
