use crate::ProviderError;
use chrono::{DateTime, NaiveDate, NaiveDateTime, TimeZone, Utc};
use ical::{
    IcalParser,
    parser::ical::component::{IcalAlarm, IcalEvent},
    property::Property,
};
use insync_core::{
    CanonicalEvent, EventAttendee, EventDateTime, EventReminder, EventStatus, EventVisibility,
    ProviderEventMeta, ProviderName, RecurrenceData,
};
use std::io::BufReader;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalendarObject {
    pub url: String,
    pub etag: Option<String>,
    pub data: Option<String>,
}

pub fn ical_object_to_canonical(
    calendar_id: &str,
    object: CalendarObject,
) -> Result<Vec<CanonicalEvent>, ProviderError> {
    let Some(data) = object.data.as_deref() else {
        return Ok(Vec::new());
    };

    let calendars = parse_calendar_object(data)?;
    let mut events = Vec::new();

    for calendar in calendars {
        for event in calendar.events {
            events.push(component_to_canonical(calendar_id, &event, &object)?);
        }
    }

    Ok(events)
}

pub fn canonical_to_ical(event: &CanonicalEvent) -> String {
    let mut lines = vec![
        "BEGIN:VCALENDAR".to_string(),
        "VERSION:2.0".to_string(),
        "PRODID:-//insync//calendar sync//EN".to_string(),
        "CALSCALE:GREGORIAN".to_string(),
        "BEGIN:VEVENT".to_string(),
        format!("UID:{}", escape_text(&event.canonical_uid)),
        format!("DTSTAMP:{}", compact_utc_now()),
        format!("SUMMARY:{}", escape_text(&event.title)),
    ];

    if let Some(description) = event.description.as_ref().filter(|value| !value.is_empty()) {
        lines.push(format!("DESCRIPTION:{}", escape_text(description)));
    }
    if let Some(location) = event.location.as_ref().filter(|value| !value.is_empty()) {
        lines.push(format!("LOCATION:{}", escape_text(location)));
    }

    lines.push(format!("STATUS:{}", status_to_ical(event.status)));
    if matches!(
        event.visibility,
        EventVisibility::Private | EventVisibility::Confidential
    ) {
        lines.push(format!("CLASS:{}", visibility_to_ical(event.visibility)));
    }

    lines.push(time_property_to_ical("DTSTART", &event.start));
    lines.push(time_property_to_ical("DTEND", &event.end));

    if let Some(sequence) = event.recurrence.as_ref().and_then(|value| value.sequence) {
        lines.push(format!("SEQUENCE:{sequence}"));
    }
    if let Some(rrule) = event
        .recurrence
        .as_ref()
        .and_then(|value| value.rrule.as_ref())
    {
        lines.push(format!("RRULE:{rrule}"));
    }
    for exdate in event
        .recurrence
        .as_ref()
        .map(|value| value.exdates.as_slice())
        .unwrap_or_default()
    {
        lines.push(format!("EXDATE:{exdate}"));
    }
    if let Some(recurrence_id) = event
        .recurrence
        .as_ref()
        .and_then(|value| value.recurrence_id.as_ref())
    {
        lines.push(format!("RECURRENCE-ID:{recurrence_id}"));
    }

    for attendee in &event.attendees {
        let mut line = "ATTENDEE".to_string();
        if let Some(name) = attendee.name.as_ref() {
            line.push_str(&format!(";CN={}", escape_param(name)));
        }
        if let Some(response_status) = attendee.response_status.as_ref() {
            line.push_str(&format!(
                ";PARTSTAT={}",
                response_status_to_ical(response_status)
            ));
        }
        if attendee.optional {
            line.push_str(";ROLE=OPT-PARTICIPANT");
        }
        line.push_str(&format!(":mailto:{}", attendee.email));
        lines.push(line);
    }

    for reminder in &event.reminders {
        lines.extend(reminder_to_alarm(reminder));
    }

    lines.push("END:VEVENT".to_string());
    lines.push("END:VCALENDAR".to_string());
    format!("{}\r\n", lines.join("\r\n"))
}

pub fn ical_meta_from_object(
    calendar_id: &str,
    object_url: &str,
    etag: Option<&str>,
) -> ProviderEventMeta {
    ProviderEventMeta {
        provider: ProviderName::Icloud,
        calendar_id: calendar_id.to_string(),
        event_id: None,
        href: Some(object_url.to_string()),
        etag: etag.map(str::to_string),
        ical_uid: None,
        updated_at: None,
        deleted: false,
    }
}

pub fn event_filename(event: &CanonicalEvent) -> String {
    let safe_uid = event
        .canonical_uid
        .chars()
        .map(|item| {
            if item.is_ascii_alphanumeric() || matches!(item, '.' | '_' | '-') {
                item
            } else {
                '_'
            }
        })
        .collect::<String>();
    format!(
        "{}.ics",
        if safe_uid.is_empty() {
            Uuid::new_v4().to_string()
        } else {
            safe_uid
        }
    )
}

fn component_to_canonical(
    calendar_id: &str,
    event: &IcalEvent,
    object: &CalendarObject,
) -> Result<CanonicalEvent, ProviderError> {
    let uid = property_value(event, "UID").unwrap_or_else(|| Uuid::new_v4().to_string());
    let status = status_value(event);
    let visibility = visibility_value(event);

    Ok(CanonicalEvent {
        canonical_uid: uid.clone(),
        title: property_value(event, "SUMMARY").unwrap_or_default(),
        description: property_value(event, "DESCRIPTION"),
        location: property_value(event, "LOCATION"),
        status,
        visibility,
        start: time_property_to_canonical(event, "DTSTART")?,
        end: time_property_to_canonical(event, "DTEND")?,
        recurrence: recurrence_value(event),
        attendees: attendees_value(event),
        reminders: reminders_value(event),
        provider_meta: ProviderEventMeta {
            provider: ProviderName::Icloud,
            calendar_id: calendar_id.to_string(),
            event_id: None,
            href: Some(object.url.clone()),
            etag: object.etag.clone(),
            ical_uid: Some(uid),
            updated_at: None,
            deleted: status == EventStatus::Cancelled,
        },
        raw: serde_json::json!({
            "href": object.url,
            "etag": object.etag,
            "ics": object.data
        }),
    })
}

fn parse_calendar_object(
    data: &str,
) -> Result<Vec<ical::parser::ical::component::IcalCalendar>, ProviderError> {
    parse_ical(data).or_else(|_| parse_ical(&repair_literal_newlines_in_text_properties(data)))
}

fn parse_ical(
    data: &str,
) -> Result<Vec<ical::parser::ical::component::IcalCalendar>, ProviderError> {
    let reader = BufReader::new(data.as_bytes());
    IcalParser::new(reader)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| ProviderError::Mapping(error.to_string()))
}

fn property_value(event: &IcalEvent, name: &str) -> Option<String> {
    event
        .properties
        .iter()
        .find(|property| property.name.eq_ignore_ascii_case(name))
        .and_then(|property| property.value.clone())
}

fn properties<'a>(event: &'a IcalEvent, name: &str) -> Vec<&'a Property> {
    event
        .properties
        .iter()
        .filter(|property| property.name.eq_ignore_ascii_case(name))
        .collect()
}

fn property_param(property: &Property, name: &str) -> Option<String> {
    property
        .params
        .as_ref()?
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .and_then(|(_, values)| values.first().cloned())
}

fn status_value(event: &IcalEvent) -> EventStatus {
    match property_value(event, "STATUS")
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "tentative" => EventStatus::Tentative,
        "cancelled" => EventStatus::Cancelled,
        _ => EventStatus::Confirmed,
    }
}

fn visibility_value(event: &IcalEvent) -> EventVisibility {
    match property_value(event, "CLASS")
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "public" => EventVisibility::Public,
        "private" => EventVisibility::Private,
        "confidential" => EventVisibility::Confidential,
        _ => EventVisibility::Default,
    }
}

fn time_property_to_canonical(
    event: &IcalEvent,
    name: &str,
) -> Result<EventDateTime, ProviderError> {
    let Some(property) = properties(event, name).into_iter().next() else {
        return Ok(EventDateTime::DateTime {
            value: DateTime::<Utc>::from_timestamp(0, 0).unwrap(),
            timezone: Some("UTC".to_string()),
        });
    };
    let Some(value) = property.value.as_deref() else {
        return Ok(EventDateTime::DateTime {
            value: DateTime::<Utc>::from_timestamp(0, 0).unwrap(),
            timezone: Some("UTC".to_string()),
        });
    };

    if property_param(property, "VALUE")
        .map(|value| value.eq_ignore_ascii_case("DATE"))
        .unwrap_or(false)
        || (value.len() == 8 && value.chars().all(|item| item.is_ascii_digit()))
    {
        let date = NaiveDate::parse_from_str(value, "%Y%m%d")
            .or_else(|_| NaiveDate::parse_from_str(value, "%Y-%m-%d"))
            .map_err(|error| ProviderError::Mapping(error.to_string()))?;
        return Ok(EventDateTime::Date { value: date });
    }

    let timezone = property_param(property, "TZID");
    let parsed = parse_ical_datetime(value)?;
    Ok(EventDateTime::DateTime {
        value: parsed,
        timezone: timezone.or_else(|| Some("UTC".to_string())),
    })
}

fn parse_ical_datetime(value: &str) -> Result<DateTime<Utc>, ProviderError> {
    if let Ok(value) = DateTime::parse_from_rfc3339(value) {
        return Ok(value.with_timezone(&Utc));
    }

    if let Some(stripped) = value.strip_suffix('Z') {
        let naive = NaiveDateTime::parse_from_str(stripped, "%Y%m%dT%H%M%S")
            .map_err(|error| ProviderError::Mapping(error.to_string()))?;
        return Ok(Utc.from_utc_datetime(&naive));
    }

    let naive = NaiveDateTime::parse_from_str(value, "%Y%m%dT%H%M%S")
        .or_else(|_| NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S"))
        .map_err(|error| ProviderError::Mapping(error.to_string()))?;
    Ok(Utc.from_utc_datetime(&naive))
}

fn recurrence_value(event: &IcalEvent) -> Option<RecurrenceData> {
    let rrule = property_value(event, "RRULE");
    let exdates = properties(event, "EXDATE")
        .into_iter()
        .filter_map(|property| property.value.clone())
        .collect::<Vec<_>>();
    let recurrence_id = property_value(event, "RECURRENCE-ID");
    let sequence = property_value(event, "SEQUENCE").and_then(|value| value.parse::<i64>().ok());

    if rrule.is_none() && exdates.is_empty() && recurrence_id.is_none() && sequence.is_none() {
        return None;
    }

    Some(RecurrenceData {
        rrule,
        exdates,
        recurrence_id,
        sequence,
    })
}

fn attendees_value(event: &IcalEvent) -> Vec<EventAttendee> {
    properties(event, "ATTENDEE")
        .into_iter()
        .filter_map(|property| {
            let email = property
                .value
                .as_deref()
                .unwrap_or_default()
                .trim_start_matches("mailto:")
                .trim_start_matches("MAILTO:")
                .to_string();
            (!email.is_empty()).then(|| EventAttendee {
                email,
                name: property_param(property, "CN"),
                response_status: normalize_partstat(
                    property_param(property, "PARTSTAT").as_deref(),
                ),
                optional: property_param(property, "ROLE")
                    .map(|value| value.eq_ignore_ascii_case("OPT-PARTICIPANT"))
                    .unwrap_or(false),
            })
        })
        .collect()
}

fn reminders_value(event: &IcalEvent) -> Vec<EventReminder> {
    event
        .alarms
        .iter()
        .filter_map(|alarm| {
            let trigger = alarm_property_value(alarm, "TRIGGER").unwrap_or_default();
            let minutes = duration_to_minutes(&trigger);
            minutes.map(|minutes| EventReminder {
                method: "display".to_string(),
                minutes_before_start: minutes,
            })
        })
        .collect()
}

fn alarm_property_value(alarm: &IcalAlarm, name: &str) -> Option<String> {
    alarm
        .properties
        .iter()
        .find(|property| property.name.eq_ignore_ascii_case(name))
        .and_then(|property| property.value.clone())
}

fn normalize_partstat(partstat: Option<&str>) -> Option<String> {
    match partstat?.to_ascii_uppercase().as_str() {
        "DECLINED" => Some("declined".to_string()),
        "TENTATIVE" => Some("tentative".to_string()),
        "ACCEPTED" => Some("accepted".to_string()),
        "NEEDS-ACTION" => Some("needsAction".to_string()),
        _ => None,
    }
}

fn duration_to_minutes(value: &str) -> Option<i64> {
    let value = value.trim_start_matches('-');
    let value = value.strip_prefix("PT")?;
    let mut hours = 0;
    let mut minutes = 0;
    let mut number = String::new();

    for item in value.chars() {
        if item.is_ascii_digit() {
            number.push(item);
            continue;
        }
        let parsed = number.parse::<i64>().unwrap_or(0);
        match item {
            'H' => hours = parsed,
            'M' => minutes = parsed,
            _ => {}
        }
        number.clear();
    }

    Some(hours * 60 + minutes)
}

fn reminder_to_alarm(reminder: &EventReminder) -> Vec<String> {
    vec![
        "BEGIN:VALARM".to_string(),
        format!(
            "ACTION:{}",
            if reminder.method == "email" {
                "EMAIL"
            } else {
                "DISPLAY"
            }
        ),
        "DESCRIPTION:Reminder".to_string(),
        format!("TRIGGER:-PT{}M", reminder.minutes_before_start),
        "END:VALARM".to_string(),
    ]
}

fn time_property_to_ical(name: &str, value: &EventDateTime) -> String {
    match value {
        EventDateTime::Date { value } => format!("{name};VALUE=DATE:{}", value.format("%Y%m%d")),
        EventDateTime::DateTime { value, timezone } => {
            if let Some(timezone) = timezone
                .as_ref()
                .filter(|timezone| timezone.as_str() != "UTC")
            {
                format!("{name};TZID={timezone}:{}", value.format("%Y%m%dT%H%M%S"))
            } else {
                format!("{name}:{}", value.format("%Y%m%dT%H%M%SZ"))
            }
        }
    }
}

fn status_to_ical(status: EventStatus) -> &'static str {
    match status {
        EventStatus::Confirmed => "CONFIRMED",
        EventStatus::Tentative => "TENTATIVE",
        EventStatus::Cancelled => "CANCELLED",
    }
}

fn visibility_to_ical(visibility: EventVisibility) -> &'static str {
    match visibility {
        EventVisibility::Private => "PRIVATE",
        EventVisibility::Confidential => "CONFIDENTIAL",
        EventVisibility::Public | EventVisibility::Default => "PUBLIC",
    }
}

fn response_status_to_ical(value: &str) -> &'static str {
    match value {
        "declined" => "DECLINED",
        "tentative" => "TENTATIVE",
        "accepted" => "ACCEPTED",
        "needsAction" => "NEEDS-ACTION",
        _ => "NEEDS-ACTION",
    }
}

fn compact_utc_now() -> String {
    Utc::now().format("%Y%m%dT%H%M%SZ").to_string()
}

fn escape_text(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace(',', "\\,")
        .replace(';', "\\;")
}

fn escape_param(value: &str) -> String {
    if value.contains([',', ';', ':', '"', '\n']) {
        format!("\"{}\"", value.replace('"', "\\\""))
    } else {
        value.to_string()
    }
}

fn repair_literal_newlines_in_text_properties(data: &str) -> String {
    let unfolded_lines = unfold_calendar_lines(data);
    let mut repaired = Vec::new();
    let mut skipping_malformed_structured_location = false;

    for line in unfolded_lines {
        if line
            .to_ascii_uppercase()
            .starts_with("X-APPLE-STRUCTURED-LOCATION")
        {
            skipping_malformed_structured_location = true;
            continue;
        }

        if skipping_malformed_structured_location {
            if !is_content_line(&line) {
                continue;
            }
            skipping_malformed_structured_location = false;
        }

        if is_content_line(&line) || repaired.is_empty() {
            repaired.push(line);
            continue;
        }

        let previous_index = repaired.len() - 1;
        if is_repairable_text_property(&repaired[previous_index]) {
            repaired[previous_index] = format!(
                "{}\\n{}",
                repaired[previous_index],
                escape_text_value(&line)
            );
            continue;
        }

        repaired.push(line);
    }

    repaired.join("\r\n")
}

fn unfold_calendar_lines(data: &str) -> Vec<String> {
    let mut unfolded: Vec<String> = Vec::new();
    for line in data.replace("\r\n", "\n").replace('\r', "\n").split('\n') {
        if (line.starts_with(' ') || line.starts_with('\t')) && !unfolded.is_empty() {
            let last = unfolded.len() - 1;
            unfolded[last].push_str(&line[1..]);
        } else {
            unfolded.push(line.to_string());
        }
    }
    unfolded
}

fn is_content_line(line: &str) -> bool {
    line.contains([';', ':'])
        && line
            .chars()
            .next()
            .map(|item| item.is_ascii_alphanumeric())
            .unwrap_or(false)
}

fn is_repairable_text_property(line: &str) -> bool {
    let upper = line.to_ascii_uppercase();
    ["SUMMARY", "DESCRIPTION", "LOCATION", "COMMENT", "CONTACT"]
        .iter()
        .any(|prefix| upper.starts_with(prefix))
}

fn escape_text_value(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace(',', "\\,")
        .replace(';', "\\;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn timed_event() -> CanonicalEvent {
        CanonicalEvent {
            canonical_uid: "uid@example.com".to_string(),
            title: "Planning".to_string(),
            description: Some("Roadmap".to_string()),
            location: Some("Office".to_string()),
            status: EventStatus::Confirmed,
            visibility: EventVisibility::Default,
            start: EventDateTime::DateTime {
                value: Utc.with_ymd_and_hms(2026, 6, 1, 13, 0, 0).unwrap(),
                timezone: Some("UTC".to_string()),
            },
            end: EventDateTime::DateTime {
                value: Utc.with_ymd_and_hms(2026, 6, 1, 14, 0, 0).unwrap(),
                timezone: Some("UTC".to_string()),
            },
            recurrence: None,
            attendees: Vec::new(),
            reminders: vec![EventReminder {
                method: "display".to_string(),
                minutes_before_start: 10,
            }],
            provider_meta: ProviderEventMeta {
                provider: ProviderName::Google,
                calendar_id: "primary".to_string(),
                event_id: None,
                href: None,
                etag: None,
                ical_uid: None,
                updated_at: None,
                deleted: false,
            },
            raw: serde_json::json!({}),
        }
    }

    #[test]
    fn round_trips_timed_event() {
        let ics = canonical_to_ical(&timed_event());
        assert!(!ics.contains("CLASS:DEFAULT"));
        let parsed = ical_object_to_canonical(
            "icloud-calendar",
            CalendarObject {
                url: "https://example.com/calendar/uid.ics".to_string(),
                etag: Some("etag".to_string()),
                data: Some(ics),
            },
        )
        .unwrap();

        assert_eq!(parsed[0].canonical_uid, "uid@example.com");
        assert_eq!(parsed[0].title, "Planning");
        assert_eq!(parsed[0].description.as_deref(), Some("Roadmap"));
        assert_eq!(parsed[0].location.as_deref(), Some("Office"));
        assert_eq!(
            parsed[0].provider_meta.href.as_deref(),
            Some("https://example.com/calendar/uid.ics")
        );
        assert_eq!(parsed[0].reminders[0].minutes_before_start, 10);
    }

    #[test]
    fn round_trips_full_ical_fixture_where_supported() {
        let mut event = timed_event();
        event.canonical_uid = "fixture@example.com".to_string();
        event.title = "Private recurring fixture".to_string();
        event.status = EventStatus::Tentative;
        event.visibility = EventVisibility::Private;
        event.recurrence = Some(RecurrenceData {
            rrule: Some("FREQ=WEEKLY;COUNT=3".to_string()),
            exdates: vec!["20260608T130000Z".to_string()],
            recurrence_id: Some("20260601T130000Z".to_string()),
            sequence: Some(4),
        });
        event.attendees = vec![EventAttendee {
            email: "person@example.com".to_string(),
            name: Some("Person".to_string()),
            response_status: Some("accepted".to_string()),
            optional: true,
        }];
        event.reminders = vec![EventReminder {
            method: "display".to_string(),
            minutes_before_start: 30,
        }];

        let ics = canonical_to_ical(&event);
        let parsed = ical_object_to_canonical(
            "icloud-calendar",
            CalendarObject {
                url: "https://example.com/calendar/fixture.ics".to_string(),
                etag: Some("etag".to_string()),
                data: Some(ics),
            },
        )
        .unwrap();
        let parsed = &parsed[0];

        assert_eq!(parsed.canonical_uid, "fixture@example.com");
        assert_eq!(parsed.status, EventStatus::Tentative);
        assert_eq!(parsed.visibility, EventVisibility::Private);
        assert_eq!(parsed.attendees[0].email, "person@example.com");
        assert_eq!(parsed.attendees[0].name.as_deref(), Some("Person"));
        assert_eq!(
            parsed.attendees[0].response_status.as_deref(),
            Some("accepted")
        );
        assert!(parsed.attendees[0].optional);
        assert_eq!(parsed.reminders[0].method, "display");
        assert_eq!(parsed.reminders[0].minutes_before_start, 30);
        let recurrence = parsed.recurrence.as_ref().unwrap();
        assert_eq!(recurrence.rrule.as_deref(), Some("FREQ=WEEKLY;COUNT=3"));
        assert_eq!(recurrence.exdates, vec!["20260608T130000Z"]);
        assert_eq!(
            recurrence.recurrence_id.as_deref(),
            Some("20260601T130000Z")
        );
        assert_eq!(recurrence.sequence, Some(4));
    }

    #[test]
    fn cancelled_ical_event_is_marked_deleted() {
        let data = [
            "BEGIN:VCALENDAR",
            "VERSION:2.0",
            "BEGIN:VEVENT",
            "UID:cancelled@example.com",
            "STATUS:CANCELLED",
            "DTSTART:20260601T120000Z",
            "DTEND:20260601T130000Z",
            "SUMMARY:Cancelled",
            "END:VEVENT",
            "END:VCALENDAR",
        ]
        .join("\r\n");

        let parsed = ical_object_to_canonical(
            "icloud-calendar",
            CalendarObject {
                url: "https://example.com/calendar/cancelled.ics".to_string(),
                etag: None,
                data: Some(data),
            },
        )
        .unwrap();

        assert_eq!(parsed[0].status, EventStatus::Cancelled);
        assert!(parsed[0].provider_meta.deleted);
    }

    #[test]
    fn writes_timezone_local_date_times_without_utc_suffix() {
        let mut event = timed_event();
        event.canonical_uid = "berlin@example.com".to_string();
        event.title = "Berlin".to_string();
        event.start = EventDateTime::DateTime {
            value: Utc.with_ymd_and_hms(2026, 6, 6, 12, 0, 0).unwrap(),
            timezone: Some("Europe/Berlin".to_string()),
        };
        event.end = EventDateTime::DateTime {
            value: Utc.with_ymd_and_hms(2026, 6, 6, 15, 0, 0).unwrap(),
            timezone: Some("Europe/Berlin".to_string()),
        };

        let ics = canonical_to_ical(&event);

        assert!(ics.contains("DTSTART;TZID=Europe/Berlin:20260606T120000"));
        assert!(ics.contains("DTEND;TZID=Europe/Berlin:20260606T150000"));
        assert!(!ics.contains("TZID=Europe/Berlin:20260606T120000Z"));
    }

    #[test]
    fn parses_all_day_attendees_and_recurrence() {
        let data = [
            "BEGIN:VCALENDAR",
            "VERSION:2.0",
            "BEGIN:VEVENT",
            "UID:all-day@example.com",
            "DTSTART;VALUE=DATE:20260601",
            "DTEND;VALUE=DATE:20260602",
            "SUMMARY:Planning",
            "CLASS:PRIVATE",
            "RRULE:FREQ=DAILY;COUNT=2",
            "EXDATE;VALUE=DATE:20260603",
            "SEQUENCE:7",
            "ATTENDEE;CN=Person;PARTSTAT=ACCEPTED;ROLE=OPT-PARTICIPANT:mailto:person@example.com",
            "END:VEVENT",
            "END:VCALENDAR",
        ]
        .join("\r\n");

        let parsed = ical_object_to_canonical(
            "icloud-calendar",
            CalendarObject {
                url: "https://example.com/calendar/all-day.ics".to_string(),
                etag: None,
                data: Some(data),
            },
        )
        .unwrap();

        assert_eq!(
            parsed[0].start,
            EventDateTime::Date {
                value: NaiveDate::from_ymd_opt(2026, 6, 1).unwrap()
            }
        );
        assert_eq!(parsed[0].visibility, EventVisibility::Private);
        assert_eq!(parsed[0].attendees[0].email, "person@example.com");
        assert_eq!(
            parsed[0].attendees[0].response_status.as_deref(),
            Some("accepted")
        );
        assert!(parsed[0].attendees[0].optional);
        let recurrence = parsed[0].recurrence.as_ref().unwrap();
        assert_eq!(recurrence.rrule.as_deref(), Some("FREQ=DAILY;COUNT=2"));
        assert_eq!(recurrence.exdates, vec!["20260603"]);
        assert_eq!(recurrence.sequence, Some(7));
    }

    #[test]
    fn repairs_literal_newlines_in_text_properties() {
        let malformed = [
            "BEGIN:VCALENDAR",
            "VERSION:2.0",
            "BEGIN:VEVENT",
            "UID:bad-location@example.com",
            "DTSTART;TZID=Europe/Berlin:20240727T180000",
            "DTEND;TZID=Europe/Berlin:20240727T230000",
            "SUMMARY:Gin Tasting",
            "LOCATION:Privat von Miriam Castle",
            "Schaefergasse 1",
            "65817 Bremthal",
            "Deutschland",
            "END:VEVENT",
            "END:VCALENDAR",
        ]
        .join("\r\n");

        let parsed = ical_object_to_canonical(
            "icloud-calendar",
            CalendarObject {
                url: "https://example.com/calendar/bad.ics".to_string(),
                data: Some(malformed),
                etag: None,
            },
        )
        .unwrap();

        assert!(
            parsed[0]
                .location
                .as_deref()
                .unwrap()
                .contains("Schaefergasse 1")
        );
        assert!(
            parsed[0]
                .location
                .as_deref()
                .unwrap()
                .contains("65817 Bremthal")
        );
    }

    #[test]
    fn event_filename_sanitizes_uid() {
        let mut event = timed_event();
        event.canonical_uid = "hello/world@example.com".to_string();

        assert_eq!(event_filename(&event), "hello_world_example.com.ics");
    }
}
