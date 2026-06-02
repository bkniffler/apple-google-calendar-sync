use crate::{ProviderCalendar, ProviderError, google::types::*};
use chrono::{DateTime, NaiveDate, Utc};
use insync_core::{
    CanonicalEvent, EventAttendee, EventDateTime, EventReminder, EventStatus, EventVisibility,
    ProviderEventMeta, ProviderName, RecurrenceData,
};
use std::collections::BTreeMap;
use uuid::Uuid;

const PRIVATE_UID_KEY: &str = "insyncCanonicalUid";
const PRIVATE_SOURCE_KEY: &str = "insyncSource";

pub fn google_to_canonical(
    calendar_id: &str,
    event: GoogleEvent,
) -> Result<CanonicalEvent, ProviderError> {
    let canonical_uid = event
        .extended_properties
        .as_ref()
        .and_then(|properties| properties.private.as_ref())
        .and_then(|private| private.get(PRIVATE_UID_KEY))
        .cloned()
        .or_else(|| event.i_cal_uid.clone())
        .or_else(|| event.id.clone())
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    let status = normalize_status(event.status.as_ref());
    let visibility = normalize_visibility(event.visibility.as_ref());

    Ok(CanonicalEvent {
        canonical_uid,
        title: event.summary.clone().unwrap_or_default(),
        description: event.description.clone(),
        location: event.location.clone(),
        status,
        visibility,
        start: google_date_to_canonical(event.start.as_ref())?,
        end: google_date_to_canonical(event.end.as_ref())?,
        recurrence: parse_google_recurrence(&event),
        attendees: event
            .attendees
            .clone()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|attendee| {
                attendee.email.map(|email| EventAttendee {
                    email,
                    name: attendee.display_name,
                    response_status: attendee.response_status,
                    optional: attendee.optional.unwrap_or(false),
                })
            })
            .collect(),
        reminders: event
            .reminders
            .clone()
            .and_then(|reminders| reminders.overrides)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|reminder| {
                reminder.minutes.map(|minutes| EventReminder {
                    method: reminder.method.unwrap_or_else(|| "popup".to_string()),
                    minutes_before_start: minutes,
                })
            })
            .collect(),
        provider_meta: google_meta_from_response(calendar_id, &event),
        raw: serde_json::to_value(event)
            .map_err(|error| ProviderError::Mapping(error.to_string()))?,
    })
}

pub fn canonical_to_google(event: &CanonicalEvent, source: ProviderName) -> GoogleEvent {
    let mut private = BTreeMap::new();
    private.insert(PRIVATE_UID_KEY.to_string(), event.canonical_uid.clone());
    private.insert(
        PRIVATE_SOURCE_KEY.to_string(),
        provider_name(source).to_string(),
    );

    GoogleEvent {
        summary: Some(event.title.clone()),
        description: event.description.clone(),
        location: event.location.clone(),
        status: Some(status_to_google(event.status)),
        visibility: Some(visibility_to_google(event.visibility)),
        start: Some(canonical_date_to_google(&event.start)),
        end: Some(canonical_date_to_google(&event.end)),
        recurrence: google_recurrence_lines(event),
        attendees: Some(
            event
                .attendees
                .iter()
                .filter(|attendee| is_email_like(&attendee.email))
                .map(|attendee| GoogleAttendee {
                    email: Some(attendee.email.clone()),
                    display_name: attendee.name.clone(),
                    optional: Some(attendee.optional),
                    response_status: attendee.response_status.clone(),
                })
                .collect(),
        ),
        reminders: Some(GoogleReminders {
            use_default: Some(event.reminders.is_empty()),
            overrides: (!event.reminders.is_empty()).then(|| {
                event
                    .reminders
                    .iter()
                    .map(|reminder| GoogleReminder {
                        method: Some(if reminder.method == "email" {
                            "email".to_string()
                        } else {
                            "popup".to_string()
                        }),
                        minutes: Some(reminder.minutes_before_start),
                    })
                    .collect()
            }),
        }),
        extended_properties: Some(GoogleExtendedProperties {
            private: Some(private),
            shared: None,
        }),
        ..GoogleEvent::default()
    }
}

pub fn google_meta_from_response(calendar_id: &str, event: &GoogleEvent) -> ProviderEventMeta {
    ProviderEventMeta {
        provider: ProviderName::Google,
        calendar_id: calendar_id.to_string(),
        event_id: event.id.clone(),
        href: None,
        etag: event.etag.clone(),
        ical_uid: event.i_cal_uid.clone(),
        updated_at: event
            .updated
            .as_deref()
            .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
            .map(|value| value.with_timezone(&Utc)),
        deleted: event.status == Some(GoogleEventStatus::Cancelled),
    }
}

pub fn google_calendar_to_provider(calendar: GoogleCalendarListEntry) -> ProviderCalendar {
    let writable = matches!(
        calendar.access_role.as_deref(),
        Some("owner" | "writer") | None
    );

    ProviderCalendar {
        id: calendar.id.clone(),
        name: calendar
            .summary
            .clone()
            .unwrap_or_else(|| calendar.id.clone()),
        timezone: calendar.time_zone.clone(),
        writable,
        raw: serde_json::to_value(calendar).unwrap_or_else(|_| serde_json::json!({})),
    }
}

fn normalize_status(status: Option<&GoogleEventStatus>) -> EventStatus {
    match status {
        Some(GoogleEventStatus::Tentative) => EventStatus::Tentative,
        Some(GoogleEventStatus::Cancelled) => EventStatus::Cancelled,
        _ => EventStatus::Confirmed,
    }
}

fn normalize_visibility(visibility: Option<&GoogleVisibility>) -> EventVisibility {
    match visibility {
        Some(GoogleVisibility::Public) => EventVisibility::Public,
        Some(GoogleVisibility::Private) => EventVisibility::Private,
        Some(GoogleVisibility::Confidential) => EventVisibility::Confidential,
        _ => EventVisibility::Default,
    }
}

fn google_date_to_canonical(
    value: Option<&GoogleEventDateTime>,
) -> Result<EventDateTime, ProviderError> {
    match value {
        Some(GoogleEventDateTime::Date { date, .. }) => {
            let value = NaiveDate::parse_from_str(date, "%Y-%m-%d")
                .map_err(|error| ProviderError::Mapping(error.to_string()))?;
            Ok(EventDateTime::Date { value })
        }
        Some(GoogleEventDateTime::DateTime {
            date_time,
            time_zone,
        }) => {
            let value = DateTime::parse_from_rfc3339(date_time)
                .map_err(|error| ProviderError::Mapping(error.to_string()))?
                .with_timezone(&Utc);
            Ok(EventDateTime::DateTime {
                value,
                timezone: time_zone.clone(),
            })
        }
        None => Ok(EventDateTime::DateTime {
            value: DateTime::<Utc>::from_timestamp(0, 0).unwrap(),
            timezone: Some("UTC".to_string()),
        }),
    }
}

fn canonical_date_to_google(value: &EventDateTime) -> GoogleEventDateTime {
    match value {
        EventDateTime::Date { value } => GoogleEventDateTime::Date {
            date: value.format("%Y-%m-%d").to_string(),
            time_zone: None,
        },
        EventDateTime::DateTime { value, timezone } => GoogleEventDateTime::DateTime {
            date_time: value.to_rfc3339(),
            time_zone: timezone
                .as_ref()
                .filter(|timezone| is_google_timezone(timezone))
                .cloned(),
        },
    }
}

fn parse_google_recurrence(event: &GoogleEvent) -> Option<RecurrenceData> {
    let recurrence = event.recurrence.clone().unwrap_or_default();
    let rrule = recurrence
        .iter()
        .find_map(|line| line.strip_prefix("RRULE:").map(str::to_string));
    let exdates = recurrence
        .iter()
        .filter_map(|line| {
            line.starts_with("EXDATE")
                .then(|| line.split_once(':').map(|(_, value)| value.to_string()))
                .flatten()
        })
        .collect::<Vec<_>>();

    if rrule.is_none()
        && exdates.is_empty()
        && event.original_start_time.is_none()
        && event.sequence.is_none()
    {
        return None;
    }

    Some(RecurrenceData {
        rrule,
        exdates,
        recurrence_id: event.original_start_time.as_ref().map(|value| match value {
            GoogleEventDateTime::Date { date, .. } => date.clone(),
            GoogleEventDateTime::DateTime { date_time, .. } => date_time.clone(),
        }),
        sequence: event.sequence,
    })
}

fn google_recurrence_lines(event: &CanonicalEvent) -> Option<Vec<String>> {
    let mut lines = Vec::new();

    if let Some(rrule) = event
        .recurrence
        .as_ref()
        .and_then(|recurrence| recurrence.rrule.as_ref())
    {
        lines.push(format!("RRULE:{rrule}"));
    }

    for exdate in event
        .recurrence
        .as_ref()
        .map(|recurrence| recurrence.exdates.as_slice())
        .unwrap_or_default()
    {
        if let Some(line) = google_exdate_line(event, exdate) {
            lines.push(line);
        }
    }

    (!lines.is_empty()).then_some(lines)
}

fn google_exdate_line(event: &CanonicalEvent, value: &str) -> Option<String> {
    if value.len() == 8 && value.chars().all(|item| item.is_ascii_digit()) {
        return Some(format!("EXDATE;VALUE=DATE:{value}"));
    }

    if let Ok(date) = NaiveDate::parse_from_str(value, "%Y-%m-%d") {
        return Some(format!("EXDATE;VALUE=DATE:{}", date.format("%Y%m%d")));
    }

    if let Ok(datetime) = DateTime::parse_from_rfc3339(value) {
        return Some(format!(
            "EXDATE:{}",
            datetime.with_timezone(&Utc).format("%Y%m%dT%H%M%SZ")
        ));
    }

    let local = value.split_once('T');
    if let Some((date, time)) = local
        && date.len() == 10
        && (time.len() == 5 || time.len() == 8)
    {
        let compact = format!(
            "{}T{}{}",
            date.replace('-', ""),
            time.replace(':', ""),
            if time.len() == 5 { "00" } else { "" }
        );
        let timezone = match &event.start {
            EventDateTime::DateTime { timezone, .. } => timezone.as_ref(),
            EventDateTime::Date { .. } => None,
        };
        return Some(
            timezone
                .filter(|timezone| is_google_timezone(timezone))
                .map(|timezone| format!("EXDATE;TZID={timezone}:{compact}"))
                .unwrap_or_else(|| format!("EXDATE:{compact}")),
        );
    }

    None
}

fn status_to_google(status: EventStatus) -> GoogleEventStatus {
    match status {
        EventStatus::Tentative => GoogleEventStatus::Tentative,
        EventStatus::Cancelled => GoogleEventStatus::Cancelled,
        EventStatus::Confirmed => GoogleEventStatus::Confirmed,
    }
}

fn visibility_to_google(visibility: EventVisibility) -> GoogleVisibility {
    match visibility {
        EventVisibility::Public => GoogleVisibility::Public,
        EventVisibility::Private => GoogleVisibility::Private,
        EventVisibility::Confidential => GoogleVisibility::Confidential,
        EventVisibility::Default => GoogleVisibility::Default,
    }
}

fn provider_name(provider: ProviderName) -> &'static str {
    match provider {
        ProviderName::Google => "google",
        ProviderName::Icloud => "icloud",
    }
}

fn is_email_like(value: &str) -> bool {
    let Some((left, right)) = value.split_once('@') else {
        return false;
    };
    !left.is_empty()
        && !right.is_empty()
        && right.contains('.')
        && !value.contains(char::is_whitespace)
        && !value.contains('<')
        && !value.contains('>')
}

fn is_google_timezone(value: &str) -> bool {
    value == "UTC" || value.contains('/')
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    #[test]
    fn normalizes_timed_google_event() {
        let event = GoogleEvent {
            id: Some("abc123".to_string()),
            etag: Some("\"etag\"".to_string()),
            i_cal_uid: Some("uid@example.com".to_string()),
            status: Some(GoogleEventStatus::Confirmed),
            summary: Some("Planning".to_string()),
            start: Some(GoogleEventDateTime::DateTime {
                date_time: "2026-06-01T09:00:00-04:00".to_string(),
                time_zone: Some("America/New_York".to_string()),
            }),
            end: Some(GoogleEventDateTime::DateTime {
                date_time: "2026-06-01T10:00:00-04:00".to_string(),
                time_zone: Some("America/New_York".to_string()),
            }),
            reminders: Some(GoogleReminders {
                use_default: Some(false),
                overrides: Some(vec![GoogleReminder {
                    method: Some("popup".to_string()),
                    minutes: Some(15),
                }]),
            }),
            ..GoogleEvent::default()
        };

        let canonical = google_to_canonical("primary", event).unwrap();

        assert_eq!(canonical.canonical_uid, "uid@example.com");
        assert_eq!(canonical.title, "Planning");
        assert_eq!(
            canonical.start,
            EventDateTime::DateTime {
                value: Utc.with_ymd_and_hms(2026, 6, 1, 13, 0, 0).unwrap(),
                timezone: Some("America/New_York".to_string())
            }
        );
        assert_eq!(canonical.reminders[0].method, "popup");
        assert_eq!(canonical.reminders[0].minutes_before_start, 15);
    }

    #[test]
    fn uses_private_uid_before_ical_uid() {
        let mut private = BTreeMap::new();
        private.insert(PRIVATE_UID_KEY.to_string(), "private-uid".to_string());

        let canonical = google_to_canonical(
            "primary",
            GoogleEvent {
                id: Some("id".to_string()),
                i_cal_uid: Some("ical-uid".to_string()),
                extended_properties: Some(GoogleExtendedProperties {
                    private: Some(private),
                    shared: None,
                }),
                ..GoogleEvent::default()
            },
        )
        .unwrap();

        assert_eq!(canonical.canonical_uid, "private-uid");
    }

    #[test]
    fn maps_all_day_events_and_recurrence() {
        let canonical = google_to_canonical(
            "primary",
            GoogleEvent {
                id: Some("abc123".to_string()),
                start: Some(GoogleEventDateTime::Date {
                    date: "2026-06-01".to_string(),
                    time_zone: None,
                }),
                end: Some(GoogleEventDateTime::Date {
                    date: "2026-06-02".to_string(),
                    time_zone: None,
                }),
                recurrence: Some(vec![
                    "RRULE:FREQ=WEEKLY;COUNT=2".to_string(),
                    "EXDATE;VALUE=DATE:20260608".to_string(),
                ]),
                sequence: Some(3),
                ..GoogleEvent::default()
            },
        )
        .unwrap();

        assert_eq!(
            canonical.start,
            EventDateTime::Date {
                value: NaiveDate::from_ymd_opt(2026, 6, 1).unwrap()
            }
        );
        let recurrence = canonical.recurrence.unwrap();
        assert_eq!(recurrence.rrule.as_deref(), Some("FREQ=WEEKLY;COUNT=2"));
        assert_eq!(recurrence.exdates, vec!["20260608"]);
        assert_eq!(recurrence.sequence, Some(3));
    }

    #[test]
    fn canonical_to_google_filters_invalid_attendees_and_sets_private_uid() {
        let event = CanonicalEvent {
            canonical_uid: "uid-1".to_string(),
            title: "Planning".to_string(),
            description: Some("Notes".to_string()),
            location: None,
            status: EventStatus::Confirmed,
            visibility: EventVisibility::Default,
            start: EventDateTime::DateTime {
                value: Utc.with_ymd_and_hms(2026, 6, 1, 13, 0, 0).unwrap(),
                timezone: Some("America/New_York".to_string()),
            },
            end: EventDateTime::DateTime {
                value: Utc.with_ymd_and_hms(2026, 6, 1, 14, 0, 0).unwrap(),
                timezone: Some("America/New_York".to_string()),
            },
            recurrence: Some(RecurrenceData {
                rrule: Some("FREQ=DAILY;COUNT=2".to_string()),
                exdates: vec!["2026-06-02".to_string()],
                recurrence_id: None,
                sequence: None,
            }),
            attendees: vec![
                EventAttendee {
                    email: "person@example.com".to_string(),
                    name: Some("Person".to_string()),
                    response_status: Some("accepted".to_string()),
                    optional: false,
                },
                EventAttendee {
                    email: "/apple-principal/".to_string(),
                    name: None,
                    response_status: None,
                    optional: false,
                },
            ],
            reminders: vec![EventReminder {
                method: "display".to_string(),
                minutes_before_start: 15,
            }],
            provider_meta: ProviderEventMeta {
                provider: ProviderName::Icloud,
                calendar_id: "icloud-cal".to_string(),
                event_id: None,
                href: None,
                etag: None,
                ical_uid: None,
                updated_at: None,
                deleted: false,
            },
            raw: serde_json::json!({}),
        };

        let google = canonical_to_google(&event, ProviderName::Icloud);

        assert_eq!(google.summary.as_deref(), Some("Planning"));
        assert_eq!(google.attendees.unwrap().len(), 1);
        assert_eq!(
            google
                .extended_properties
                .unwrap()
                .private
                .unwrap()
                .get(PRIVATE_UID_KEY)
                .map(String::as_str),
            Some("uid-1")
        );
        assert_eq!(
            google.recurrence.unwrap(),
            vec![
                "RRULE:FREQ=DAILY;COUNT=2".to_string(),
                "EXDATE;VALUE=DATE:20260602".to_string()
            ]
        );
    }

    #[test]
    fn google_calendar_entry_maps_to_provider_calendar() {
        let calendar = google_calendar_to_provider(GoogleCalendarListEntry {
            id: "primary".to_string(),
            summary: Some("Primary".to_string()),
            time_zone: Some("UTC".to_string()),
            access_role: Some("owner".to_string()),
            ..GoogleCalendarListEntry::default()
        });

        assert_eq!(calendar.id, "primary");
        assert_eq!(calendar.name, "Primary");
        assert_eq!(calendar.timezone.as_deref(), Some("UTC"));
        assert!(calendar.writable);
    }
}
