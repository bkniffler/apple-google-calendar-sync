import { describe, expect, test } from "bun:test";
import { googleToCanonical } from "./google-mapper";
import type { GoogleEvent } from "./google-types";

describe("googleToCanonical", () => {
  test("normalizes a timed Google event", () => {
    const event: GoogleEvent = {
      id: "abc123",
      etag: "\"etag\"",
      iCalUID: "uid@example.com",
      status: "confirmed",
      summary: "Planning",
      start: {
        dateTime: "2026-06-01T09:00:00-04:00",
        timeZone: "America/New_York"
      },
      end: {
        dateTime: "2026-06-01T10:00:00-04:00",
        timeZone: "America/New_York"
      },
      reminders: {
        useDefault: false,
        overrides: [{ method: "popup", minutes: 15 }]
      }
    };

    expect(googleToCanonical("primary", event)).toMatchObject({
      canonicalUid: "uid@example.com",
      title: "Planning",
      start: {
        kind: "dateTime",
        value: "2026-06-01T09:00:00-04:00",
        timezone: "America/New_York"
      },
      reminders: [{ method: "popup", minutesBeforeStart: 15 }]
    });
  });
});
