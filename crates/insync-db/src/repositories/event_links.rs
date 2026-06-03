use crate::{DbError, repositories::stable_id};
use insync_core::EventLink;
use rusqlite::{Connection, OptionalExtension, params};

#[derive(Debug, Clone, Default)]
pub struct EventLinkUpsert {
    pub sync_pair_id: String,
    pub canonical_uid: String,
    pub google_event_id: Option<String>,
    pub google_ical_uid: Option<String>,
    pub google_etag: Option<String>,
    pub icloud_href: Option<String>,
    pub icloud_uid: Option<String>,
    pub icloud_etag: Option<String>,
    pub google_hash: Option<String>,
    pub icloud_hash: Option<String>,
    pub last_synced_hash: Option<String>,
    pub deleted_google_at: Option<String>,
    pub deleted_icloud_at: Option<String>,
}

pub fn load_event_links(conn: &Connection, sync_pair_id: &str) -> Result<Vec<EventLink>, DbError> {
    let mut statement = conn.prepare(
        r#"
        SELECT
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
        WHERE sync_pair_id = ?
        ORDER BY canonical_uid
        "#,
    )?;

    let rows = statement
        .query_map(params![sync_pair_id], event_link_from_row)?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(rows)
}

pub fn get_event_link(
    conn: &Connection,
    sync_pair_id: &str,
    canonical_uid: &str,
) -> Result<Option<EventLink>, DbError> {
    conn.query_row(
        r#"
        SELECT
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
        WHERE sync_pair_id = ? AND canonical_uid = ?
        "#,
        params![sync_pair_id, canonical_uid],
        event_link_from_row,
    )
    .optional()
    .map_err(DbError::from)
}

pub fn upsert_event_link(conn: &Connection, input: EventLinkUpsert) -> Result<(), DbError> {
    let id = stable_id(&["event-link", &input.sync_pair_id, &input.canonical_uid]);

    conn.execute(
        r#"
        INSERT INTO event_links (
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
          updated_at = CURRENT_TIMESTAMP
        "#,
        params![
            id,
            input.sync_pair_id,
            input.canonical_uid,
            input.google_event_id,
            input.google_ical_uid,
            input.google_etag,
            input.icloud_href,
            input.icloud_uid,
            input.icloud_etag,
            input.google_hash,
            input.icloud_hash,
            input.last_synced_hash,
            input.deleted_google_at,
            input.deleted_icloud_at,
        ],
    )?;

    Ok(())
}

fn event_link_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<EventLink> {
    Ok(EventLink {
        id: row.get("id")?,
        sync_pair_id: row.get("sync_pair_id")?,
        canonical_uid: row.get("canonical_uid")?,
        google_event_id: row.get("google_event_id")?,
        google_ical_uid: row.get("google_ical_uid")?,
        google_etag: row.get("google_etag")?,
        icloud_href: row.get("icloud_href")?,
        icloud_uid: row.get("icloud_uid")?,
        icloud_etag: row.get("icloud_etag")?,
        google_hash: row.get("google_hash")?,
        icloud_hash: row.get("icloud_hash")?,
        last_synced_hash: row.get("last_synced_hash")?,
        deleted_google_at: row.get("deleted_google_at")?,
        deleted_icloud_at: row.get("deleted_icloud_at")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{migrate, open_in_memory};

    fn conn() -> rusqlite::Connection {
        let conn = open_in_memory().unwrap();
        migrate(&conn).unwrap();
        seed_pair(&conn);
        conn
    }

    fn seed_pair(conn: &rusqlite::Connection) {
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

    #[test]
    fn upserts_and_loads_event_links() {
        let conn = conn();

        upsert_event_link(
            &conn,
            EventLinkUpsert {
                sync_pair_id: "personal".to_string(),
                canonical_uid: "uid-1".to_string(),
                google_event_id: Some("google-1".to_string()),
                google_hash: Some("google-hash".to_string()),
                icloud_href: Some("icloud-1.ics".to_string()),
                icloud_hash: Some("icloud-hash".to_string()),
                last_synced_hash: Some("synced-hash".to_string()),
                ..EventLinkUpsert::default()
            },
        )
        .unwrap();

        let links = load_event_links(&conn, "personal").unwrap();

        assert_eq!(links.len(), 1);
        assert_eq!(links[0].canonical_uid, "uid-1");
        assert_eq!(links[0].google_event_id.as_deref(), Some("google-1"));
        assert_eq!(links[0].icloud_href.as_deref(), Some("icloud-1.ics"));
    }

    #[test]
    fn upsert_preserves_existing_provider_ids_when_input_is_none() {
        let conn = conn();

        upsert_event_link(
            &conn,
            EventLinkUpsert {
                sync_pair_id: "personal".to_string(),
                canonical_uid: "uid-1".to_string(),
                google_event_id: Some("google-1".to_string()),
                icloud_href: Some("icloud-1.ics".to_string()),
                ..EventLinkUpsert::default()
            },
        )
        .unwrap();
        upsert_event_link(
            &conn,
            EventLinkUpsert {
                sync_pair_id: "personal".to_string(),
                canonical_uid: "uid-1".to_string(),
                google_hash: Some("new-google-hash".to_string()),
                deleted_google_at: Some("2026-06-02T00:00:00Z".to_string()),
                ..EventLinkUpsert::default()
            },
        )
        .unwrap();

        let link = get_event_link(&conn, "personal", "uid-1").unwrap().unwrap();

        assert_eq!(link.google_event_id.as_deref(), Some("google-1"));
        assert_eq!(link.icloud_href.as_deref(), Some("icloud-1.ics"));
        assert_eq!(link.google_hash.as_deref(), Some("new-google-hash"));
        assert_eq!(
            link.deleted_google_at.as_deref(),
            Some("2026-06-02T00:00:00Z")
        );
        assert_eq!(link.deleted_icloud_at, None);
    }
}
