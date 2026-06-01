import { createHash } from "node:crypto";
import type { ResolvedServiceConfig } from "../config/service-config";
import type { ProviderName } from "../providers/types";
import type { AppDatabase } from "./database";

export function seedConfiguredPairs(db: AppDatabase, config: ResolvedServiceConfig): void {
  const insertAccount = db.query(`
    INSERT INTO accounts (id, provider, email)
    VALUES (?, ?, ?)
    ON CONFLICT(provider, email) DO UPDATE SET
      updated_at = CURRENT_TIMESTAMP
  `);

  const insertCalendar = db.query(`
    INSERT INTO calendars (id, account_id, provider_calendar_id, name)
    VALUES (?, ?, ?, ?)
    ON CONFLICT(account_id, provider_calendar_id) DO UPDATE SET
      name = excluded.name,
      updated_at = CURRENT_TIMESTAMP
  `);

  const insertSyncState = db.query(`
    INSERT INTO sync_state (calendar_id)
    VALUES (?)
    ON CONFLICT(calendar_id) DO NOTHING
  `);

  const insertPair = db.query(`
    INSERT INTO sync_pairs (
      id,
      left_calendar_id,
      right_calendar_id,
      direction,
      enabled,
      conflict_policy
    )
    VALUES (?, ?, ?, ?, ?, ?)
    ON CONFLICT(id) DO UPDATE SET
      left_calendar_id = excluded.left_calendar_id,
      right_calendar_id = excluded.right_calendar_id,
      direction = excluded.direction,
      enabled = excluded.enabled,
      conflict_policy = excluded.conflict_policy,
      updated_at = CURRENT_TIMESTAMP
  `);

  db.transaction(() => {
    for (const pair of config.pairs) {
      const googleAccountId = stableId("account", "google", pair.google.accountEmail);
      const icloudAccountId = stableId("account", "icloud", pair.icloud.accountEmail);
      const googleCalendarId = stableId(
        "calendar",
        "google",
        pair.google.accountEmail,
        pair.google.calendarId
      );
      const icloudCalendarId = stableId(
        "calendar",
        "icloud",
        pair.icloud.accountEmail,
        pair.icloud.calendarPath
      );

      insertAccount.run(googleAccountId, "google", pair.google.accountEmail);
      insertAccount.run(icloudAccountId, "icloud", pair.icloud.accountEmail);
      insertCalendar.run(
        googleCalendarId,
        googleAccountId,
        pair.google.calendarId,
        pair.google.calendarId
      );
      insertCalendar.run(
        icloudCalendarId,
        icloudAccountId,
        pair.icloud.calendarPath,
        pair.icloud.calendarPath
      );
      insertSyncState.run(googleCalendarId);
      insertSyncState.run(icloudCalendarId);
      insertPair.run(
        pair.id,
        googleCalendarId,
        icloudCalendarId,
        pair.direction,
        pair.enabled ? 1 : 0,
        config.conflictPolicy
      );
    }
  })();
}

export function stableId(...parts: string[]): string {
  return createHash("sha256").update(parts.join("\0")).digest("hex").slice(0, 24);
}

export type EventLink = {
  id: string;
  sync_pair_id: string;
  canonical_uid: string;
  google_event_id: string | null;
  google_ical_uid: string | null;
  google_etag: string | null;
  icloud_href: string | null;
  icloud_uid: string | null;
  icloud_etag: string | null;
  google_hash: string | null;
  icloud_hash: string | null;
  last_synced_hash: string | null;
  deleted_google_at: string | null;
  deleted_icloud_at: string | null;
};

export type CalendarIds = {
  googleAccountId: string;
  icloudAccountId: string;
  googleCalendarId: string;
  icloudCalendarId: string;
};

export function configuredCalendarIds(
  pair: ResolvedServiceConfig["pairs"][number]
): CalendarIds {
  return {
    googleAccountId: stableId("account", "google", pair.google.accountEmail),
    icloudAccountId: stableId("account", "icloud", pair.icloud.accountEmail),
    googleCalendarId: stableId(
      "calendar",
      "google",
      pair.google.accountEmail,
      pair.google.calendarId
    ),
    icloudCalendarId: stableId(
      "calendar",
      "icloud",
      pair.icloud.accountEmail,
      pair.icloud.calendarPath
    )
  };
}

export function loadEventLinks(db: AppDatabase, syncPairId: string): EventLink[] {
  return db
    .query<EventLink, [string]>(
      `SELECT
        id,
        sync_pair_id,
        canonical_uid,
        google_event_id,
        google_ical_uid,
        google_etag,
        icloud_href,
        icloud_uid,
        icloud_etag,
        google_hash,
        icloud_hash,
        last_synced_hash,
        deleted_google_at,
        deleted_icloud_at
      FROM event_links
      WHERE sync_pair_id = ?`
    )
    .all(syncPairId);
}

export type EventLinkUpsert = {
  syncPairId: string;
  canonicalUid: string;
  googleEventId?: string | null | undefined;
  googleICalUid?: string | null | undefined;
  googleEtag?: string | null | undefined;
  icloudHref?: string | null | undefined;
  icloudUid?: string | null | undefined;
  icloudEtag?: string | null | undefined;
  googleHash?: string | null | undefined;
  icloudHash?: string | null | undefined;
  lastSyncedHash?: string | null | undefined;
  deletedGoogleAt?: string | null | undefined;
  deletedICloudAt?: string | null | undefined;
};

export function upsertEventLink(db: AppDatabase, input: EventLinkUpsert): void {
  const id = stableId("event-link", input.syncPairId, input.canonicalUid);
  db.query(
    `INSERT INTO event_links (
      id,
      sync_pair_id,
      canonical_uid,
      google_event_id,
      google_ical_uid,
      google_etag,
      icloud_href,
      icloud_uid,
      icloud_etag,
      google_hash,
      icloud_hash,
      last_synced_hash,
      deleted_google_at,
      deleted_icloud_at
    )
    VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
    ON CONFLICT(sync_pair_id, canonical_uid) DO UPDATE SET
      google_event_id = COALESCE(excluded.google_event_id, event_links.google_event_id),
      google_ical_uid = COALESCE(excluded.google_ical_uid, event_links.google_ical_uid),
      google_etag = COALESCE(excluded.google_etag, event_links.google_etag),
      icloud_href = COALESCE(excluded.icloud_href, event_links.icloud_href),
      icloud_uid = COALESCE(excluded.icloud_uid, event_links.icloud_uid),
      icloud_etag = COALESCE(excluded.icloud_etag, event_links.icloud_etag),
      google_hash = COALESCE(excluded.google_hash, event_links.google_hash),
      icloud_hash = COALESCE(excluded.icloud_hash, event_links.icloud_hash),
      last_synced_hash = COALESCE(excluded.last_synced_hash, event_links.last_synced_hash),
      deleted_google_at = excluded.deleted_google_at,
      deleted_icloud_at = excluded.deleted_icloud_at,
      updated_at = CURRENT_TIMESTAMP`
  ).run(
    id,
    input.syncPairId,
    input.canonicalUid,
    input.googleEventId ?? null,
    input.googleICalUid ?? null,
    input.googleEtag ?? null,
    input.icloudHref ?? null,
    input.icloudUid ?? null,
    input.icloudEtag ?? null,
    input.googleHash ?? null,
    input.icloudHash ?? null,
    input.lastSyncedHash ?? null,
    input.deletedGoogleAt ?? null,
    input.deletedICloudAt ?? null
  );
}

export function updateCalendarSyncToken(
  db: AppDatabase,
  calendarId: string,
  syncToken: string | undefined
): void {
  if (!syncToken) {
    return;
  }

  db.query(
    `INSERT INTO sync_state (
      calendar_id,
      provider_sync_token,
      last_incremental_sync_at,
      updated_at
    )
    VALUES (?, ?, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)
    ON CONFLICT(calendar_id) DO UPDATE SET
      provider_sync_token = excluded.provider_sync_token,
      last_incremental_sync_at = CURRENT_TIMESTAMP,
      updated_at = CURRENT_TIMESTAMP`
  ).run(calendarId, syncToken);
}

export function loadCalendarSyncToken(
  db: AppDatabase,
  calendarId: string
): string | undefined {
  const row = db
    .query<{ provider_sync_token: string | null }, [string]>(
      "SELECT provider_sync_token FROM sync_state WHERE calendar_id = ?"
    )
    .get(calendarId);
  return row?.provider_sync_token ?? undefined;
}

export function recordConflict(
  db: AppDatabase,
  input: {
    syncPairId: string;
    eventLinkId?: string | undefined;
    canonicalUid: string;
    reason: string;
    googleSnapshot?: unknown | undefined;
    icloudSnapshot?: unknown | undefined;
  }
): void {
  db.query(
    `INSERT INTO sync_conflicts (
      id,
      sync_pair_id,
      event_link_id,
      canonical_uid,
      reason,
      google_snapshot,
      icloud_snapshot
    )
    VALUES (?, ?, ?, ?, ?, ?, ?)`
  ).run(
    stableId("conflict", input.syncPairId, input.canonicalUid, Date.now().toString()),
    input.syncPairId,
    input.eventLinkId ?? null,
    input.canonicalUid,
    input.reason,
    input.googleSnapshot ? JSON.stringify(input.googleSnapshot) : null,
    input.icloudSnapshot ? JSON.stringify(input.icloudSnapshot) : null
  );
}

export function loadUnresolvedConflictUids(
  db: AppDatabase,
  syncPairId: string,
  reason: string
): Set<string> {
  const rows = db
    .query<{ canonical_uid: string | null }, [string, string]>(
      `SELECT DISTINCT canonical_uid
      FROM sync_conflicts
      WHERE sync_pair_id = ?
        AND reason = ?
        AND resolved_at IS NULL
        AND canonical_uid IS NOT NULL`
    )
    .all(syncPairId, reason);

  return new Set(rows.flatMap((row) => (row.canonical_uid ? [row.canonical_uid] : [])));
}

export function accountCredentialEnv(provider: ProviderName): string[] {
  return provider === "google"
    ? ["GOOGLE_CLIENT_ID", "GOOGLE_CLIENT_SECRET", "GOOGLE_REFRESH_TOKEN"]
    : ["ICLOUD_USERNAME", "ICLOUD_APP_SPECIFIC_PASSWORD"];
}
