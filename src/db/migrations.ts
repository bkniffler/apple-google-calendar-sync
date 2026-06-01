import type { AppDatabase } from "./database";

type Migration = {
  id: number;
  name: string;
  sql: string;
};

const migrations: Migration[] = [
  {
    id: 1,
    name: "initial_sync_ledger",
    sql: `
      CREATE TABLE IF NOT EXISTS schema_migrations (
        id INTEGER PRIMARY KEY,
        name TEXT NOT NULL,
        applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
      );

      CREATE TABLE accounts (
        id TEXT PRIMARY KEY,
        provider TEXT NOT NULL CHECK (provider IN ('google', 'icloud')),
        email TEXT NOT NULL,
        auth_json TEXT,
        created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
        updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
        UNIQUE (provider, email)
      );

      CREATE TABLE calendars (
        id TEXT PRIMARY KEY,
        account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
        provider_calendar_id TEXT NOT NULL,
        name TEXT,
        color TEXT,
        timezone TEXT,
        writable INTEGER NOT NULL DEFAULT 1,
        raw_json TEXT,
        created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
        updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
        UNIQUE (account_id, provider_calendar_id)
      );

      CREATE TABLE sync_pairs (
        id TEXT PRIMARY KEY,
        left_calendar_id TEXT NOT NULL REFERENCES calendars(id) ON DELETE CASCADE,
        right_calendar_id TEXT NOT NULL REFERENCES calendars(id) ON DELETE CASCADE,
        direction TEXT NOT NULL CHECK (direction IN ('two_way', 'left_to_right', 'right_to_left')),
        enabled INTEGER NOT NULL DEFAULT 1,
        conflict_policy TEXT NOT NULL CHECK (conflict_policy IN ('manual', 'google_wins', 'icloud_wins', 'newest_updated_wins')),
        created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
        updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
      );

      CREATE TABLE sync_state (
        calendar_id TEXT PRIMARY KEY REFERENCES calendars(id) ON DELETE CASCADE,
        provider_sync_token TEXT,
        last_full_sync_at TEXT,
        last_incremental_sync_at TEXT,
        updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
      );

      CREATE TABLE event_links (
        id TEXT PRIMARY KEY,
        sync_pair_id TEXT NOT NULL REFERENCES sync_pairs(id) ON DELETE CASCADE,
        canonical_uid TEXT NOT NULL,
        google_event_id TEXT,
        google_ical_uid TEXT,
        google_etag TEXT,
        icloud_href TEXT,
        icloud_uid TEXT,
        icloud_etag TEXT,
        google_hash TEXT,
        icloud_hash TEXT,
        last_synced_hash TEXT,
        deleted_google_at TEXT,
        deleted_icloud_at TEXT,
        created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
        updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
        UNIQUE (sync_pair_id, canonical_uid)
      );

      CREATE INDEX event_links_google_event_id_idx ON event_links (google_event_id);
      CREATE INDEX event_links_icloud_href_idx ON event_links (icloud_href);

      CREATE TABLE sync_runs (
        id TEXT PRIMARY KEY,
        sync_pair_id TEXT REFERENCES sync_pairs(id) ON DELETE SET NULL,
        status TEXT NOT NULL CHECK (status IN ('running', 'completed', 'failed')),
        started_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
        finished_at TEXT,
        error TEXT
      );

      CREATE TABLE sync_conflicts (
        id TEXT PRIMARY KEY,
        sync_pair_id TEXT NOT NULL REFERENCES sync_pairs(id) ON DELETE CASCADE,
        event_link_id TEXT REFERENCES event_links(id) ON DELETE SET NULL,
        canonical_uid TEXT,
        reason TEXT NOT NULL,
        google_snapshot TEXT,
        icloud_snapshot TEXT,
        resolved_at TEXT,
        created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
      );
    `
  }
];

export function migrate(db: AppDatabase): void {
  db.exec(`
    CREATE TABLE IF NOT EXISTS schema_migrations (
      id INTEGER PRIMARY KEY,
      name TEXT NOT NULL,
      applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
    );
  `);

  const appliedRows = db
    .query<{ id: number }, []>("SELECT id FROM schema_migrations")
    .all();
  const applied = new Set(appliedRows.map((row) => row.id));

  db.transaction(() => {
    for (const migration of migrations) {
      if (applied.has(migration.id)) {
        continue;
      }

      db.exec(migration.sql);
      db.query("INSERT INTO schema_migrations (id, name) VALUES (?, ?)").run(
        migration.id,
        migration.name
      );
    }
  })();
}
