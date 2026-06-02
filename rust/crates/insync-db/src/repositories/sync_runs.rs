use crate::DbError;
use rusqlite::{Connection, OptionalExtension, params};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncRunStatus {
    Running,
    Completed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncRun {
    pub id: String,
    pub sync_pair_id: Option<String>,
    pub status: SyncRunStatus,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub error: Option<String>,
}

pub fn start_sync_run(conn: &Connection, sync_pair_id: Option<&str>) -> Result<SyncRun, DbError> {
    let id = Uuid::new_v4().to_string();
    conn.execute(
        r#"
        INSERT INTO sync_runs (id, sync_pair_id, status)
        VALUES (?, ?, 'running')
        "#,
        params![id, sync_pair_id],
    )?;

    get_sync_run(conn, &id)?.ok_or_else(|| DbError::Sqlite(rusqlite::Error::QueryReturnedNoRows))
}

pub fn complete_sync_run(conn: &Connection, id: &str) -> Result<(), DbError> {
    conn.execute(
        r#"
        UPDATE sync_runs
        SET status = 'completed',
            finished_at = CURRENT_TIMESTAMP,
            error = NULL
        WHERE id = ?
        "#,
        params![id],
    )?;
    Ok(())
}

pub fn fail_sync_run(conn: &Connection, id: &str, error: &str) -> Result<(), DbError> {
    conn.execute(
        r#"
        UPDATE sync_runs
        SET status = 'failed',
            finished_at = CURRENT_TIMESTAMP,
            error = ?
        WHERE id = ?
        "#,
        params![error, id],
    )?;
    Ok(())
}

pub fn get_sync_run(conn: &Connection, id: &str) -> Result<Option<SyncRun>, DbError> {
    conn.query_row(
        r#"
        SELECT id, sync_pair_id, status, started_at, finished_at, error
        FROM sync_runs
        WHERE id = ?
        "#,
        params![id],
        sync_run_from_row,
    )
    .optional()
    .map_err(DbError::from)
}

pub fn latest_sync_run(conn: &Connection) -> Result<Option<SyncRun>, DbError> {
    conn.query_row(
        r#"
        SELECT id, sync_pair_id, status, started_at, finished_at, error
        FROM sync_runs
        ORDER BY started_at DESC, rowid DESC
        LIMIT 1
        "#,
        [],
        sync_run_from_row,
    )
    .optional()
    .map_err(DbError::from)
}

pub fn latest_sync_run_for_pair(
    conn: &Connection,
    sync_pair_id: &str,
) -> Result<Option<SyncRun>, DbError> {
    conn.query_row(
        r#"
        SELECT id, sync_pair_id, status, started_at, finished_at, error
        FROM sync_runs
        WHERE sync_pair_id = ?
        ORDER BY started_at DESC, rowid DESC
        LIMIT 1
        "#,
        params![sync_pair_id],
        sync_run_from_row,
    )
    .optional()
    .map_err(DbError::from)
}

pub fn recent_sync_runs(conn: &Connection, limit: u32) -> Result<Vec<SyncRun>, DbError> {
    let mut statement = conn.prepare(
        r#"
        SELECT id, sync_pair_id, status, started_at, finished_at, error
        FROM sync_runs
        ORDER BY started_at DESC, rowid DESC
        LIMIT ?
        "#,
    )?;

    let rows = statement
        .query_map(params![i64::from(limit)], sync_run_from_row)?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(rows)
}

fn sync_run_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SyncRun> {
    let status: String = row.get("status")?;
    Ok(SyncRun {
        id: row.get("id")?,
        sync_pair_id: row.get("sync_pair_id")?,
        status: match status.as_str() {
            "running" => SyncRunStatus::Running,
            "completed" => SyncRunStatus::Completed,
            "failed" => SyncRunStatus::Failed,
            _ => return Err(rusqlite::Error::InvalidQuery),
        },
        started_at: row.get("started_at")?,
        finished_at: row.get("finished_at")?,
        error: row.get("error")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{migrate, open_in_memory};

    #[test]
    fn records_completed_and_failed_runs() {
        let conn = conn();
        let completed = start_sync_run(&conn, Some("personal")).unwrap();
        complete_sync_run(&conn, &completed.id).unwrap();
        let failed = start_sync_run(&conn, Some("personal")).unwrap();
        fail_sync_run(&conn, &failed.id, "network failed").unwrap();

        let latest = latest_sync_run(&conn).unwrap().unwrap();
        assert_eq!(latest.id, failed.id);
        assert_eq!(latest.status, SyncRunStatus::Failed);
        assert_eq!(latest.error.as_deref(), Some("network failed"));

        let pair_latest = latest_sync_run_for_pair(&conn, "personal")
            .unwrap()
            .unwrap();
        assert_eq!(pair_latest.id, failed.id);
        let recent = recent_sync_runs(&conn, 10).unwrap();
        assert_eq!(recent.len(), 2);
    }

    #[test]
    fn latest_run_is_optional() {
        let conn = conn();
        assert_eq!(latest_sync_run(&conn).unwrap(), None);
    }

    fn conn() -> Connection {
        let conn = open_in_memory().unwrap();
        migrate(&conn).unwrap();
        seed_pair(&conn);
        conn
    }

    fn seed_pair(conn: &Connection) {
        conn.execute(
            "INSERT INTO accounts (id, provider, email) VALUES ('google-account', 'google', 'personal')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO accounts (id, provider, email) VALUES ('icloud-account', 'icloud', 'personal')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO calendars (id, account_id, provider_calendar_id, name) VALUES ('google-cal', 'google-account', 'primary', 'Primary')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO calendars (id, account_id, provider_calendar_id, name) VALUES ('icloud-cal', 'icloud-account', 'https://caldav.example/cal', 'Primary')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO sync_pairs (id, left_calendar_id, right_calendar_id, direction, conflict_policy) VALUES ('personal', 'google-cal', 'icloud-cal', 'two_way', 'manual')",
            [],
        )
        .unwrap();
    }
}
