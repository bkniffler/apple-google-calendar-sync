use crate::DbError;
use rusqlite::Connection;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountRow {
    pub id: String,
    pub provider: String,
    pub email: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalendarRow {
    pub id: String,
    pub account_id: String,
    pub provider: String,
    pub account_email: String,
    pub provider_calendar_id: String,
    pub name: Option<String>,
    pub color: Option<String>,
    pub timezone: Option<String>,
    pub writable: bool,
    pub raw_json: Option<String>,
    pub provider_sync_token: Option<String>,
    pub last_full_sync_at: Option<String>,
    pub last_incremental_sync_at: Option<String>,
    pub sync_state_updated_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

pub fn list_accounts(conn: &Connection) -> Result<Vec<AccountRow>, DbError> {
    let mut statement = conn.prepare(
        r#"
        SELECT id, provider, email, created_at, updated_at
        FROM accounts
        ORDER BY provider, email
        "#,
    )?;

    Ok(statement
        .query_map([], |row| {
            Ok(AccountRow {
                id: row.get(0)?,
                provider: row.get(1)?,
                email: row.get(2)?,
                created_at: row.get(3)?,
                updated_at: row.get(4)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?)
}

pub fn list_calendars(conn: &Connection) -> Result<Vec<CalendarRow>, DbError> {
    let mut statement = conn.prepare(
        r#"
        SELECT
          calendars.id,
          calendars.account_id,
          accounts.provider,
          accounts.email,
          calendars.provider_calendar_id,
          calendars.name,
          calendars.color,
          calendars.timezone,
          calendars.writable,
          calendars.raw_json,
          sync_state.provider_sync_token,
          sync_state.last_full_sync_at,
          sync_state.last_incremental_sync_at,
          sync_state.updated_at,
          calendars.created_at,
          calendars.updated_at
        FROM calendars
        JOIN accounts ON accounts.id = calendars.account_id
        LEFT JOIN sync_state ON sync_state.calendar_id = calendars.id
        ORDER BY accounts.provider, accounts.email, calendars.name, calendars.provider_calendar_id
        "#,
    )?;

    Ok(statement
        .query_map([], |row| {
            Ok(CalendarRow {
                id: row.get(0)?,
                account_id: row.get(1)?,
                provider: row.get(2)?,
                account_email: row.get(3)?,
                provider_calendar_id: row.get(4)?,
                name: row.get(5)?,
                color: row.get(6)?,
                timezone: row.get(7)?,
                writable: row.get::<_, i64>(8)? != 0,
                raw_json: row.get(9)?,
                provider_sync_token: row.get(10)?,
                last_full_sync_at: row.get(11)?,
                last_incremental_sync_at: row.get(12)?,
                sync_state_updated_at: row.get(13)?,
                created_at: row.get(14)?,
                updated_at: row.get(15)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{migrate, open_in_memory, repositories::configured_pairs::seed_configured_pairs};
    use insync_config::{GoogleConfig, IcloudConfig, ServiceConfig, SyncConfig, SyncPairConfig};
    use insync_core::SyncDirection;

    #[test]
    fn lists_accounts_and_calendars_with_provider_context() {
        let conn = open_in_memory().unwrap();
        migrate(&conn).unwrap();
        seed_configured_pairs(&conn, &config()).unwrap();

        let accounts = list_accounts(&conn).unwrap();
        let calendars = list_calendars(&conn).unwrap();

        assert_eq!(accounts.len(), 2);
        assert_eq!(calendars.len(), 2);
        assert_eq!(calendars[0].provider, "google");
        assert_eq!(calendars[0].account_email, "personal");
        assert!(calendars[0].writable);
    }

    fn config() -> ServiceConfig {
        ServiceConfig {
            google: GoogleConfig {
                account_label: "personal".to_string(),
                ..GoogleConfig::default()
            },
            icloud: IcloudConfig {
                account_label: "personal".to_string(),
                ..IcloudConfig::default()
            },
            sync: SyncConfig {
                pairs: vec![SyncPairConfig {
                    id: "personal".to_string(),
                    enabled: true,
                    direction: SyncDirection::TwoWay,
                    google_calendar_id: "primary".to_string(),
                    icloud_calendar_id: "https://caldav.example/cal".to_string(),
                }],
                ..SyncConfig::default()
            },
            ..serde_json::from_str(
                r#"{
                  "google": {},
                  "icloud": {}
                }"#,
            )
            .unwrap()
        }
    }
}
