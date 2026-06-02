use crate::DbError;
use rusqlite::{Connection, params};

struct Migration {
    id: i64,
    name: &'static str,
    sql: &'static str,
}

const MIGRATIONS: &[Migration] = &[Migration {
    id: 1,
    name: "initial_sync_ledger",
    sql: r#"
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
    "#,
}];

pub fn migrate(conn: &Connection) -> Result<(), DbError> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS schema_migrations (
          id INTEGER PRIMARY KEY,
          name TEXT NOT NULL,
          applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        );
        "#,
    )?;

    let applied = {
        let mut statement = conn.prepare("SELECT id FROM schema_migrations")?;
        statement
            .query_map([], |row| row.get::<_, i64>(0))?
            .collect::<Result<Vec<_>, _>>()?
    };

    for migration in MIGRATIONS {
        if applied.contains(&migration.id) {
            continue;
        }

        conn.execute_batch(migration.sql)?;
        conn.execute(
            "INSERT INTO schema_migrations (id, name) VALUES (?, ?)",
            params![migration.id, migration.name],
        )?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::open_in_memory;

    #[test]
    fn migrates_empty_database_to_latest_schema() {
        let conn = open_in_memory().unwrap();
        migrate(&conn).unwrap();

        let tables = conn
            .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();

        assert!(tables.contains(&"accounts".to_string()));
        assert!(tables.contains(&"event_links".to_string()));
        assert!(tables.contains(&"sync_conflicts".to_string()));
        assert!(tables.contains(&"sync_runs".to_string()));
    }

    #[test]
    fn migration_is_idempotent() {
        let conn = open_in_memory().unwrap();
        migrate(&conn).unwrap();
        migrate(&conn).unwrap();

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM schema_migrations", [], |row| {
                row.get(0)
            })
            .unwrap();

        assert_eq!(count, 1);
    }
}
