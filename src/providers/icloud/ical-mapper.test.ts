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
    expect(ics).not.toContain("CLASS:DEFAULT");
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

  test("writes timezone-local date-times without UTC suffix", () => {
    const event: CanonicalEvent = {
      canonicalUid: "berlin@example.com",
      title: "Berlin",
      status: "confirmed",
      visibility: "default",
      start: {
        kind: "dateTime",
        value: "2026-06-06T12:00:00+02:00",
        timezone: "Europe/Berlin"
      },
      end: {
        kind: "dateTime",
        value: "2026-06-06T15:00:00+02:00",
        timezone: "Europe/Berlin"
      },
      attendees: [],
      reminders: [],
      providerMeta: {
        provider: "google",
        calendarId: "primary"
      },
      raw: {}
    };

    const ics = canonicalToICal(event);

    expect(ics).toContain("DTSTART;TZID=Europe/Berlin:20260606T120000");
    expect(ics).toContain("DTEND;TZID=Europe/Berlin:20260606T150000");
    expect(ics).not.toContain("TZID=Europe/Berlin:20260606T100000Z");
  });

  test("repairs literal newlines in text properties from malformed producers", () => {
    const malformed = [
      "BEGIN:VCALENDAR",
      "VERSION:2.0",
      "BEGIN:VEVENT",
      "UID:bad-location@example.com",
      "DTSTART;TZID=Europe/Berlin:20240727T180000",
      "DTEND;TZID=Europe/Berlin:20240727T230000",
      "SUMMARY:Gin Tasting",
      "LOCATION:Privat von Miriam Castle",
      "Schäfergasse 1",
      "65817 Bremthal",
      "Deutschland",
      "END:VEVENT",
      "END:VCALENDAR"
    ].join("\r\n");

    const [parsed] = icalObjectToCanonical("icloud-calendar", {
      url: "https://example.com/calendar/bad.ics",
      data: malformed
    });

    expect(parsed?.location).toContain("Schäfergasse 1");
    expect(parsed?.location).toContain("65817 Bremthal");
  });

  test("drops malformed Apple structured location blocks", () => {
    const malformed = [
      "BEGIN:VCALENDAR",
      "VERSION:2.0",
      "BEGIN:VEVENT",
      "UID:bad-structured-location@example.com",
      "DTSTART;TZID=Europe/Berlin:20240727T180000",
      "DTEND;TZID=Europe/Berlin:20240727T230000",
      "SUMMARY:Gin Tasting",
      "LOCATION:Privat von Miriam Castle\\nSchäfergasse 1\\n65817 Bremthal",
      "X-APPLE-STRUCTURED-LOCATION;VALUE=URI;X-TITLE=Privat:",
      "65817 Bremthal",
      "Deutschland\":geo:50.140315,8.359774",
      "X-APPLE-CREATOR-IDENTITY:com.apple.mobilecal",
      "END:VEVENT",
      "END:VCALENDAR"
    ].join("\r\n");

    const [parsed] = icalObjectToCanonical("icloud-calendar", {
      url: "https://example.com/calendar/bad-structured.ics",
      data: malformed
    });

    expect(parsed?.canonicalUid).toBe("bad-structured-location@example.com");
    expect(parsed?.location).toContain("Privat von Miriam Castle");
  });
});
