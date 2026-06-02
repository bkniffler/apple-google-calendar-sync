# Provider Mapping Notes

This file documents what the Rust provider mappers preserve and what is known
to be lossy. The sync planner compares canonical events, so every lossy field
below is either intentionally ignored or stored only in provider metadata/raw
payloads for diagnostics.

## Preserved

- Event identity: canonical UID, Google event ID/iCalUID, iCloud href/UID.
- Basic fields: title, description, location, status, visibility/privacy.
- Time values: all-day dates, timed events, UTC instants, and provider time zone
  labels when available.
- Recurrence basics: RRULE, EXDATE, recurrence ID, and sequence.
- Attendees: email, display name, response status, and optional flag.
- Reminders: method and minutes before start.
- Provider metadata: etag, updated timestamp where supplied, deleted/cancelled
  state, and raw provider payloads for inspection.

## Known Losses

- Google reminder defaults are not expanded. Empty canonical reminders serialize
  as Google `useDefault: true`; provider defaults are not copied into each event.
- Google supports only valid IANA-like time zone IDs in event writes. Unknown
  time zone labels are preserved in canonical data but omitted from Google write
  payloads.
- Invalid attendee identifiers are not written to Google. This intentionally
  drops CalDAV principal URLs and malformed attendee values that Google rejects.
- iCalendar timezone definitions (`VTIMEZONE`) are not preserved verbatim. Rust
  keeps the `TZID` label and UTC instant used for comparison.
- iCalendar alarms are normalized to minutes before start and read back as
  display reminders. Alarm descriptions, email alarm methods/attendees,
  absolute triggers, and repeat/count metadata are not round-tripped.
- iCalendar attendee parameters beyond CN, PARTSTAT, and optional role are not
  preserved.
- Provider-specific cosmetic fields such as Google colors, transparency,
  hangout/conference data, organizer/creator details, and CalDAV collection
  display metadata are kept in raw payloads when read but are not part of the
  canonical comparison.

## Test Coverage

The Rust provider tests cover all-day and timed events, time zones, recurrence,
attendees, reminders, cancelled events, privacy/visibility, status, and
round-trips where provider limitations allow it. Live provider integration tests
should still run against throwaway calendars before Rust apply is trusted for a
primary calendar set.
