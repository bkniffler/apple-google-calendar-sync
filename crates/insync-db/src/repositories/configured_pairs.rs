use crate::{DbError, repositories::stable_id};
use insync_config::{ServiceConfig, SyncPairConfig};
use insync_core::{ConflictPolicy, ProviderName, SyncDirection};
use rusqlite::{Connection, params};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalendarIds {
    pub google_account_id: String,
    pub icloud_account_id: String,
    pub google_calendar_id: String,
    pub icloud_calendar_id: String,
}

#[derive(Debug, Clone)]
pub struct ProviderCalendarCache {
    pub provider: ProviderName,
    pub account_label: String,
    pub calendars: Vec<DiscoveredCalendar>,
}

#[derive(Debug, Clone)]
pub struct DiscoveredCalendar {
    pub id: String,
    pub name: String,
    pub timezone: Option<String>,
    pub writable: bool,
    pub raw: Value,
}

pub fn configured_calendar_ids(config: &ServiceConfig, pair: &SyncPairConfig) -> CalendarIds {
    CalendarIds {
        google_account_id: stable_id(&["account", "google", &config.google.account_label]),
        icloud_account_id: stable_id(&["account", "icloud", &config.icloud.account_label]),
        google_calendar_id: stable_id(&[
            "calendar",
            "google",
            &config.google.account_label,
            &pair.google_calendar_id,
        ]),
        icloud_calendar_id: stable_id(&[
            "calendar",
            "icloud",
            &config.icloud.account_label,
            &pair.icloud_calendar_id,
        ]),
    }
}

pub fn seed_configured_pairs(conn: &Connection, config: &ServiceConfig) -> Result<(), DbError> {
    for pair in &config.sync.pairs {
        let ids = configured_calendar_ids(config, pair);

        conn.execute(
            r#"
            INSERT INTO accounts (id, provider, email)
            VALUES (?, ?, ?)
            ON CONFLICT(provider, email) DO UPDATE SET
              updated_at = CURRENT_TIMESTAMP
            "#,
            params![ids.google_account_id, "google", config.google.account_label],
        )?;
        conn.execute(
            r#"
            INSERT INTO accounts (id, provider, email)
            VALUES (?, ?, ?)
            ON CONFLICT(provider, email) DO UPDATE SET
              updated_at = CURRENT_TIMESTAMP
            "#,
            params![ids.icloud_account_id, "icloud", config.icloud.account_label],
        )?;
        conn.execute(
            r#"
            INSERT INTO calendars (id, account_id, provider_calendar_id, name)
            VALUES (?, ?, ?, ?)
            ON CONFLICT(account_id, provider_calendar_id) DO UPDATE SET
              name = excluded.name,
              updated_at = CURRENT_TIMESTAMP
            "#,
            params![
                ids.google_calendar_id,
                ids.google_account_id,
                pair.google_calendar_id,
                pair.google_calendar_id
            ],
        )?;
        conn.execute(
            r#"
            INSERT INTO calendars (id, account_id, provider_calendar_id, name)
            VALUES (?, ?, ?, ?)
            ON CONFLICT(account_id, provider_calendar_id) DO UPDATE SET
              name = excluded.name,
              updated_at = CURRENT_TIMESTAMP
            "#,
            params![
                ids.icloud_calendar_id,
                ids.icloud_account_id,
                pair.icloud_calendar_id,
                pair.icloud_calendar_id
            ],
        )?;
        conn.execute(
            "INSERT INTO sync_state (calendar_id) VALUES (?) ON CONFLICT(calendar_id) DO NOTHING",
            params![ids.google_calendar_id],
        )?;
        conn.execute(
            "INSERT INTO sync_state (calendar_id) VALUES (?) ON CONFLICT(calendar_id) DO NOTHING",
            params![ids.icloud_calendar_id],
        )?;
        conn.execute(
            r#"
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
            "#,
            params![
                pair.id,
                ids.google_calendar_id,
                ids.icloud_calendar_id,
                direction_name(pair.direction),
                pair.enabled,
                conflict_policy_name(config.sync.conflict_policy)
            ],
        )?;
    }

    Ok(())
}

pub fn cache_provider_calendars(
    conn: &Connection,
    input: ProviderCalendarCache,
) -> Result<(), DbError> {
    let provider = provider_name(input.provider);
    let account_id = stable_id(&["account", provider, &input.account_label]);

    conn.execute(
        r#"
        INSERT INTO accounts (id, provider, email)
        VALUES (?, ?, ?)
        ON CONFLICT(provider, email) DO UPDATE SET
          updated_at = CURRENT_TIMESTAMP
        "#,
        params![account_id, provider, input.account_label],
    )?;

    for calendar in input.calendars {
        let calendar_id = stable_id(&["calendar", provider, &input.account_label, &calendar.id]);
        conn.execute(
            r#"
            INSERT INTO calendars (
              id,
              account_id,
              provider_calendar_id,
              name,
              timezone,
              writable,
              raw_json
            )
            VALUES (?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(account_id, provider_calendar_id) DO UPDATE SET
              name = excluded.name,
              timezone = excluded.timezone,
              writable = excluded.writable,
              raw_json = excluded.raw_json,
              updated_at = CURRENT_TIMESTAMP
            "#,
            params![
                calendar_id,
                account_id,
                calendar.id,
                calendar.name,
                calendar.timezone,
                calendar.writable,
                calendar.raw.to_string()
            ],
        )?;
        conn.execute(
            "INSERT INTO sync_state (calendar_id) VALUES (?) ON CONFLICT(calendar_id) DO NOTHING",
            params![calendar_id],
        )?;
    }

    Ok(())
}

fn provider_name(provider: ProviderName) -> &'static str {
    match provider {
        ProviderName::Google => "google",
        ProviderName::Icloud => "icloud",
    }
}

fn direction_name(direction: SyncDirection) -> &'static str {
    match direction {
        SyncDirection::TwoWay => "two_way",
        SyncDirection::LeftToRight => "left_to_right",
        SyncDirection::RightToLeft => "right_to_left",
    }
}

fn conflict_policy_name(policy: ConflictPolicy) -> &'static str {
    match policy {
        ConflictPolicy::Manual => "manual",
        ConflictPolicy::GoogleWins => "google_wins",
        ConflictPolicy::IcloudWins => "icloud_wins",
        ConflictPolicy::NewestUpdatedWins => "newest_updated_wins",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{migrate, open_in_memory};
    use insync_config::{GoogleConfig, IcloudConfig, SyncConfig};

    #[test]
    fn seeds_configured_pairs() {
        let conn = open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let config = config();

        seed_configured_pairs(&conn, &config).unwrap();

        let account_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM accounts", [], |row| row.get(0))
            .unwrap();
        let calendar_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM calendars", [], |row| row.get(0))
            .unwrap();
        let pair_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM sync_pairs", [], |row| row.get(0))
            .unwrap();
        let state_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM sync_state", [], |row| row.get(0))
            .unwrap();

        assert_eq!(account_count, 2);
        assert_eq!(calendar_count, 2);
        assert_eq!(pair_count, 1);
        assert_eq!(state_count, 2);
    }

    #[test]
    fn caches_discovered_calendars() {
        let conn = open_in_memory().unwrap();
        migrate(&conn).unwrap();

        cache_provider_calendars(
            &conn,
            ProviderCalendarCache {
                provider: ProviderName::Google,
                account_label: "personal".to_string(),
                calendars: vec![DiscoveredCalendar {
                    id: "primary".to_string(),
                    name: "Primary".to_string(),
                    timezone: Some("UTC".to_string()),
                    writable: true,
                    raw: serde_json::json!({ "id": "primary" }),
                }],
            },
        )
        .unwrap();

        let name: String = conn
            .query_row("SELECT name FROM calendars", [], |row| row.get(0))
            .unwrap();
        let state_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM sync_state", [], |row| row.get(0))
            .unwrap();

        assert_eq!(name, "Primary");
        assert_eq!(state_count, 1);
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
