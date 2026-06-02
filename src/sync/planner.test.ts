import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import type { EventLink } from "../db/repositories";
import type { CanonicalEvent } from "../providers/types";
import { hashCanonicalEvent } from "./event-hash";
import { planInitialActions } from "./planner";
import { planTwoWayActions } from "./runner";

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

describe("planTwoWayActions conflict policies", () => {
  test("auto-resolves both-side changes with newest updated wins", () => {
    const google = {
      ...makeEvent("shared-uid", "New Google Title"),
      providerMeta: {
        ...makeEvent("shared-uid", "New Google Title").providerMeta,
        updatedAt: "2026-06-02T10:00:00Z"
      }
    };
    const icloud = {
      ...makeEvent("shared-uid", "Old iCloud Title"),
      providerMeta: {
        ...makeEvent("shared-uid", "Old iCloud Title").providerMeta,
        updatedAt: "2026-06-02T09:00:00Z"
      }
    };

    const actions = planTwoWayActions({
      links: [makeLink("shared-uid")],
      googleEvents: [google],
      icloudEvents: [icloud],
      direction: "two_way",
      conflictPolicy: {
        bothSidesChanged: "newest_updated_wins",
        unlinkedSameUid: "manual",
        deleteVsUpdate: "update_wins",
        icloudUidCollision: "ignore_known"
      }
    });

    expect(actions).toMatchObject([
      {
        kind: "update_icloud",
        canonicalUid: "shared-uid",
        resolution: {
          kind: "auto_resolved",
          reason: "both_sides_changed",
          policy: "newest_updated_wins"
        }
      }
    ]);
  });

  test("auto-resolves delete-versus-update with update wins", () => {
    const google = makeEvent("shared-uid", "Changed on Google");

    const actions = planTwoWayActions({
      links: [
        {
          ...makeLink("shared-uid"),
          icloud_href: "https://caldav.icloud.com/calendars/example/shared-uid.ics"
        }
      ],
      googleEvents: [google],
      icloudEvents: [],
      direction: "two_way",
      conflictPolicy: {
        bothSidesChanged: "manual",
        unlinkedSameUid: "manual",
        deleteVsUpdate: "update_wins",
        icloudUidCollision: "ignore_known"
      }
    });

    expect(actions).toMatchObject([
      {
        kind: "create_icloud",
        canonicalUid: "shared-uid",
        resolution: {
          kind: "auto_resolved",
          reason: "google_changed_while_icloud_deleted",
          policy: "update_wins"
        }
      }
    ]);
  });
});

describe("planner parity fixtures", () => {
  const fixture = loadPlannerParityFixture();

  for (const item of fixture.initialCases) {
    test(`initial: ${item.name}`, () => {
      const googleEvents = item.googleEvents.map((event) => fixtureEvent("google", event));
      const icloudEvents = item.icloudEvents.map((event) => fixtureEvent("icloud", event));

      expect(planInitialActions(googleEvents, icloudEvents).map(actionSummary)).toEqual(
        item.expected
      );
    });
  }

  for (const item of fixture.twoWayCases) {
    test(`two-way: ${item.name}`, () => {
      const googleEvents = item.googleEvents.map((event) => fixtureEvent("google", event));
      const icloudEvents = item.icloudEvents.map((event) => fixtureEvent("icloud", event));
      const links = item.links.map((link) => fixtureLink(link, googleEvents, icloudEvents));

      const actions = planTwoWayActions({
        links,
        googleEvents,
        icloudEvents,
        knownICloudUidCollisions: new Set(item.knownICloudUidCollisions ?? []),
        direction: item.direction,
        conflictPolicy: item.conflictPolicy
      });

      expect(actions.map(actionSummary)).toEqual(item.expected);
    });
  }
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

type PlannerParityFixture = {
  initialCases: InitialFixtureCase[];
  twoWayCases: TwoWayFixtureCase[];
};

type InitialFixtureCase = {
  name: string;
  googleEvents: EventSpec[];
  icloudEvents: EventSpec[];
  expected: ActionSummary[];
};

type TwoWayFixtureCase = InitialFixtureCase & {
  direction: "two_way" | "left_to_right" | "right_to_left";
  conflictPolicy: {
    bothSidesChanged: "manual" | "google_wins" | "icloud_wins" | "newest_updated_wins";
    unlinkedSameUid: "manual" | "google_wins" | "icloud_wins" | "newest_updated_wins";
    deleteVsUpdate: "manual" | "delete_wins" | "update_wins";
    icloudUidCollision: "manual" | "ignore_known";
  };
  links: LinkSpec[];
  knownICloudUidCollisions?: string[] | undefined;
};

type EventSpec = {
  uid: string;
  title: string;
  updatedAt?: string | undefined;
  deleted?: boolean | undefined;
};

type LinkSpec = {
  uid: string;
  googleHash?: string | undefined;
  icloudHash?: string | undefined;
};

type ActionSummary = {
  kind: string;
  canonicalUid: string;
  reason?: string | undefined;
  resolution?: string | undefined;
  policy?: string | undefined;
  eventTitle?: string | undefined;
};

function loadPlannerParityFixture(): PlannerParityFixture {
  return JSON.parse(
    readFileSync(join(import.meta.dir, "../../test-fixtures/planner-parity.json"), "utf8")
  ) as PlannerParityFixture;
}

function fixtureEvent(provider: "google" | "icloud", spec: EventSpec): CanonicalEvent {
  return {
    canonicalUid: spec.uid,
    title: spec.title,
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
      provider,
      calendarId: provider === "google" ? "primary" : "icloud-calendar",
      ...(provider === "google"
        ? { eventId: spec.uid }
        : { href: `https://caldav.icloud.com/calendars/example/${spec.uid}.ics` }),
      etag: `${provider}-etag-${spec.uid}`,
      iCalUid: spec.uid,
      ...(spec.updatedAt ? { updatedAt: spec.updatedAt } : {}),
      ...(spec.deleted ? { deleted: true } : {})
    },
    raw: {}
  };
}

function fixtureLink(
  spec: LinkSpec,
  googleEvents: CanonicalEvent[],
  icloudEvents: CanonicalEvent[]
): EventLink {
  return {
    id: `link-${spec.uid}`,
    sync_pair_id: "personal",
    canonical_uid: spec.uid,
    google_event_id: spec.uid,
    google_ical_uid: spec.uid,
    google_etag: "old-google-etag",
    icloud_href: `https://caldav.icloud.com/calendars/example/${spec.uid}.ics`,
    icloud_uid: spec.uid,
    icloud_etag: "old-icloud-etag",
    google_hash: resolveHash(spec.googleHash, "google", googleEvents, icloudEvents),
    icloud_hash: resolveHash(spec.icloudHash, "icloud", googleEvents, icloudEvents),
    last_synced_hash: "old-hash",
    deleted_google_at: null,
    deleted_icloud_at: null
  };
}

function resolveHash(
  value: string | undefined,
  side: "google" | "icloud",
  googleEvents: CanonicalEvent[],
  icloudEvents: CanonicalEvent[]
): string | null {
  if (!value) {
    return null;
  }
  const prefix = `$hash:${side}:`;
  if (!value.startsWith(prefix)) {
    return value;
  }
  const uid = value.slice(prefix.length);
  const event = (side === "google" ? googleEvents : icloudEvents).find(
    (item) => item.canonicalUid === uid
  );
  if (!event) {
    throw new Error(`Missing ${side} fixture event for hash placeholder ${value}`);
  }
  return hashCanonicalEvent(event);
}

function actionSummary(
  action: ReturnType<typeof planInitialActions>[number] | ReturnType<typeof planTwoWayActions>[number]
): ActionSummary {
  const event = "event" in action ? action.event : undefined;
  const resolution = "resolution" in action ? action.resolution : undefined;
  const autoResolution =
    typeof resolution === "object" && resolution !== null ? resolution : undefined;
  const conflictResolution = typeof resolution === "string" ? resolution : undefined;
  return stripUndefined({
    kind: action.kind,
    canonicalUid: action.canonicalUid,
    reason:
      action.kind === "conflict" || action.kind === "noop" ? action.reason : autoResolution?.reason,
    resolution:
      action.kind === "conflict"
        ? conflictResolution
        : autoResolution
          ? "auto_resolved"
          : undefined,
    policy: action.kind === "conflict" ? undefined : autoResolution?.policy,
    eventTitle: event?.title
  });
}

function stripUndefined<T extends Record<string, unknown>>(value: T): T {
  return Object.fromEntries(
    Object.entries(value).filter(([, item]) => item !== undefined)
  ) as T;
}

function makeLink(canonicalUid: string) {
  return {
    id: `link-${canonicalUid}`,
    sync_pair_id: "personal",
    canonical_uid: canonicalUid,
    google_event_id: canonicalUid,
    google_ical_uid: canonicalUid,
    google_etag: "old-google-etag",
    icloud_href: `https://caldav.icloud.com/calendars/example/${canonicalUid}.ics`,
    icloud_uid: canonicalUid,
    icloud_etag: "old-icloud-etag",
    google_hash: "old-google-hash",
    icloud_hash: "old-icloud-hash",
    last_synced_hash: "old-hash",
    deleted_google_at: null,
    deleted_icloud_at: null
  };
}
