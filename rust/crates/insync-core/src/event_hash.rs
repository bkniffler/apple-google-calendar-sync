use crate::{CanonicalEvent, EventDateTime, RecurrenceData};
use chrono::SecondsFormat;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

pub fn hash_canonical_event(event: &CanonicalEvent) -> String {
    let comparable = json!({
        "title": normalize_text(Some(&event.title)),
        "description": normalize_text(event.description.as_ref()),
        "location": normalize_location(event.location.as_ref()),
        "status": event.status,
        "start": normalize_date_time(&event.start),
        "end": normalize_date_time(&event.end),
        "recurrence": normalize_recurrence(event.recurrence.as_ref())
    });
    let sorted = sort_value(comparable);
    let body = serde_json::to_string(&sorted).expect("hash JSON serialization should not fail");
    let digest = Sha256::digest(body.as_bytes());
    format!("{digest:x}")
}

fn normalize_date_time(value: &EventDateTime) -> Value {
    match value {
        EventDateTime::Date { value } => json!({
            "kind": "date",
            "value": value.format("%Y-%m-%d").to_string()
        }),
        EventDateTime::DateTime { value, .. } => json!({
            "kind": "dateTime",
            "value": value.to_rfc3339_opts(SecondsFormat::Millis, true)
        }),
    }
}

fn normalize_recurrence(value: Option<&RecurrenceData>) -> Value {
    let Some(value) = value else {
        return Value::Null;
    };

    let mut exdates = value
        .exdates
        .iter()
        .filter(|item| !item.is_empty())
        .cloned()
        .collect::<Vec<_>>();
    exdates.sort();

    let recurrence_id = value
        .recurrence_id
        .as_deref()
        .map(normalize_recurrence_id)
        .filter(|item| !item.is_empty());

    if value.rrule.is_none() && exdates.is_empty() && recurrence_id.is_none() {
        return Value::Null;
    }

    json!({
        "rrule": value.rrule,
        "exdates": exdates,
        "recurrenceId": recurrence_id
    })
}

fn normalize_recurrence_id(value: &str) -> String {
    if value.len() >= 19
        && value.as_bytes().get(4) == Some(&b'-')
        && value.as_bytes().get(7) == Some(&b'-')
        && value.as_bytes().get(10) == Some(&b'T')
    {
        return value[..19].to_string();
    }

    if value.len() >= 10
        && value.as_bytes().get(4) == Some(&b'-')
        && value.as_bytes().get(7) == Some(&b'-')
    {
        return value[..10].to_string();
    }

    value.to_string()
}

fn normalize_text(value: Option<&String>) -> String {
    value
        .map(|item| item.replace("\r\n", "\n").trim().to_string())
        .unwrap_or_default()
}

fn normalize_location(value: Option<&String>) -> String {
    normalize_text(value).replace("\\n", "\n")
}

fn sort_value(value: Value) -> Value {
    match value {
        Value::Array(items) => Value::Array(items.into_iter().map(sort_value).collect()),
        Value::Object(object) => {
            let sorted = object
                .into_iter()
                .map(|(key, value)| (key, sort_value(value)))
                .collect::<Map<_, _>>();
            Value::Object(sorted)
        }
        value => value,
    }
}
