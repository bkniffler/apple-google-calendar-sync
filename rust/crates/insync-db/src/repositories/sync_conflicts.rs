use crate::DbError;
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::Value;
use std::collections::HashSet;
use uuid::Uuid;

#[derive(Debug, Clone, Default)]
pub struct RecordConflictInput {
    pub sync_pair_id: String,
    pub event_link_id: Option<String>,
    pub canonical_uid: String,
    pub reason: String,
    pub google_snapshot: Option<Value>,
    pub icloud_snapshot: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnresolvedConflictSummary {
    pub sync_pair_id: String,
    pub reason: String,
    pub count: i64,
    pub first_seen_at: String,
    pub last_seen_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnresolvedConflictRow {
    pub id: String,
    pub sync_pair_id: String,
    pub canonical_uid: Option<String>,
    pub reason: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Default)]
pub struct ConflictFilter {
    pub sync_pair_id: Option<String>,
    pub reason: Option<String>,
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveConflict {
    pub canonical_uid: String,
    pub reason: String,
}

pub fn record_conflict(conn: &Connection, input: RecordConflictInput) -> Result<(), DbError> {
    let existing = conn
        .query_row(
            r#"
            SELECT id
            FROM sync_conflicts
            WHERE sync_pair_id = ?
              AND canonical_uid = ?
              AND reason = ?
              AND resolved_at IS NULL
            LIMIT 1
            "#,
            params![input.sync_pair_id, input.canonical_uid, input.reason],
            |row| row.get::<_, String>(0),
        )
        .optional()?;

    if existing.is_some() {
        return Ok(());
    }

    conn.execute(
        r#"
        INSERT INTO sync_conflicts (
          id,
          sync_pair_id,
          event_link_id,
          canonical_uid,
          reason,
          google_snapshot,
          icloud_snapshot
        )
        VALUES (?, ?, ?, ?, ?, ?, ?)
        "#,
        params![
            Uuid::new_v4().to_string(),
            input.sync_pair_id,
            input.event_link_id,
            input.canonical_uid,
            input.reason,
            input.google_snapshot.map(|value| value.to_string()),
            input.icloud_snapshot.map(|value| value.to_string()),
        ],
    )?;

    Ok(())
}

pub fn load_unresolved_conflict_uids(
    conn: &Connection,
    sync_pair_id: &str,
    reason: &str,
) -> Result<HashSet<String>, DbError> {
    let mut statement = conn.prepare(
        r#"
        SELECT DISTINCT canonical_uid
        FROM sync_conflicts
        WHERE sync_pair_id = ?
          AND reason = ?
          AND resolved_at IS NULL
          AND canonical_uid IS NOT NULL
        "#,
    )?;

    let rows = statement
        .query_map(params![sync_pair_id, reason], |row| row.get::<_, String>(0))?
        .collect::<Result<HashSet<_>, _>>()?;

    Ok(rows)
}

pub fn list_unresolved_conflict_summaries(
    conn: &Connection,
) -> Result<Vec<UnresolvedConflictSummary>, DbError> {
    let mut statement = conn.prepare(
        r#"
        SELECT
          sync_pair_id,
          reason,
          COUNT(*) AS count,
          MIN(created_at) AS first_seen_at,
          MAX(created_at) AS last_seen_at
        FROM sync_conflicts
        WHERE resolved_at IS NULL
        GROUP BY sync_pair_id, reason
        ORDER BY sync_pair_id, reason
        "#,
    )?;

    let rows = statement
        .query_map([], |row| {
            Ok(UnresolvedConflictSummary {
                sync_pair_id: row.get("sync_pair_id")?,
                reason: row.get("reason")?,
                count: row.get("count")?,
                first_seen_at: row.get("first_seen_at")?,
                last_seen_at: row.get("last_seen_at")?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(rows)
}

pub fn list_unresolved_conflicts(
    conn: &Connection,
    filter: ConflictFilter,
) -> Result<Vec<UnresolvedConflictRow>, DbError> {
    let limit = i64::from(filter.limit.unwrap_or(100));

    match (filter.sync_pair_id, filter.reason) {
        (Some(sync_pair_id), Some(reason)) => query_conflicts(
            conn,
            "resolved_at IS NULL AND sync_pair_id = ? AND reason = ?",
            params![sync_pair_id, reason, limit],
        ),
        (Some(sync_pair_id), None) => query_conflicts(
            conn,
            "resolved_at IS NULL AND sync_pair_id = ?",
            params![sync_pair_id, limit],
        ),
        (None, Some(reason)) => query_conflicts(
            conn,
            "resolved_at IS NULL AND reason = ?",
            params![reason, limit],
        ),
        (None, None) => query_conflicts(conn, "resolved_at IS NULL", params![limit]),
    }
}

pub fn dedupe_unresolved_conflicts(conn: &Connection) -> Result<usize, DbError> {
    let rows = {
        let mut statement = conn.prepare(
            r#"
            SELECT id
            FROM (
              SELECT
                id,
                ROW_NUMBER() OVER (
                  PARTITION BY sync_pair_id, canonical_uid, reason
                  ORDER BY created_at ASC, id ASC
                ) AS duplicate_rank
              FROM sync_conflicts
              WHERE resolved_at IS NULL
            )
            WHERE duplicate_rank > 1
            "#,
        )?;

        statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?
    };

    for id in &rows {
        conn.execute(
            "UPDATE sync_conflicts SET resolved_at = CURRENT_TIMESTAMP WHERE id = ?",
            params![id],
        )?;
    }

    Ok(rows.len())
}

pub fn resolve_stale_conflicts(
    conn: &Connection,
    sync_pair_id: &str,
    active: &[ActiveConflict],
) -> Result<usize, DbError> {
    let rows = {
        let mut statement = conn.prepare(
            r#"
            SELECT id, canonical_uid, reason
            FROM sync_conflicts
            WHERE sync_pair_id = ?
              AND resolved_at IS NULL
            "#,
        )?;

        statement
            .query_map(params![sync_pair_id], |row| {
                Ok((
                    row.get::<_, String>("id")?,
                    row.get::<_, Option<String>>("canonical_uid")?,
                    row.get::<_, String>("reason")?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?
    };

    let active = active
        .iter()
        .map(|item| format!("{}\0{}", item.canonical_uid, item.reason))
        .collect::<HashSet<_>>();
    let stale = rows
        .into_iter()
        .filter(|(_, canonical_uid, reason)| {
            canonical_uid
                .as_ref()
                .map(|uid| !active.contains(&format!("{uid}\0{reason}")))
                .unwrap_or(true)
        })
        .collect::<Vec<_>>();

    for (id, _, _) in &stale {
        conn.execute(
            "UPDATE sync_conflicts SET resolved_at = CURRENT_TIMESTAMP WHERE id = ?",
            params![id],
        )?;
    }

    Ok(stale.len())
}

fn query_conflicts<P>(
    conn: &Connection,
    where_clause: &str,
    params: P,
) -> Result<Vec<UnresolvedConflictRow>, DbError>
where
    P: rusqlite::Params,
{
    let sql = format!(
        r#"
        SELECT
          id,
          sync_pair_id,
          canonical_uid,
          reason,
          created_at
        FROM sync_conflicts
        WHERE {where_clause}
        ORDER BY created_at DESC
        LIMIT ?
        "#
    );

    let mut statement = conn.prepare(&sql)?;
    let rows = statement
        .query_map(params, |row| {
            Ok(UnresolvedConflictRow {
                id: row.get("id")?,
                sync_pair_id: row.get("sync_pair_id")?,
                canonical_uid: row.get("canonical_uid")?,
                reason: row.get("reason")?,
                created_at: row.get("created_at")?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{migrate, open_in_memory};

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

    fn conflict(uid: &str, reason: &str) -> RecordConflictInput {
        RecordConflictInput {
            sync_pair_id: "personal".to_string(),
            canonical_uid: uid.to_string(),
            reason: reason.to_string(),
            google_snapshot: Some(serde_json::json!({ "title": "Google" })),
            ..RecordConflictInput::default()
        }
    }

    #[test]
    fn record_conflict_skips_existing_unresolved_duplicate() {
        let conn = conn();

        record_conflict(&conn, conflict("uid-1", "both_sides_changed")).unwrap();
        record_conflict(&conn, conflict("uid-1", "both_sides_changed")).unwrap();

        let rows = list_unresolved_conflicts(&conn, ConflictFilter::default()).unwrap();
        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn summarizes_and_filters_unresolved_conflicts() {
        let conn = conn();

        record_conflict(&conn, conflict("uid-1", "both_sides_changed")).unwrap();
        record_conflict(
            &conn,
            conflict("uid-2", "icloud_uid_exists_in_different_calendar"),
        )
        .unwrap();

        let summaries = list_unresolved_conflict_summaries(&conn).unwrap();
        assert_eq!(summaries.len(), 2);

        let filtered = list_unresolved_conflicts(
            &conn,
            ConflictFilter {
                reason: Some("both_sides_changed".to_string()),
                ..ConflictFilter::default()
            },
        )
        .unwrap();

        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].canonical_uid.as_deref(), Some("uid-1"));
    }

    #[test]
    fn loads_unresolved_conflict_uids() {
        let conn = conn();

        record_conflict(
            &conn,
            conflict("uid-1", "icloud_uid_exists_in_different_calendar"),
        )
        .unwrap();

        let uids = load_unresolved_conflict_uids(
            &conn,
            "personal",
            "icloud_uid_exists_in_different_calendar",
        )
        .unwrap();

        assert!(uids.contains("uid-1"));
    }

    #[test]
    fn resolves_stale_conflicts() {
        let conn = conn();

        record_conflict(&conn, conflict("uid-1", "both_sides_changed")).unwrap();
        record_conflict(&conn, conflict("uid-2", "both_sides_changed")).unwrap();

        let resolved = resolve_stale_conflicts(
            &conn,
            "personal",
            &[ActiveConflict {
                canonical_uid: "uid-1".to_string(),
                reason: "both_sides_changed".to_string(),
            }],
        )
        .unwrap();

        assert_eq!(resolved, 1);

        let rows = list_unresolved_conflicts(&conn, ConflictFilter::default()).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].canonical_uid.as_deref(), Some("uid-1"));
    }
}
