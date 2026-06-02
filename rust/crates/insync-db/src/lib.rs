pub mod migrations;
pub mod repositories;

use rusqlite::{
    Connection, ToSql, params,
    types::{Value as SqlValue, ValueRef},
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum DbError {
    #[error("failed to create database directory {path}: {source}")]
    CreateDir {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("io error for {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("json error for {path}: {source}")]
    Json {
        path: PathBuf,
        source: serde_json::Error,
    },
    #[error("unsupported export table: {0}")]
    UnsupportedExportTable(String),
    #[error("unsupported export format: {0}")]
    UnsupportedExportFormat(String),
}

const EXPORT_FORMAT: &str = "insync.sqlite.export.v1";

const EXPORT_TABLES: &[TableSpec] = &[
    TableSpec {
        name: "schema_migrations",
        columns: &["id", "name", "applied_at"],
    },
    TableSpec {
        name: "accounts",
        columns: &[
            "id",
            "provider",
            "email",
            "auth_json",
            "created_at",
            "updated_at",
        ],
    },
    TableSpec {
        name: "calendars",
        columns: &[
            "id",
            "account_id",
            "provider_calendar_id",
            "name",
            "color",
            "timezone",
            "writable",
            "raw_json",
            "created_at",
            "updated_at",
        ],
    },
    TableSpec {
        name: "sync_pairs",
        columns: &[
            "id",
            "left_calendar_id",
            "right_calendar_id",
            "direction",
            "enabled",
            "conflict_policy",
            "created_at",
            "updated_at",
        ],
    },
    TableSpec {
        name: "sync_state",
        columns: &[
            "calendar_id",
            "provider_sync_token",
            "last_full_sync_at",
            "last_incremental_sync_at",
            "updated_at",
        ],
    },
    TableSpec {
        name: "event_links",
        columns: &[
            "id",
            "sync_pair_id",
            "canonical_uid",
            "google_event_id",
            "google_ical_uid",
            "google_etag",
            "icloud_href",
            "icloud_uid",
            "icloud_etag",
            "google_hash",
            "icloud_hash",
            "last_synced_hash",
            "deleted_google_at",
            "deleted_icloud_at",
            "created_at",
            "updated_at",
        ],
    },
    TableSpec {
        name: "sync_runs",
        columns: &[
            "id",
            "sync_pair_id",
            "status",
            "started_at",
            "finished_at",
            "error",
        ],
    },
    TableSpec {
        name: "sync_conflicts",
        columns: &[
            "id",
            "sync_pair_id",
            "event_link_id",
            "canonical_uid",
            "reason",
            "google_snapshot",
            "icloud_snapshot",
            "resolved_at",
            "created_at",
        ],
    },
];

#[derive(Debug, Clone, Copy)]
struct TableSpec {
    name: &'static str,
    columns: &'static [&'static str],
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseExport {
    pub format: String,
    pub tables: BTreeMap<String, Vec<BTreeMap<String, Value>>>,
}

pub fn open(path: impl AsRef<Path>) -> Result<Connection, DbError> {
    let path = path.as_ref();
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(|source| DbError::CreateDir {
            path: parent.to_path_buf(),
            source,
        })?;
    }

    let conn = Connection::open(path)?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    Ok(conn)
}

pub fn open_in_memory() -> Result<Connection, DbError> {
    let conn = Connection::open_in_memory()?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    Ok(conn)
}

pub fn migrate(conn: &Connection) -> Result<(), DbError> {
    migrations::migrate(conn)
}

pub fn backup_database(
    source: impl AsRef<Path>,
    destination: impl AsRef<Path>,
) -> Result<(), DbError> {
    let source = source.as_ref();
    let destination = destination.as_ref();
    ensure_parent_dir(destination)?;
    let conn = open(source)?;
    migrate(&conn)?;
    conn.execute(
        "VACUUM INTO ?",
        params![destination.to_string_lossy().as_ref()],
    )?;
    Ok(())
}

pub fn export_database_json(
    source: impl AsRef<Path>,
    destination: impl AsRef<Path>,
) -> Result<DatabaseExport, DbError> {
    let source = source.as_ref();
    let destination = destination.as_ref();
    ensure_parent_dir(destination)?;
    let conn = open(source)?;
    migrate(&conn)?;
    let export = export_connection(&conn)?;
    let body = serde_json::to_string_pretty(&export).map_err(|source| DbError::Json {
        path: destination.to_path_buf(),
        source,
    })?;
    fs::write(destination, format!("{body}\n")).map_err(|source| DbError::Io {
        path: destination.to_path_buf(),
        source,
    })?;
    Ok(export)
}

pub fn import_database_json(
    source: impl AsRef<Path>,
    destination: impl AsRef<Path>,
) -> Result<(), DbError> {
    let source = source.as_ref();
    let destination = destination.as_ref();
    ensure_parent_dir(destination)?;

    let mut body = String::new();
    fs::File::open(source)
        .and_then(|mut file| file.read_to_string(&mut body))
        .map_err(|source_error| DbError::Io {
            path: source.to_path_buf(),
            source: source_error,
        })?;
    let export: DatabaseExport =
        serde_json::from_str(&body).map_err(|source_error| DbError::Json {
            path: source.to_path_buf(),
            source: source_error,
        })?;

    let mut conn = open(destination)?;
    migrate(&conn)?;
    import_export(&mut conn, export)
}

pub fn export_connection(conn: &Connection) -> Result<DatabaseExport, DbError> {
    let mut tables = BTreeMap::new();
    for table in EXPORT_TABLES {
        tables.insert(table.name.to_string(), export_table(conn, table)?);
    }

    Ok(DatabaseExport {
        format: EXPORT_FORMAT.to_string(),
        tables,
    })
}

pub fn import_export(conn: &mut Connection, export: DatabaseExport) -> Result<(), DbError> {
    if export.format != EXPORT_FORMAT {
        return Err(DbError::UnsupportedExportFormat(export.format));
    }
    for table in export.tables.keys() {
        if !EXPORT_TABLES.iter().any(|spec| spec.name == table) {
            return Err(DbError::UnsupportedExportTable(table.clone()));
        }
    }

    let tx = conn.transaction()?;
    for table in EXPORT_TABLES.iter().rev() {
        tx.execute(&format!("DELETE FROM {}", table.name), [])?;
    }
    for table in EXPORT_TABLES {
        let rows = export.tables.get(table.name).cloned().unwrap_or_default();
        import_table(&tx, table, &rows)?;
    }
    tx.commit()?;
    Ok(())
}

fn export_table(
    conn: &Connection,
    table: &TableSpec,
) -> Result<Vec<BTreeMap<String, Value>>, DbError> {
    let sql = format!(
        "SELECT {} FROM {} ORDER BY {}",
        table.columns.join(", "),
        table.name,
        table.columns[0]
    );
    let mut statement = conn.prepare(&sql)?;
    let rows = statement
        .query_map([], |row| {
            let mut values = BTreeMap::new();
            for (index, column) in table.columns.iter().enumerate() {
                values.insert(column.to_string(), sql_value_to_json(row.get_ref(index)?));
            }
            Ok(values)
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

fn import_table(
    conn: &Connection,
    table: &TableSpec,
    rows: &[BTreeMap<String, Value>],
) -> Result<(), DbError> {
    for row in rows {
        for key in row.keys() {
            if !table.columns.contains(&key.as_str()) {
                return Err(DbError::UnsupportedExportTable(format!(
                    "{}.{}",
                    table.name, key
                )));
            }
        }

        let placeholders = std::iter::repeat_n("?", table.columns.len())
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "INSERT INTO {} ({}) VALUES ({})",
            table.name,
            table.columns.join(", "),
            placeholders
        );
        let values = table
            .columns
            .iter()
            .map(|column| json_value_to_sql(row.get(*column).unwrap_or(&Value::Null)))
            .collect::<Vec<_>>();
        let params = values.iter().map(|value| value as &dyn ToSql);
        conn.execute(&sql, rusqlite::params_from_iter(params))?;
    }

    Ok(())
}

fn sql_value_to_json(value: ValueRef<'_>) -> Value {
    match value {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(value) => Value::from(value),
        ValueRef::Real(value) => Value::from(value),
        ValueRef::Text(value) => Value::from(String::from_utf8_lossy(value).into_owned()),
        ValueRef::Blob(value) => {
            let mut object = Map::new();
            object.insert("$blobHex".to_string(), Value::from(hex_encode(value)));
            Value::Object(object)
        }
    }
}

fn json_value_to_sql(value: &Value) -> SqlValue {
    match value {
        Value::Null => SqlValue::Null,
        Value::Bool(value) => SqlValue::Integer(i64::from(*value)),
        Value::Number(value) => {
            if let Some(value) = value.as_i64() {
                SqlValue::Integer(value)
            } else {
                SqlValue::Real(value.as_f64().unwrap_or_default())
            }
        }
        Value::String(value) => SqlValue::Text(value.clone()),
        Value::Array(_) => SqlValue::Text(value.to_string()),
        Value::Object(object) => object
            .get("$blobHex")
            .and_then(Value::as_str)
            .map(hex_decode)
            .map(SqlValue::Blob)
            .unwrap_or_else(|| SqlValue::Text(value.to_string())),
    }
}

fn ensure_parent_dir(path: &Path) -> Result<(), DbError> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(|source| DbError::CreateDir {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    Ok(())
}

fn hex_encode(value: &[u8]) -> String {
    value.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn hex_decode(value: &str) -> Vec<u8> {
    value
        .as_bytes()
        .chunks(2)
        .filter_map(|chunk| std::str::from_utf8(chunk).ok())
        .filter_map(|chunk| u8::from_str_radix(chunk, 16).ok())
        .collect()
}

pub fn write_export_to_writer(
    export: &DatabaseExport,
    mut writer: impl Write,
) -> Result<(), DbError> {
    serde_json::to_writer_pretty(&mut writer, export).map_err(|source| DbError::Json {
        path: PathBuf::from("<writer>"),
        source,
    })?;
    writer.write_all(b"\n").map_err(|source| DbError::Io {
        path: PathBuf::from("<writer>"),
        source,
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repositories::configured_pairs::seed_configured_pairs;
    use insync_config::{GoogleConfig, IcloudConfig, ServiceConfig, SyncConfig, SyncPairConfig};
    use insync_core::SyncDirection;

    #[test]
    fn exports_and_imports_database_json() {
        let temp_dir =
            std::env::temp_dir().join(format!("insync-db-export-{}", std::process::id()));
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).unwrap();
        let source = temp_dir.join("source.db");
        let export_path = temp_dir.join("support.json");
        let destination = temp_dir.join("destination.db");
        let conn = open(&source).unwrap();
        migrate(&conn).unwrap();
        seed_configured_pairs(&conn, &config()).unwrap();

        let export = export_database_json(&source, &export_path).unwrap();
        import_database_json(&export_path, &destination).unwrap();
        let imported = open(&destination).unwrap();
        let imported_export = export_connection(&imported).unwrap();

        assert_eq!(export.tables["accounts"].len(), 2);
        assert_eq!(export.tables, imported_export.tables);

        fs::remove_dir_all(&temp_dir).unwrap();
    }

    #[test]
    fn backs_up_database_with_vacuum_into() {
        let temp_dir =
            std::env::temp_dir().join(format!("insync-db-backup-{}", std::process::id()));
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).unwrap();
        let source = temp_dir.join("source.db");
        let destination = temp_dir.join("backup.db");
        let conn = open(&source).unwrap();
        migrate(&conn).unwrap();
        seed_configured_pairs(&conn, &config()).unwrap();
        drop(conn);

        backup_database(&source, &destination).unwrap();
        let backup = open(&destination).unwrap();
        let count: i64 = backup
            .query_row("SELECT COUNT(*) FROM calendars", [], |row| row.get(0))
            .unwrap();

        assert_eq!(count, 2);

        fs::remove_dir_all(&temp_dir).unwrap();
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
