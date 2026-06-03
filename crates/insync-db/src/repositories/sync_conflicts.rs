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
    pub event_link_id: Option<String>,
    pub canonical_uid: Option<String>,
    pub reason: String,
    pub manual_resolution: Option<ManualConflictResolution>,
    pub google_snapshot: Option<Value>,
    pub icloud_snapshot: Option<Value>,
    pub resolution_requested_at: Option<String>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManualConflictResolution {
    GoogleWins,
    IcloudWins,
    DeleteWins,
    UpdateWins,
}

impl ManualConflictResolution {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::GoogleWins => "google_wins",
            Self::IcloudWins => "icloud_wins",
            Self::DeleteWins => "delete_wins",
            Self::UpdateWins => "update_wins",
        }
    }

    fn from_str(value: &str) -> Option<Self> {
        match value {
            "google_wins" => Some(Self::GoogleWins),
            "icloud_wins" => Some(Self::IcloudWins),
            "delete_wins" => Some(Self::DeleteWins),
            "update_wins" => Some(Self::UpdateWins),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingManualResolution {
    pub conflict_id: String,
    pub sync_pair_id: String,
    pub canonical_uid: String,
    pub reason: String,
    pub resolution: ManualConflictResolution,
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

pub fn request_manual_resolution(
    conn: &Connection,
    conflict_id: &str,
    resolution: ManualConflictResolution,
) -> Result<Option<PendingManualResolution>, DbError> {
    conn.execute(
        r#"
        UPDATE sync_conflicts
        SET manual_resolution = ?,
            resolution_requested_at = CURRENT_TIMESTAMP
        WHERE id = ?
          AND resolved_at IS NULL
          AND canonical_uid IS NOT NULL
        "#,
        params![resolution.as_str(), conflict_id],
    )?;

    load_pending_resolution(conn, conflict_id)
}

pub fn list_pending_manual_resolutions(
    conn: &Connection,
    sync_pair_id: &str,
) -> Result<Vec<PendingManualResolution>, DbError> {
    let mut statement = conn.prepare(
        r#"
        SELECT id, sync_pair_id, canonical_uid, reason, manual_resolution
        FROM sync_conflicts
        WHERE sync_pair_id = ?
          AND resolved_at IS NULL
          AND canonical_uid IS NOT NULL
          AND manual_resolution IS NOT NULL
        ORDER BY resolution_requested_at ASC, created_at ASC
        "#,
    )?;

    let rows = statement
        .query_map(params![sync_pair_id], pending_resolution_from_row)?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(rows)
}

pub fn mark_manual_resolution_applied(conn: &Connection, conflict_id: &str) -> Result<(), DbError> {
    conn.execute(
        "UPDATE sync_conflicts SET resolved_at = CURRENT_TIMESTAMP WHERE id = ?",
        params![conflict_id],
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

pub fn get_unresolved_conflict(
    conn: &Connection,
    conflict_id: &str,
) -> Result<Option<UnresolvedConflictRow>, DbError> {
    Ok(query_conflicts(
        conn,
        "resolved_at IS NULL AND id = ?",
        params![conflict_id, 1_i64],
    )?
    .into_iter()
    .next())
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
          event_link_id,
          canonical_uid,
          reason,
          manual_resolution,
          google_snapshot,
          icloud_snapshot,
          resolution_requested_at,
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
                event_link_id: row.get("event_link_id")?,
                canonical_uid: row.get("canonical_uid")?,
                reason: row.get("reason")?,
                manual_resolution: row
                    .get::<_, Option<String>>("manual_resolution")?
                    .and_then(|value| ManualConflictResolution::from_str(&value)),
                google_snapshot: row
                    .get::<_, Option<String>>("google_snapshot")?
                    .and_then(|value| serde_json::from_str(&value).ok()),
                icloud_snapshot: row
                    .get::<_, Option<String>>("icloud_snapshot")?
                    .and_then(|value| serde_json::from_str(&value).ok()),
                resolution_requested_at: row.get("resolution_requested_at")?,
                created_at: row.get("created_at")?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(rows)
}

fn load_pending_resolution(
    conn: &Connection,
    conflict_id: &str,
) -> Result<Option<PendingManualResolution>, DbError> {
    conn.query_row(
        r#"
        SELECT id, sync_pair_id, canonical_uid, reason, manual_resolution
        FROM sync_conflicts
        WHERE id = ?
          AND resolved_at IS NULL
          AND canonical_uid IS NOT NULL
          AND manual_resolution IS NOT NULL
        "#,
        params![conflict_id],
        pending_resolution_from_row,
    )
    .optional()
    .map_err(Into::into)
}

fn pending_resolution_from_row(
    row: &rusqlite::Row<'_>,
) -> Result<PendingManualResolution, rusqlite::Error> {
    let resolution = row
        .get::<_, String>("manual_resolution")
        .ok()
        .and_then(|value| ManualConflictResolution::from_str(&value))
        .unwrap_or(ManualConflictResolution::UpdateWins);

    Ok(PendingManualResolution {
        conflict_id: row.get("id")?,
        sync_pair_id: row.get("sync_pair_id")?,
        canonical_uid: row.get("canonical_uid")?,
        reason: row.get("reason")?,
        resolution,
    })
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
        assert_eq!(
            rows[0]
                .google_snapshot
                .as_ref()
                .and_then(|snapshot| snapshot.get("title"))
                .and_then(Value::as_str),
            Some("Google")
        );
        assert!(rows[0].icloud_snapshot.is_none());
        assert!(rows[0].event_link_id.is_none());
        assert!(rows[0].manual_resolution.is_none());
        assert!(rows[0].resolution_requested_at.is_none());
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

    #[test]
    fn queues_and_marks_manual_resolution() {
        let conn = conn();

        record_conflict(&conn, conflict("uid-1", "both_sides_changed")).unwrap();
        let row = list_unresolved_conflicts(&conn, ConflictFilter::default())
            .unwrap()
            .pop()
            .unwrap();

        let pending =
            request_manual_resolution(&conn, &row.id, ManualConflictResolution::GoogleWins)
                .unwrap()
                .unwrap();
        assert_eq!(pending.canonical_uid, "uid-1");
        assert_eq!(pending.reason, "both_sides_changed");
        assert_eq!(pending.resolution, ManualConflictResolution::GoogleWins);

        let pending_for_pair = list_pending_manual_resolutions(&conn, "personal").unwrap();
        assert_eq!(pending_for_pair, vec![pending.clone()]);
        let queued_row = get_unresolved_conflict(&conn, &pending.conflict_id)
            .unwrap()
            .unwrap();
        assert_eq!(
            queued_row.manual_resolution,
            Some(ManualConflictResolution::GoogleWins)
        );

        mark_manual_resolution_applied(&conn, &pending.conflict_id).unwrap();
        assert!(
            list_pending_manual_resolutions(&conn, "personal")
                .unwrap()
                .is_empty()
        );
        assert!(
            list_unresolved_conflicts(&conn, ConflictFilter::default())
                .unwrap()
                .is_empty()
        );
    }
}
