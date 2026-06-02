use crate::DbError;
use rusqlite::{Connection, OptionalExtension, params};

pub fn update_calendar_sync_token(
    conn: &Connection,
    calendar_id: &str,
    sync_token: Option<&str>,
) -> Result<(), DbError> {
    let Some(sync_token) = sync_token else {
        return Ok(());
    };

    conn.execute(
        r#"
        INSERT INTO sync_state (
          calendar_id,
          provider_sync_token,
          last_incremental_sync_at,
          updated_at
        )
        VALUES (?, ?, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)
        ON CONFLICT(calendar_id) DO UPDATE SET
          provider_sync_token = excluded.provider_sync_token,
          last_incremental_sync_at = CURRENT_TIMESTAMP,
          updated_at = CURRENT_TIMESTAMP
        "#,
        params![calendar_id, sync_token],
    )?;

    Ok(())
}

pub fn load_calendar_sync_token(
    conn: &Connection,
    calendar_id: &str,
) -> Result<Option<String>, DbError> {
    conn.query_row(
        "SELECT provider_sync_token FROM sync_state WHERE calendar_id = ?",
        params![calendar_id],
        |row| row.get::<_, Option<String>>(0),
    )
    .optional()
    .map(|value| value.flatten())
    .map_err(DbError::from)
}

pub fn clear_calendar_sync_token(conn: &Connection, calendar_id: &str) -> Result<(), DbError> {
    conn.execute(
        r#"
        INSERT INTO sync_state (calendar_id, provider_sync_token, updated_at)
        VALUES (?, NULL, CURRENT_TIMESTAMP)
        ON CONFLICT(calendar_id) DO UPDATE SET
          provider_sync_token = NULL,
          updated_at = CURRENT_TIMESTAMP
        "#,
        params![calendar_id],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{migrate, open_in_memory};

    #[test]
    fn updates_and_loads_calendar_sync_token() {
        let conn = open_in_memory().unwrap();
        migrate(&conn).unwrap();
        seed_calendar(&conn);

        update_calendar_sync_token(&conn, "google-cal", Some("token-1")).unwrap();
        assert_eq!(
            load_calendar_sync_token(&conn, "google-cal")
                .unwrap()
                .as_deref(),
            Some("token-1")
        );

        update_calendar_sync_token(&conn, "google-cal", Some("token-2")).unwrap();
        assert_eq!(
            load_calendar_sync_token(&conn, "google-cal")
                .unwrap()
                .as_deref(),
            Some("token-2")
        );
    }

    #[test]
    fn ignores_empty_sync_token_updates() {
        let conn = open_in_memory().unwrap();
        migrate(&conn).unwrap();
        seed_calendar(&conn);

        update_calendar_sync_token(&conn, "google-cal", None).unwrap();

        assert_eq!(load_calendar_sync_token(&conn, "google-cal").unwrap(), None);
    }

    #[test]
    fn clears_sync_token() {
        let conn = open_in_memory().unwrap();
        migrate(&conn).unwrap();
        seed_calendar(&conn);

        update_calendar_sync_token(&conn, "google-cal", Some("token-1")).unwrap();
        clear_calendar_sync_token(&conn, "google-cal").unwrap();

        assert_eq!(load_calendar_sync_token(&conn, "google-cal").unwrap(), None);
    }

    fn seed_calendar(conn: &Connection) {
        conn.execute(
            "INSERT INTO accounts (id, provider, email) VALUES ('google-account', 'google', 'personal')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO calendars (id, account_id, provider_calendar_id, name) VALUES ('google-cal', 'google-account', 'primary', 'Primary')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO sync_state (calendar_id) VALUES ('google-cal')",
            [],
        )
        .unwrap();
    }
}
