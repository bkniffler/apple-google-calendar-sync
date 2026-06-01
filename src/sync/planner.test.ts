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
