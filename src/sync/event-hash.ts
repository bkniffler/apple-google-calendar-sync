import { createHash } from "node:crypto";
import type { CanonicalEvent, EventDateTime, RecurrenceData } from "../providers/types";

export function hashCanonicalEvent(event: CanonicalEvent): string {
  const comparable = {
    title: normalizeText(event.title),
    description: normalizeText(event.description),
    location: normalizeLocation(event.location),
    status: event.status,
    start: normalizeDateTime(event.start),
    end: normalizeDateTime(event.end),
    recurrence: normalizeRecurrence(event.recurrence)
  };

  return createHash("sha256")
    .update(JSON.stringify(sortObject(comparable)))
    .digest("hex");
}

function normalizeDateTime(value: EventDateTime): EventDateTime {
  if (value.kind === "date") {
    return value;
  }

  return {
    kind: "dateTime",
    value: new Date(value.value).toISOString()
  };
}

function normalizeRecurrence(value: RecurrenceData | undefined): RecurrenceData | undefined {
  if (!value) {
    return undefined;
  }

  const recurrence = {
    rrule: value.rrule,
    exdates: value.exdates?.filter(Boolean).sort(),
    recurrenceId: normalizeRecurrenceId(value.recurrenceId)
  };

  if (!recurrence.rrule && !recurrence.recurrenceId && !recurrence.exdates?.length) {
    return undefined;
  }

  return recurrence;
}

function normalizeRecurrenceId(value: string | undefined): string | undefined {
  if (!value) {
    return undefined;
  }

  const match = value.match(/^(\d{4}-\d{2}-\d{2})(?:T(\d{2}:\d{2}:\d{2}))?/);
  if (!match) {
    return value;
  }

  return match[2] ? `${match[1]}T${match[2]}` : match[1];
}

function normalizeText(value: string | undefined): string {
  return (value ?? "").replace(/\r\n/g, "\n").trim();
}

function normalizeLocation(value: string | undefined): string {
  return normalizeText(value).replace(/\\n/g, "\n");
}

function sortObject(value: unknown): unknown {
  if (Array.isArray(value)) {
    return value.map(sortObject);
  }

  if (value && typeof value === "object") {
    return Object.fromEntries(
      Object.entries(value)
        .sort(([a], [b]) => a.localeCompare(b))
        .map(([key, nested]) => [key, sortObject(nested)])
    );
  }

  return value;
}
