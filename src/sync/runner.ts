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
  reportSnapshots?: boolean | undefined;
};

export class SyncRunner {
  constructor(private readonly options: SyncRunnerOptions) {}

  async runOnce(): Promise<void> {
    const { db, config, google, icloud, logger } = this.options;
    const dryRun = this.options.dryRun ?? true;
    const reportRows: ReportRow[] = [];
    seedConfiguredPairs(db, config);

    for (const pair of config.sync.pairs.filter((item) => item.enabled)) {
      logger.info({ pairId: pair.id }, "starting sync pair");
      const calendarIds = configuredCalendarIds(config, pair);

      const googleChanges = await google.getChanges(pair.googleCalendarId, {
        fullSync: true
      });
      const icloudChanges = await icloud.getChanges(pair.icloudCalendarId, {
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
        conflictPolicy: resolveConflictPolicies(config)
      });

      logger.info(
        {
          pairId: pair.id,
          googleEvents: googleChanges.events.length,
          icloudEvents: icloudChanges.events.length,
          actions: actions.length,
          actionCounts: countActions(actions),
          resolutionCounts: countResolutions(actions),
          samples: sampleActions(actions)
        },
        "planned sync actions"
      );

      reportRows.push(
        ...actions
          .filter((action) => this.options.reportSnapshots || action.kind !== "snapshot")
          .map((action) => actionToReportRow(pair.id, action))
      );

      for (const action of actions) {
        await this.applyAction(pair.id, pair.googleCalendarId, pair.icloudCalendarId, action);
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
    const dryRun = this.options.dryRun ?? true;

    if (!dryRun && action.kind !== "snapshot") {
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
      if (action.resolution === "ignored") {
        return;
      }

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

function countResolutions(actions: PlannedAction[]): Record<string, number> {
  const counts: Record<string, number> = {};
  for (const action of actions) {
    const resolution =
      action.kind === "conflict"
        ? `conflict_${action.resolution}`
        : action.resolution?.kind;
    if (!resolution) {
      continue;
    }
    counts[resolution] = (counts[resolution] ?? 0) + 1;
  }
  return counts;
}

function sampleActions(actions: PlannedAction[], limit = 20): Array<{
  kind: PlannedAction["kind"];
  canonicalUid: string;
  reason?: string | undefined;
  resolution?: string | undefined;
  policy?: string | undefined;
}> {
  const prioritized = [
    ...actions.filter((action) => action.kind !== "snapshot"),
    ...actions.filter((action) => action.kind === "snapshot")
  ];

  return prioritized.slice(0, limit).map((action) => ({
    kind: action.kind,
    canonicalUid: action.canonicalUid,
    reason: action.kind === "conflict" ? action.reason : action.resolution?.reason,
    resolution: action.kind === "conflict" ? action.resolution : action.resolution?.kind,
    policy: action.kind === "conflict" ? undefined : action.resolution?.policy
  }));
}

type ReportRow = {
  pair_id: string;
  action: PlannedAction["kind"];
  canonical_uid: string;
  reason: string;
  resolution: string;
  conflict_policy: string;
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
    reason: action.kind === "conflict" ? action.reason : action.resolution?.reason ?? "",
    resolution: action.kind === "conflict" ? action.resolution : action.resolution?.kind ?? "",
    conflict_policy: action.kind === "conflict" ? "" : action.resolution?.policy ?? "",
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
    "resolution",
    "conflict_policy",
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
  | ConflictAction;

type ConflictAction = {
  kind: "conflict";
  canonicalUid: string;
  reason: string;
  resolution: "manual" | "ignored";
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
  resolution?: AutoResolution | undefined;
};

type AutoResolution = {
  kind: "auto_resolved";
  reason: string;
  policy: string;
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
  resolution?: AutoResolution | undefined;
};

export function planTwoWayActions(input: {
  links: EventLink[];
  googleEvents: CanonicalEvent[];
  icloudEvents: CanonicalEvent[];
  knownICloudUidCollisions?: Set<string>;
  direction: ResolvedServiceConfig["sync"]["pairs"][number]["direction"];
  conflictPolicy: ResolvedConflictPolicies;
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
        resolution: input.conflictPolicy.icloudUidCollision === "ignore_known" ? "ignored" : "manual",
        link,
        google
      });
      continue;
    }

    if (!link) {
      actions.push(...planUnlinked(uid, google, icloud, input.direction, input.conflictPolicy.unlinkedSameUid));
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
        actions.push(resolveChangedConflict(
          uid,
          link,
          google,
          icloud,
          input.direction,
          input.conflictPolicy.bothSidesChanged
        ));
      } else {
        actions.push({ kind: "snapshot", canonicalUid: uid, link, google, icloud, googleHash, icloudHash });
      }
      continue;
    }

    if (google && icloudDeleted) {
      if (googleChanged) {
        actions.push(resolveDeleteUpdateConflict({
          uid,
          reason: "google_changed_while_icloud_deleted",
          deletedSide: "icloud",
          changedEvent: google,
          link,
          direction: input.direction,
          policy: input.conflictPolicy.deleteVsUpdate
        }));
      } else if (canWriteGoogle(input.direction)) {
        actions.push({ kind: "delete_google", canonicalUid: uid, event: google, link, google, googleHash });
      } else if (canWriteICloud(input.direction)) {
        actions.push({ kind: "create_icloud", canonicalUid: uid, event: google, link, google, googleHash });
      }
      continue;
    }

    if (icloud && googleDeleted) {
      if (icloudChanged) {
        actions.push(resolveDeleteUpdateConflict({
          uid,
          reason: "icloud_changed_while_google_deleted",
          deletedSide: "google",
          changedEvent: icloud,
          link,
          direction: input.direction,
          policy: input.conflictPolicy.deleteVsUpdate
        }));
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
  direction: ResolvedServiceConfig["sync"]["pairs"][number]["direction"],
  conflictPolicy: ResolvedConflictPolicies["unlinkedSameUid"]
): PlannedAction[] {
  if (google && icloud) {
    const googleHash = hashCanonicalEvent(google);
    const icloudHash = hashCanonicalEvent(icloud);
    if (googleHash === icloudHash) {
      return [{ kind: "snapshot", canonicalUid: uid, google, icloud, googleHash, icloudHash }];
    }
    return [
      resolveUnlinkedSameUidConflict(uid, google, icloud, direction, conflictPolicy)
    ];
  }

  if (google && canWriteICloud(direction)) {
    return [{ kind: "create_icloud", canonicalUid: uid, event: google, google, googleHash: hashCanonicalEvent(google) }];
  }

  if (icloud && canWriteGoogle(direction)) {
    return [{ kind: "create_google", canonicalUid: uid, event: icloud, icloud, icloudHash: hashCanonicalEvent(icloud) }];
  }

  return [{ kind: "snapshot", canonicalUid: uid, google, icloud }];
}

type ResolvedConflictPolicies = {
  bothSidesChanged: ResolvedServiceConfig["sync"]["conflictPolicy"];
  unlinkedSameUid: ResolvedServiceConfig["sync"]["conflictPolicy"];
  deleteVsUpdate: ResolvedServiceConfig["sync"]["conflicts"]["deleteVsUpdate"];
  icloudUidCollision: ResolvedServiceConfig["sync"]["conflicts"]["icloudUidCollision"];
};

function resolveConflictPolicies(config: ResolvedServiceConfig): ResolvedConflictPolicies {
  const defaultPolicy = config.sync.conflicts.default ?? config.sync.conflictPolicy;
  return {
    bothSidesChanged: config.sync.conflicts.bothSidesChanged ?? defaultPolicy,
    unlinkedSameUid: config.sync.conflicts.unlinkedSameUid ?? defaultPolicy,
    deleteVsUpdate: config.sync.conflicts.deleteVsUpdate,
    icloudUidCollision: config.sync.conflicts.icloudUidCollision
  };
}

function resolveChangedConflict(
  uid: string,
  link: EventLink,
  google: CanonicalEvent,
  icloud: CanonicalEvent,
  direction: ResolvedServiceConfig["sync"]["pairs"][number]["direction"],
  policy: ResolvedServiceConfig["sync"]["conflictPolicy"]
): PlannedAction {
  return resolveProviderWinnerConflict({
    uid,
    reason: "both_sides_changed",
    link,
    google,
    icloud,
    policy,
    direction
  });
}

function resolveUnlinkedSameUidConflict(
  uid: string,
  google: CanonicalEvent,
  icloud: CanonicalEvent,
  direction: ResolvedServiceConfig["sync"]["pairs"][number]["direction"],
  policy: ResolvedServiceConfig["sync"]["conflictPolicy"]
): PlannedAction {
  return resolveProviderWinnerConflict({
    uid,
    reason: "unlinked_events_have_same_uid_but_differ",
    google,
    icloud,
    policy,
    direction
  });
}

function resolveProviderWinnerConflict(input: {
  uid: string;
  reason: string;
  link?: EventLink | undefined;
  google: CanonicalEvent;
  icloud: CanonicalEvent;
  policy: ResolvedServiceConfig["sync"]["conflictPolicy"];
  direction: ResolvedServiceConfig["sync"]["pairs"][number]["direction"];
}): PlannedAction {
  const winner = selectProviderWinner(input.google, input.icloud, input.policy);

  if (winner === "google" && canWriteICloud(input.direction)) {
    return autoResolvedAction({
      kind: "update_icloud",
      canonicalUid: input.uid,
      event: input.google,
      link: input.link,
      google: input.google,
      icloud: input.icloud,
      reason: input.reason,
      policy: input.policy
    });
  }

  if (winner === "icloud" && canWriteGoogle(input.direction)) {
    return autoResolvedAction({
      kind: "update_google",
      canonicalUid: input.uid,
      event: input.icloud,
      link: input.link,
      google: input.google,
      icloud: input.icloud,
      reason: input.reason,
      policy: input.policy
    });
  }

  return manualConflict(input.uid, input.reason, {
    link: input.link,
    google: input.google,
    icloud: input.icloud
  });
}

function resolveDeleteUpdateConflict(input: {
  uid: string;
  reason: "google_changed_while_icloud_deleted" | "icloud_changed_while_google_deleted";
  deletedSide: "google" | "icloud";
  changedEvent: CanonicalEvent;
  link?: EventLink | undefined;
  direction: ResolvedServiceConfig["sync"]["pairs"][number]["direction"];
  policy: ResolvedConflictPolicies["deleteVsUpdate"];
}): PlannedAction {
  if (input.policy === "manual") {
    return manualConflict(input.uid, input.reason, {
      link: input.link,
      google: input.deletedSide === "icloud" ? input.changedEvent : undefined,
      icloud: input.deletedSide === "google" ? input.changedEvent : undefined
    });
  }

  if (input.policy === "delete_wins") {
    if (input.deletedSide === "icloud" && canWriteGoogle(input.direction)) {
      return autoResolvedAction({
        kind: "delete_google",
        canonicalUid: input.uid,
        event: input.changedEvent,
        link: input.link,
        google: input.changedEvent,
        reason: input.reason,
        policy: input.policy
      });
    }

    if (input.deletedSide === "google" && canWriteICloud(input.direction)) {
      return autoResolvedAction({
        kind: "delete_icloud",
        canonicalUid: input.uid,
        event: input.changedEvent,
        link: input.link,
        icloud: input.changedEvent,
        reason: input.reason,
        policy: input.policy
      });
    }

    return manualConflict(input.uid, input.reason, {
      link: input.link,
      google: input.deletedSide === "icloud" ? input.changedEvent : undefined,
      icloud: input.deletedSide === "google" ? input.changedEvent : undefined
    });
  }

  if (input.deletedSide === "icloud" && canWriteICloud(input.direction)) {
    return autoResolvedAction({
      kind: "create_icloud",
      canonicalUid: input.uid,
      event: input.changedEvent,
      link: input.link,
      google: input.changedEvent,
      reason: input.reason,
      policy: input.policy
    });
  }

  if (input.deletedSide === "google" && canWriteGoogle(input.direction)) {
    return autoResolvedAction({
      kind: "create_google",
      canonicalUid: input.uid,
      event: input.changedEvent,
      link: input.link,
      icloud: input.changedEvent,
      reason: input.reason,
      policy: input.policy
    });
  }

  return manualConflict(input.uid, input.reason, {
    link: input.link,
    google: input.deletedSide === "icloud" ? input.changedEvent : undefined,
    icloud: input.deletedSide === "google" ? input.changedEvent : undefined
  });
}

function selectProviderWinner(
  google: CanonicalEvent,
  icloud: CanonicalEvent,
  policy: ResolvedServiceConfig["sync"]["conflictPolicy"]
): "google" | "icloud" | undefined {
  if (policy === "google_wins") {
    return "google";
  }
  if (policy === "icloud_wins") {
    return "icloud";
  }
  if (policy === "newest_updated_wins") {
    const googleUpdated = Date.parse(google.providerMeta.updatedAt ?? "");
    const icloudUpdated = Date.parse(icloud.providerMeta.updatedAt ?? "");
    if (Number.isFinite(googleUpdated) && Number.isFinite(icloudUpdated)) {
      if (googleUpdated > icloudUpdated) {
        return "google";
      }
      if (icloudUpdated > googleUpdated) {
        return "icloud";
      }
    }
  }
  return undefined;
}

function autoResolvedAction<T extends MutatingAction>(
  action: T & { reason: string; policy: string }
): T {
  (action as T & { resolution: AutoResolution }).resolution = {
    kind: "auto_resolved",
    reason: action.reason,
    policy: action.policy
  };
  delete (action as T & { reason?: string }).reason;
  delete (action as T & { policy?: string }).policy;
  return action;
}

function manualConflict(
  canonicalUid: string,
  reason: string,
  values: {
    link?: EventLink | undefined;
    google?: CanonicalEvent | undefined;
    icloud?: CanonicalEvent | undefined;
  }
): ConflictAction {
  return {
    kind: "conflict",
    canonicalUid,
    reason,
    resolution: "manual",
    ...values
  };
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

function canWriteGoogle(direction: ResolvedServiceConfig["sync"]["pairs"][number]["direction"]): boolean {
  return direction === "two_way" || direction === "right_to_left";
}

function canWriteICloud(direction: ResolvedServiceConfig["sync"]["pairs"][number]["direction"]): boolean {
  return direction === "two_way" || direction === "left_to_right";
}
