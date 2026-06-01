import { describe, expect, test } from "bun:test";
import type { CanonicalEvent } from "../providers/types";
import { hashCanonicalEvent } from "./event-hash";
import { planInitialActions } from "./planner";

describe("planInitialActions", () => {
  test("creates missing event on the opposite provider", () => {
    const googleEvent = makeEvent("shared-uid", "Lunch");
    const icloudEvent = makeEvent("icloud-only", "Dentist");

    const actions = planInitialActions([googleEvent], [icloudEvent]);

    expect(actions).toEqual([
      {
        kind: "create_icloud",
        canonicalUid: "shared-uid",
        event: googleEvent
      },
      {
        kind: "create_google",
        canonicalUid: "icloud-only",
        event: icloudEvent
      }
    ]);
  });

  test("does not plan a create when both sides have the event", () => {
    const googleEvent = makeEvent("shared-uid", "Lunch");
    const icloudEvent = makeEvent("shared-uid", "Lunch");

    expect(planInitialActions([googleEvent], [icloudEvent])).toEqual([
      {
        kind: "noop",
        canonicalUid: "shared-uid",
        reason: "present_on_both_sides"
      }
    ]);
  });
});

describe("hashCanonicalEvent", () => {
  test("ignores provider metadata", () => {
    const base = makeEvent("shared-uid", "Lunch");
    const movedProviderMeta = {
      ...base,
      providerMeta: {
        ...base.providerMeta,
        etag: "changed"
      }
    };

    expect(hashCanonicalEvent(base)).toEqual(hashCanonicalEvent(movedProviderMeta));
  });

  test("treats equivalent date-time representations as equal", () => {
    const google = {
      ...makeEvent("shared-uid", "Lunch"),
      start: {
        kind: "dateTime" as const,
        value: "2026-06-06T12:00:00+02:00",
        timezone: "Europe/Berlin"
      },
      end: {
        kind: "dateTime" as const,
        value: "2026-06-06T13:00:00+02:00",
        timezone: "Europe/Berlin"
      },
      reminders: [{ method: "popup" as const, minutesBeforeStart: 0 }],
      recurrence: { sequence: 1 }
    };
    const icloud = {
      ...makeEvent("shared-uid", "Lunch"),
      start: {
        kind: "dateTime" as const,
        value: "2026-06-06T10:00:00.000Z",
        timezone: "Europe/Berlin"
      },
      end: {
        kind: "dateTime" as const,
        value: "2026-06-06T11:00:00.000Z",
        timezone: "Europe/Berlin"
      },
      reminders: [{ method: "display" as const, minutesBeforeStart: 15 }],
      recurrence: { sequence: 5 }
    };

    expect(hashCanonicalEvent(google)).toEqual(hashCanonicalEvent(icloud));
  });

  test("ignores provider-specific metadata that should not drive sync", () => {
    const google = {
      ...makeEvent("shared-uid", "Lunch"),
      visibility: "public" as const,
      attendees: [],
      reminders: [{ method: "email" as const, minutesBeforeStart: 10 }]
    };
    const icloud = {
      ...makeEvent("shared-uid", "Lunch"),
      visibility: "default" as const,
      attendees: [
        {
          email: "/apple-principal/",
          name: "Apple Principal",
          responseStatus: "accepted" as const
        }
      ],
      reminders: []
    };

    expect(hashCanonicalEvent(google)).toEqual(hashCanonicalEvent(icloud));
  });

  test("ignores timezone labels when instants match", () => {
    const google = {
      ...makeEvent("shared-uid", "Flight"),
      start: {
        kind: "dateTime" as const,
        value: "2025-03-19T16:55:00.000Z",
        timezone: "UTC"
      },
      end: {
        kind: "dateTime" as const,
        value: "2025-03-19T19:20:00.000Z",
        timezone: "UTC"
      }
    };
    const icloud = {
      ...makeEvent("shared-uid", "Flight"),
      start: {
        kind: "dateTime" as const,
        value: "2025-03-19T17:55:00+01:00",
        timezone: "GMT+0100"
      },
      end: {
        kind: "dateTime" as const,
        value: "2025-03-19T20:20:00+01:00",
        timezone: "GMT+0100"
      }
    };

    expect(hashCanonicalEvent(google)).toEqual(hashCanonicalEvent(icloud));
  });

  test("ignores harmless text formatting differences", () => {
    const google = {
      ...makeEvent("shared-uid", "Lunch"),
      description: "Bring notes\r\n",
      location: "Room 1\\nFloor 2"
    };
    const icloud = {
      ...makeEvent("shared-uid", "Lunch"),
      description: "Bring notes",
      location: "Room 1\nFloor 2"
    };

    expect(hashCanonicalEvent(google)).toEqual(hashCanonicalEvent(icloud));
  });
});

function makeEvent(canonicalUid: string, title: string): CanonicalEvent {
  return {
    canonicalUid,
    title,
    status: "confirmed",
    visibility: "default",
    start: {
      kind: "dateTime",
      value: "2026-06-01T12:00:00Z",
      timezone: "UTC"
    },
    end: {
      kind: "dateTime",
      value: "2026-06-01T13:00:00Z",
      timezone: "UTC"
    },
    attendees: [],
    reminders: [],
    providerMeta: {
      provider: "google",
      calendarId: "primary",
      eventId: canonicalUid,
      etag: "etag"
    },
    raw: {}
  };
}
