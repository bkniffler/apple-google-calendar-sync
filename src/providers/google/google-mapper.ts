import type { CanonicalEvent, EventDateTime, ProviderEventMeta } from "../types";
import type { GoogleEvent, GoogleEventDateTime } from "./google-types";

const PRIVATE_UID_KEY = "insyncCanonicalUid";
const PRIVATE_SOURCE_KEY = "insyncSource";

export function googleToCanonical(calendarId: string, event: GoogleEvent): CanonicalEvent {
  const canonicalUid =
    event.extendedProperties?.private?.[PRIVATE_UID_KEY] ??
    event.iCalUID ??
    event.id ??
    crypto.randomUUID();

  return {
    canonicalUid,
    title: event.summary ?? "",
    description: event.description ?? undefined,
    location: event.location ?? undefined,
    status: normalizeStatus(event.status),
    visibility: event.visibility ?? "default",
    start: googleDateToCanonical(event.start),
    end: googleDateToCanonical(event.end),
    recurrence: parseGoogleRecurrence(event),
    attendees:
      event.attendees
        ?.filter((attendee) => attendee.email)
        .map((attendee) => ({
          email: attendee.email as string,
          name: attendee.displayName ?? undefined,
          responseStatus: attendee.responseStatus,
          optional: attendee.optional
        })) ?? [],
    reminders:
      event.reminders?.overrides
        ?.filter((reminder) => typeof reminder.minutes === "number")
        .map((reminder) => ({
          method: reminder.method ?? "popup",
          minutesBeforeStart: reminder.minutes as number
        })) ?? [],
    providerMeta: {
      provider: "google",
      calendarId,
      eventId: event.id,
      etag: event.etag,
      iCalUid: event.iCalUID,
      updatedAt: event.updated,
      deleted: event.status === "cancelled"
    },
    raw: event
  };
}

export function canonicalToGoogle(
  event: CanonicalEvent,
  source: "google" | "icloud" = "icloud"
): GoogleEvent {
  return {
    summary: event.title,
    description: event.description,
    location: event.location,
    status: event.status,
    visibility: event.visibility,
    start: canonicalDateToGoogle(event.start),
    end: canonicalDateToGoogle(event.end),
    recurrence: googleRecurrenceLines(event),
    attendees: event.attendees
      .filter((attendee) => isEmailLike(attendee.email))
      .map((attendee) => ({
        email: attendee.email,
        displayName: attendee.name,
        optional: attendee.optional,
        responseStatus: attendee.responseStatus
      })),
    reminders: {
      useDefault: event.reminders.length === 0,
      overrides:
        event.reminders.length > 0
          ? event.reminders.map((reminder) => ({
              method: reminder.method === "email" ? "email" : "popup",
              minutes: reminder.minutesBeforeStart
            }))
          : undefined
    },
    extendedProperties: {
      private: {
        [PRIVATE_UID_KEY]: event.canonicalUid,
        [PRIVATE_SOURCE_KEY]: source
      }
    }
  };
}

export function googleMetaFromResponse(
  calendarId: string,
  event: GoogleEvent
): ProviderEventMeta {
  return {
    provider: "google",
    calendarId,
    eventId: event.id,
    etag: event.etag,
    iCalUid: event.iCalUID,
    updatedAt: event.updated,
    deleted: event.status === "cancelled"
  };
}

function normalizeStatus(status: GoogleEvent["status"]): CanonicalEvent["status"] {
  if (status === "tentative" || status === "cancelled") {
    return status;
  }
  return "confirmed";
}

function googleDateToCanonical(value: GoogleEventDateTime | undefined): EventDateTime {
  if (!value) {
    return { kind: "dateTime", value: new Date(0).toISOString(), timezone: "UTC" };
  }

  if ("date" in value && value.date) {
    return { kind: "date", value: value.date };
  }

  return {
    kind: "dateTime",
    value: value.dateTime ?? new Date(0).toISOString(),
    timezone: value.timeZone
  };
}

function canonicalDateToGoogle(value: EventDateTime): GoogleEventDateTime {
  if (value.kind === "date") {
    return { date: value.value };
  }

  return {
    dateTime: value.value,
    timeZone: isGoogleTimeZone(value.timezone) ? value.timezone : undefined
  };
}

function parseGoogleRecurrence(event: GoogleEvent): CanonicalEvent["recurrence"] {
  const recurrence = event.recurrence ?? [];
  const rrule = recurrence.find((line) => line.startsWith("RRULE:"))?.slice("RRULE:".length);
  const exdates = recurrence
    .filter((line) => line.startsWith("EXDATE"))
    .map((line) => line.slice(line.indexOf(":") + 1));

  if (!rrule && exdates.length === 0 && !event.originalStartTime && event.sequence === undefined) {
    return undefined;
  }

  return {
    rrule,
    exdates,
    recurrenceId: event.originalStartTime
      ? event.originalStartTime.date ?? event.originalStartTime.dateTime
      : undefined,
    sequence: event.sequence
  };
}

function googleRecurrenceLines(event: CanonicalEvent): string[] | undefined {
  const lines: string[] = [];
  if (event.recurrence?.rrule) {
    lines.push(`RRULE:${event.recurrence.rrule}`);
  }

  for (const exdate of event.recurrence?.exdates ?? []) {
    const line = googleExdateLine(event, exdate);
    if (line) {
      lines.push(line);
    }
  }

  return lines.length > 0 ? lines : undefined;
}

function isEmailLike(value: string): boolean {
  return /^[^\s@<>]+@[^\s@<>]+\.[^\s@<>]+$/.test(value);
}

function isGoogleTimeZone(value: string | undefined): value is string {
  return value === "UTC" || Boolean(value?.includes("/"));
}

function googleExdateLine(event: CanonicalEvent, value: string): string | undefined {
  if (/^\d{8}$/.test(value)) {
    return `EXDATE;VALUE=DATE:${value}`;
  }

  if (/^\d{4}-\d{2}-\d{2}$/.test(value)) {
    return `EXDATE;VALUE=DATE:${value.replaceAll("-", "")}`;
  }

  const localDateTime = value.match(
    /^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2})(?::(\d{2}))?$/
  );
  if (localDateTime) {
    const compact = `${localDateTime[1]}${localDateTime[2]}${localDateTime[3]}T${localDateTime[4]}${localDateTime[5]}${localDateTime[6] ?? "00"}`;
    const timezone = event.start.kind === "dateTime" ? event.start.timezone : undefined;
    return isGoogleTimeZone(timezone) ? `EXDATE;TZID=${timezone}:${compact}` : `EXDATE:${compact}`;
  }

  const parsed = new Date(value);
  if (Number.isFinite(parsed.getTime())) {
    return `EXDATE:${parsed.toISOString().replace(/[-:]/g, "").replace(/\.\d{3}Z$/, "Z")}`;
  }

  return undefined;
}
