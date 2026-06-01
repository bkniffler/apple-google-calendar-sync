import { describe, expect, test } from "bun:test";
import type { CanonicalEvent } from "../types";
import { canonicalToICal, icalObjectToCanonical } from "./ical-mapper";

describe("iCalendar mapper", () => {
  test("round-trips a timed event", () => {
    const event: CanonicalEvent = {
      canonicalUid: "uid@example.com",
      title: "Planning",
      description: "Roadmap",
      location: "Office",
      status: "confirmed",
      visibility: "default",
      start: {
        kind: "dateTime",
        value: "2026-06-01T13:00:00.000Z",
        timezone: "UTC"
      },
      end: {
        kind: "dateTime",
        value: "2026-06-01T14:00:00.000Z",
        timezone: "UTC"
      },
      attendees: [],
      reminders: [{ method: "display", minutesBeforeStart: 10 }],
      providerMeta: {
        provider: "google",
        calendarId: "primary"
      },
      raw: {}
    };

    const ics = canonicalToICal(event);
    const [parsed] = icalObjectToCanonical("icloud-calendar", {
      url: "https://example.com/calendar/uid.ics",
      etag: "etag",
      data: ics
    });

    expect(parsed).toMatchObject({
      canonicalUid: "uid@example.com",
      title: "Planning",
      description: "Roadmap",
      location: "Office",
      providerMeta: {
        provider: "icloud",
        href: "https://example.com/calendar/uid.ics"
      }
    });
  });
});
