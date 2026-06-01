import { createHash } from "node:crypto";
import type { CanonicalEvent } from "../providers/types";

export function hashCanonicalEvent(event: CanonicalEvent): string {
  const comparable = {
    title: event.title,
    description: event.description,
    location: event.location,
    status: event.status,
    visibility: event.visibility,
    start: event.start,
    end: event.end,
    recurrence: event.recurrence,
    attendees: event.attendees,
    reminders: event.reminders
  };

  return createHash("sha256")
    .update(JSON.stringify(sortObject(comparable)))
    .digest("hex");
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
