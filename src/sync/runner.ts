import type { Logger } from "pino";
import { mkdirSync, writeFileSync } from "node:fs";
import { dirname } from "node:path";
import type { ResolvedServiceConfig } from "../config/service-config";
import type { AppDatabase } from "../db/database";
import {
  configuredCalendarIds,
  loadEventLinks,
  loadUnresolvedConflictUids,
  recordConflict,
  seedConfiguredPairs,
  updateCalendarSyncToken,
  upsertEventLink,
  type EventLink
} from "../db/repositories";
import { ProviderUidCollisionError } from "../providers/errors";
import type { CalendarProvider, CanonicalEvent, ProviderEventMeta } from "../providers/types";
import { hashCanonicalEvent } from "./event-hash";

export type SyncRunnerOptions = {
  db: AppDatabase;
  config: ResolvedServiceConfig;
  google: CalendarProvider;
  icloud: CalendarProvider;
  logger: Logger;
  dryRun?: boolean;
  reportPath?: string | undefined;
};

export class SyncRunner {
  constructor(private readonly options: SyncRunnerOptions) {}

  async runOnce(): Promise<void> {
    const { db, config, google, icloud, logger } = this.options;
    const dryRun = this.options.dryRun ?? config.dryRun;
    const reportRows: ReportRow[] = [];
    seedConfiguredPairs(db, config);

    for (const pair of config.pairs.filter((item) => item.enabled)) {
      logger.info({ pairId: pair.id }, "starting sync pair");
      const calendarIds = configuredCalendarIds(pair);

      const googleChanges = await google.getChanges(pair.google.calendarId, {
        fullSync: true
      });
      const icloudChanges = await icloud.getChanges(pair.icloud.calendarPath, {
        fullSync: true
      });
      const links = loadEventLinks(db, pair.id);
      const knownICloudUidCollisions = loadUnresolvedConflictUids(
        db,
        pair.id,
        "icloud_uid_exists_in_different_calendar"
      );
      const actions = planTwoWayActions({
        links,
        googleEvents: googleChanges.events,
        icloudEvents: icloudChanges.events,
        knownICloudUidCollisions,
        direction: pair.direction,
        conflictPolicy: config.conflictPolicy
      });

      logger.info(
        {
          pairId: pair.id,
          googleEvents: googleChanges.events.length,
          icloudEvents: icloudChanges.events.length,
          actions: actions.length,
          actionCounts: countActions(actions),
          samples: sampleActions(actions)
        },
        "planned sync actions"
      );

      reportRows.push(...actions.map((action) => actionToReportRow(pair.id, action)));

      for (const action of actions) {
        await this.applyAction(pair.id, pair.google.calendarId, pair.icloud.calendarPath, action);
      }

      updateCalendarSyncToken(db, calendarIds.googleCalendarId, googleChanges.syncToken);
      updateCalendarSyncToken(db, calendarIds.icloudCalendarId, icloudChanges.syncToken);
    }

    if (dryRun && this.options.reportPath) {
      writeCsvReport(this.options.reportPath, reportRows);
      logger.info(
        { reportPath: this.options.reportPath, rows: reportRows.length },
        "wrote dry-run report"
      );
    }
  }

  private async applyAction(
    syncPairId: string,
    googleCalendarId: string,
    icloudCalendarId: string,
    action: PlannedAction
  ): Promise<void> {
    const { db, google, icloud, logger } = this.options;
    const dryRun = this.options.dryRun ?? this.options.config.dryRun;

    if (!dryRun) {
      logger.info(
        {
          action: action.kind,
          canonicalUid: action.canonicalUid,
          dryRun
        },
        "sync action"
      );
    }

    if (dryRun) {
      return;
    }

    if (action.kind === "snapshot") {
      upsertEventLink(db, snapshotUpsert(syncPairId, action));
      return;
    }

    if (action.kind === "conflict") {
      recordConflict(db, {
        syncPairId,
        eventLinkId: action.link?.id,
        canonicalUid: action.canonicalUid,
        reason: action.reason,
        googleSnapshot: action.google?.raw,
        icloudSnapshot: action.icloud?.raw
      });
      return;
    }

    if (action.kind === "create_icloud") {
      const created = await this.createOrRecordICloudCollision(
        syncPairId,
        icloudCalendarId,
        action
      );
      if (!created) {
        return;
      }
      upsertEventLink(
        db,
        snapshotUpsert(syncPairId, {
          ...action,
          google: action.event,
          icloudMeta: created,
          googleHash: hashCanonicalEvent(action.event),
          icloudHash: hashCanonicalEvent(action.event)
        })
      );
      return;
    }

    if (action.kind === "create_google") {
      const created = await google.createEvent(googleCalendarId, action.event);
      upsertEventLink(
        db,
        snapshotUpsert(syncPairId, {
          ...action,
          googleMeta: created,
          icloud: action.event,
          googleHash: hashCanonicalEvent(action.event),
          icloudHash: hashCanonicalEvent(action.event)
        })
      );
      return;
    }

    if (action.kind === "update_icloud") {
      const href = action.link?.icloud_href ?? action.icloud?.providerMeta.href;
      if (!href) {
        const created = await this.createOrRecordICloudCollision(
          syncPairId,
          icloudCalendarId,
          action
        );
        if (!created) {
          return;
        }
        upsertEventLink(
          db,
          snapshotUpsert(syncPairId, {
            ...action,
            google: action.event,
            icloudMeta: created,
            googleHash: hashCanonicalEvent(action.event),
            icloudHash: hashCanonicalEvent(action.event)
          })
        );
        return;
      }

      const updated = await icloud.updateEvent(
        icloudCalendarId,
        href,
        action.event,
        action.link?.icloud_etag ?? action.icloud?.providerMeta.etag
      );
      upsertEventLink(
        db,
        snapshotUpsert(syncPairId, {
          ...action,
          google: action.event,
          icloudMeta: updated,
          googleHash: hashCanonicalEvent(action.event),
          icloudHash: hashCanonicalEvent(action.event)
        })
      );
      return;
    }

    if (action.kind === "update_google") {
      const eventId = action.link?.google_event_id ?? action.google?.providerMeta.eventId;
      if (!eventId) {
        const created = await google.createEvent(googleCalendarId, action.event);
        upsertEventLink(
          db,
          snapshotUpsert(syncPairId, {
            ...action,
            googleMeta: created,
            icloud: action.event,
            googleHash: hashCanonicalEvent(action.event),
            icloudHash: hashCanonicalEvent(action.event)
          })
        );
        return;
      }

      const updated = await google.updateEvent(
        googleCalendarId,
        eventId,
        action.event,
        action.link?.google_etag ?? action.google?.providerMeta.etag
      );
      upsertEventLink(
        db,
        snapshotUpsert(syncPairId, {
          ...action,
          googleMeta: updated,
          icloud: action.event,
          googleHash: hashCanonicalEvent(action.event),
          icloudHash: hashCanonicalEvent(action.event)
        })
      );
      return;
    }

    if (action.kind === "delete_icloud") {
      const href = action.link?.icloud_href ?? action.icloud?.providerMeta.href;
      if (href) {
        await icloud.deleteEvent(
          icloudCalendarId,
          href,
          action.link?.icloud_etag ?? action.icloud?.providerMeta.etag
        );
      }
      upsertEventLink(db, {
        syncPairId,
        canonicalUid: action.canonicalUid,
        deletedICloudAt: new Date().toISOString(),
        deletedGoogleAt: action.link?.deleted_google_at
      });
      return;
    }

    if (action.kind === "delete_google") {
      const eventId = action.link?.google_event_id ?? action.google?.providerMeta.eventId;
      if (eventId) {
        await google.deleteEvent(
          googleCalendarId,
          eventId,
          action.link?.google_etag ?? action.google?.providerMeta.etag
        );
      }
      upsertEventLink(db, {
        syncPairId,
        canonicalUid: action.canonicalUid,
        deletedGoogleAt: new Date().toISOString(),
        deletedICloudAt: action.link?.deleted_icloud_at
      });
    }
  }

  private async createOrRecordICloudCollision(
    syncPairId: string,
    icloudCalendarId: string,
    action: MutatingAction
  ): Promise<ProviderEventMeta | undefined> {
    const { db, icloud, logger } = this.options;

    try {
      return await icloud.createEvent(icloudCalendarId, action.event);
    } catch (error) {
      if (!(error instanceof ProviderUidCollisionError)) {
        throw error;
      }

      logger.warn(
        {
          canonicalUid: error.canonicalUid,
          targetCalendarId: error.targetCalendarId,
          existingCalendarId: error.existingCalendarId,
          existingCalendarName: error.existingCalendarName
        },
        "icloud uid exists in a different calendar; recording conflict"
      );
      recordConflict(db, {
        syncPairId,
        eventLinkId: action.link?.id,
        canonicalUid: action.canonicalUid,
        reason: "icloud_uid_exists_in_different_calendar",
        googleSnapshot: action.google?.raw ?? action.event.raw,
        icloudSnapshot: action.icloud?.raw
      });
      return undefined;
    }
  }
}

function countActions(actions: PlannedAction[]): Record<string, number> {
  const counts: Record<string, number> = {};
  for (const action of actions) {
    counts[action.kind] = (counts[action.kind] ?? 0) + 1;
  }
  return counts;
}

function sampleActions(actions: PlannedAction[], limit = 20): Array<{
  kind: PlannedAction["kind"];
  canonicalUid: string;
  reason?: string | undefined;
}> {
  return actions.slice(0, limit).map((action) => ({
    kind: action.kind,
    canonicalUid: action.canonicalUid,
    reason: action.kind === "conflict" ? action.reason : undefined
  }));
}

type ReportRow = {
  pair_id: string;
  action: PlannedAction["kind"];
  canonical_uid: string;
  reason: string;
  title: string;
  google_present: string;
  icloud_present: string;
  google_title: string;
  icloud_title: string;
  google_start: string;
  icloud_start: string;
  google_end: string;
  icloud_end: string;
  google_status: string;
  icloud_status: string;
  google_hash: string;
  icloud_hash: string;
  diff_fields: string;
};

function actionToReportRow(pairId: string, action: PlannedAction): ReportRow {
  const google = action.google ?? (action.kind !== "conflict" ? undefined : action.google);
  const icloud = action.icloud ?? (action.kind !== "conflict" ? undefined : action.icloud);
  const event = action.kind !== "snapshot" && action.kind !== "conflict" ? action.event : google ?? icloud;
  const googleHash = google ? hashCanonicalEvent(google) : "";
  const icloudHash = icloud ? hashCanonicalEvent(icloud) : "";

  return {
    pair_id: pairId,
    action: action.kind,
    canonical_uid: action.canonicalUid,
    reason: action.kind === "conflict" ? action.reason : "",
    title: event?.title ?? "",
    google_present: google ? "yes" : "no",
    icloud_present: icloud ? "yes" : "no",
    google_title: google?.title ?? "",
    icloud_title: icloud?.title ?? "",
    google_start: formatEventDate(google?.start),
    icloud_start: formatEventDate(icloud?.start),
    google_end: formatEventDate(google?.end),
    icloud_end: formatEventDate(icloud?.end),
    google_status: google?.status ?? "",
    icloud_status: icloud?.status ?? "",
    google_hash: googleHash,
    icloud_hash: icloudHash,
    diff_fields: google && icloud ? diffEventFields(google, icloud).join("|") : ""
  };
}

function writeCsvReport(path: string, rows: ReportRow[]): void {
  mkdirSync(dirname(path), { recursive: true });
  const headers = [
    "pair_id",
    "action",
    "canonical_uid",
    "reason",
    "title",
    "google_present",
    "icloud_present",
    "google_title",
    "icloud_title",
    "google_start",
    "icloud_start",
    "google_end",
    "icloud_end",
    "google_status",
    "icloud_status",
    "google_hash",
    "icloud_hash",
    "diff_fields"
  ] satisfies Array<keyof ReportRow>;
  const lines = [
    headers.join(","),
    ...rows.map((row) => headers.map((header) => csvEscape(row[header])).join(","))
  ];
  writeFileSync(path, `${lines.join("\n")}\n`);
}

function csvEscape(value: string): string {
  if (!/[",\n\r]/.test(value)) {
    return value;
  }

  return `"${value.replace(/"/g, '""')}"`;
}

function formatEventDate(value: CanonicalEvent["start"] | undefined): string {
  if (!value) {
    return "";
  }

  return value.kind === "date"
    ? value.value
    : `${value.value}${value.timezone ? ` [${value.timezone}]` : ""}`;
}

function diffEventFields(google: CanonicalEvent, icloud: CanonicalEvent): string[] {
  const googleCore = reportComparable(google);
  const icloudCore = reportComparable(icloud);

  return Object.keys(googleCore).filter(
    (field) =>
      JSON.stringify(googleCore[field as keyof typeof googleCore]) !==
      JSON.stringify(icloudCore[field as keyof typeof icloudCore])
  );
}

function reportComparable(event: CanonicalEvent) {
  return {
    title: normalizeText(event.title),
    description: normalizeText(event.description),
    location: normalizeText(event.location).replace(/\\n/g, "\n"),
    status: event.status,
    start: normalizeReportDate(event.start),
    end: normalizeReportDate(event.end),
    recurrence: {
      rrule: event.recurrence?.rrule ?? "",
      exdates: event.recurrence?.exdates?.filter(Boolean).sort() ?? [],
      recurrenceId: event.recurrence?.recurrenceId ?? ""
    }
  };
}

function normalizeText(value: string | undefined): string {
  return (value ?? "").replace(/\r\n/g, "\n").trim();
}

function normalizeReportDate(value: CanonicalEvent["start"]) {
  if (value.kind === "date") {
    return value;
  }

  return {
    kind: "dateTime",
    instant: new Date(value.value).toISOString()
  };
}

type PlannedAction =
  | SnapshotAction
  | MutatingAction
    | {
        kind: "conflict";
        canonicalUid: string;
        reason: string;
      link?: EventLink | undefined;
      google?: CanonicalEvent | undefined;
      icloud?: CanonicalEvent | undefined;
    };

type SnapshotAction = {
  kind: "snapshot";
  canonicalUid: string;
  link?: EventLink | undefined;
  google?: CanonicalEvent | undefined;
  icloud?: CanonicalEvent | undefined;
  googleHash?: string | undefined;
  icloudHash?: string | undefined;
  googleMeta?: ProviderEventMeta | undefined;
  icloudMeta?: ProviderEventMeta | undefined;
};

type MutatingAction = {
  kind:
    | "create_icloud"
    | "create_google"
    | "update_icloud"
    | "update_google"
    | "delete_icloud"
    | "delete_google";
  canonicalUid: string;
  event: CanonicalEvent;
  link?: EventLink | undefined;
  google?: CanonicalEvent | undefined;
  icloud?: CanonicalEvent | undefined;
  googleHash?: string | undefined;
  icloudHash?: string | undefined;
  googleMeta?: ProviderEventMeta | undefined;
  icloudMeta?: ProviderEventMeta | undefined;
};

function planTwoWayActions(input: {
  links: EventLink[];
  googleEvents: CanonicalEvent[];
  icloudEvents: CanonicalEvent[];
  knownICloudUidCollisions?: Set<string>;
  direction: ResolvedServiceConfig["pairs"][number]["direction"];
  conflictPolicy: ResolvedServiceConfig["conflictPolicy"];
}): PlannedAction[] {
  const googleByUid = currentEventsByUid(input.googleEvents);
  const icloudByUid = currentEventsByUid(input.icloudEvents);
  const linksByUid = new Map(input.links.map((link) => [link.canonical_uid, link]));
  const uids = new Set([...googleByUid.keys(), ...icloudByUid.keys(), ...linksByUid.keys()]);
  const actions: PlannedAction[] = [];

  for (const uid of uids) {
    const link = linksByUid.get(uid);
    const google = googleByUid.get(uid);
    const icloud = icloudByUid.get(uid);
    const googleHash = google ? hashCanonicalEvent(google) : undefined;
    const icloudHash = icloud ? hashCanonicalEvent(icloud) : undefined;
    const googleChanged = googleHash !== (link?.google_hash ?? undefined);
    const icloudChanged = icloudHash !== (link?.icloud_hash ?? undefined);
    const googleDeleted = !google && Boolean(link?.google_event_id || link?.google_ical_uid);
    const icloudDeleted = !icloud && Boolean(link?.icloud_href || link?.icloud_uid);

    if (input.knownICloudUidCollisions?.has(uid) && google && !icloud) {
      actions.push({
        kind: "conflict",
        canonicalUid: uid,
        reason: "icloud_uid_exists_in_different_calendar",
        link,
        google
      });
      continue;
    }

    if (!link) {
      actions.push(...planUnlinked(uid, google, icloud, input.direction));
      continue;
    }

    if (google && icloud) {
      if (googleHash === icloudHash) {
        actions.push({ kind: "snapshot", canonicalUid: uid, link, google, icloud, googleHash, icloudHash });
      } else if (googleChanged && !icloudChanged && canWriteICloud(input.direction)) {
        actions.push({ kind: "update_icloud", canonicalUid: uid, event: google, link, google, icloud, googleHash, icloudHash });
      } else if (!googleChanged && icloudChanged && canWriteGoogle(input.direction)) {
        actions.push({ kind: "update_google", canonicalUid: uid, event: icloud, link, google, icloud, googleHash, icloudHash });
      } else if (googleChanged && icloudChanged) {
        actions.push(resolveConflict(uid, link, google, icloud, input.conflictPolicy));
      } else {
        actions.push({ kind: "snapshot", canonicalUid: uid, link, google, icloud, googleHash, icloudHash });
      }
      continue;
    }

    if (google && icloudDeleted) {
      if (googleChanged) {
        actions.push({ kind: "conflict", canonicalUid: uid, reason: "google_changed_while_icloud_deleted", link, google });
      } else if (canWriteGoogle(input.direction)) {
        actions.push({ kind: "delete_google", canonicalUid: uid, event: google, link, google, googleHash });
      } else if (canWriteICloud(input.direction)) {
        actions.push({ kind: "create_icloud", canonicalUid: uid, event: google, link, google, googleHash });
      }
      continue;
    }

    if (icloud && googleDeleted) {
      if (icloudChanged) {
        actions.push({ kind: "conflict", canonicalUid: uid, reason: "icloud_changed_while_google_deleted", link, icloud });
      } else if (canWriteICloud(input.direction)) {
        actions.push({ kind: "delete_icloud", canonicalUid: uid, event: icloud, link, icloud, icloudHash });
      } else if (canWriteGoogle(input.direction)) {
        actions.push({ kind: "create_google", canonicalUid: uid, event: icloud, link, icloud, icloudHash });
      }
      continue;
    }

    if (google && !icloud && canWriteICloud(input.direction)) {
      actions.push({ kind: "create_icloud", canonicalUid: uid, event: google, link, google, googleHash });
      continue;
    }

    if (icloud && !google && canWriteGoogle(input.direction)) {
      actions.push({ kind: "create_google", canonicalUid: uid, event: icloud, link, icloud, icloudHash });
      continue;
    }

    actions.push({ kind: "snapshot", canonicalUid: uid, link, google, icloud, googleHash, icloudHash });
  }

  return actions;
}

function planUnlinked(
  uid: string,
  google: CanonicalEvent | undefined,
  icloud: CanonicalEvent | undefined,
  direction: ResolvedServiceConfig["pairs"][number]["direction"]
): PlannedAction[] {
  if (google && icloud) {
    const googleHash = hashCanonicalEvent(google);
    const icloudHash = hashCanonicalEvent(icloud);
    if (googleHash === icloudHash) {
      return [{ kind: "snapshot", canonicalUid: uid, google, icloud, googleHash, icloudHash }];
    }
    return [{ kind: "conflict", canonicalUid: uid, reason: "unlinked_events_have_same_uid_but_differ", google, icloud }];
  }

  if (google && canWriteICloud(direction)) {
    return [{ kind: "create_icloud", canonicalUid: uid, event: google, google, googleHash: hashCanonicalEvent(google) }];
  }

  if (icloud && canWriteGoogle(direction)) {
    return [{ kind: "create_google", canonicalUid: uid, event: icloud, icloud, icloudHash: hashCanonicalEvent(icloud) }];
  }

  return [{ kind: "snapshot", canonicalUid: uid, google, icloud }];
}

function resolveConflict(
  uid: string,
  link: EventLink,
  google: CanonicalEvent,
  icloud: CanonicalEvent,
  policy: ResolvedServiceConfig["conflictPolicy"]
): PlannedAction {
  if (policy === "google_wins") {
    return { kind: "update_icloud", canonicalUid: uid, event: google, link, google, icloud };
  }
  if (policy === "icloud_wins") {
    return { kind: "update_google", canonicalUid: uid, event: icloud, link, google, icloud };
  }
  if (policy === "newest_updated_wins") {
    const googleUpdated = Date.parse(google.providerMeta.updatedAt ?? "");
    const icloudUpdated = Date.parse(icloud.providerMeta.updatedAt ?? "");
    if (Number.isFinite(googleUpdated) && googleUpdated > icloudUpdated) {
      return { kind: "update_icloud", canonicalUid: uid, event: google, link, google, icloud };
    }
    if (Number.isFinite(icloudUpdated) && icloudUpdated > googleUpdated) {
      return { kind: "update_google", canonicalUid: uid, event: icloud, link, google, icloud };
    }
  }
  return { kind: "conflict", canonicalUid: uid, reason: "both_sides_changed", link, google, icloud };
}

function currentEventsByUid(events: CanonicalEvent[]): Map<string, CanonicalEvent> {
  return new Map(
    events
      .filter((event) => !event.providerMeta.deleted)
      .map((event) => [event.canonicalUid, event])
  );
}

function snapshotUpsert(
  syncPairId: string,
  action: SnapshotAction | MutatingAction
): Parameters<typeof upsertEventLink>[1] {
  const google = action.google;
  const icloud = action.icloud;
  const googleMeta = action.googleMeta ?? google?.providerMeta;
  const icloudMeta = action.icloudMeta ?? icloud?.providerMeta;
  const googleHash =
    action.googleHash ?? (google && !google.providerMeta.deleted ? hashCanonicalEvent(google) : undefined);
  const icloudHash =
    action.icloudHash ?? (icloud && !icloud.providerMeta.deleted ? hashCanonicalEvent(icloud) : undefined);

  return {
    syncPairId,
    canonicalUid: action.canonicalUid,
    googleEventId: googleMeta?.eventId,
    googleICalUid: googleMeta?.iCalUid,
    googleEtag: googleMeta?.etag,
    icloudHref: icloudMeta?.href,
    icloudUid: icloudMeta?.iCalUid,
    icloudEtag: icloudMeta?.etag,
    googleHash,
    icloudHash,
    lastSyncedHash: googleHash === icloudHash ? googleHash : action.kind === "snapshot" ? undefined : googleHash ?? icloudHash,
    deletedGoogleAt: google ? null : undefined,
    deletedICloudAt: icloud ? null : undefined
  };
}

function canWriteGoogle(direction: ResolvedServiceConfig["pairs"][number]["direction"]): boolean {
  return direction === "two_way" || direction === "right_to_left";
}

function canWriteICloud(direction: ResolvedServiceConfig["pairs"][number]["direction"]): boolean {
  return direction === "two_way" || direction === "left_to_right";
}
