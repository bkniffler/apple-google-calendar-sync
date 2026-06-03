//! Cheap, read-only status snapshot read directly from the SQLite cache.
//!
//! The tray refreshes on a timer, so it must avoid anything that touches the OS
//! secret store (e.g. `SyncEngine::doctor`, which resolves credentials). Reading
//! the database directly keeps each refresh fast and side-effect free.

use std::path::{Path, PathBuf};

use insync_db::repositories::sync_conflicts::list_unresolved_conflict_summaries;
use insync_db::repositories::sync_runs::{SyncRun, SyncRunStatus, latest_sync_run};

/// High-level sync state, used to pick the tray icon colour and summary line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncState {
    /// No config/database yet — the user still needs to run setup.
    Unconfigured,
    /// Nothing running, no conflicts, last run (if any) succeeded.
    Idle,
    /// A run is currently in progress (daemon tick or manual sync).
    Syncing,
    /// Unresolved conflicts are waiting for a decision.
    Conflicts,
    /// The most recent run failed.
    Error,
}

#[derive(Debug, Clone)]
pub struct StatusSnapshot {
    pub state: SyncState,
    pub conflict_count: i64,
    pub last_run: Option<SyncRun>,
    pub background_running: bool,
}

impl StatusSnapshot {
    pub fn unconfigured() -> Self {
        Self {
            state: SyncState::Unconfigured,
            conflict_count: 0,
            last_run: None,
            background_running: false,
        }
    }

    /// Read run history and conflict counts from the database at `db_path`.
    ///
    /// Returns an unconfigured snapshot if the database cannot be opened yet.
    pub fn read(db_path: &Path, background_running: bool) -> Self {
        let Ok(conn) = insync_db::open(db_path) else {
            return Self::unconfigured();
        };
        if insync_db::migrate(&conn).is_err() {
            return Self::unconfigured();
        }

        let conflict_count = list_unresolved_conflict_summaries(&conn)
            .map(|rows| rows.iter().map(|row| row.count).sum())
            .unwrap_or(0);
        let last_run = latest_sync_run(&conn).ok().flatten();

        let state = match last_run.as_ref().map(|run| run.status) {
            Some(SyncRunStatus::Running) => SyncState::Syncing,
            Some(SyncRunStatus::Failed) => SyncState::Error,
            _ if conflict_count > 0 => SyncState::Conflicts,
            _ => SyncState::Idle,
        };

        Self {
            state,
            conflict_count,
            last_run,
            background_running,
        }
    }

    /// One-line summary shown in the (disabled) header menu item and tooltip.
    pub fn summary_line(&self) -> String {
        let head = match self.state {
            SyncState::Unconfigured => "Not configured",
            SyncState::Syncing => "Syncing…",
            SyncState::Error => "Last sync failed",
            SyncState::Conflicts => "Conflicts need attention",
            SyncState::Idle => "Idle",
        };
        let bg = if self.state == SyncState::Unconfigured {
            ""
        } else if self.background_running {
            " · background on"
        } else {
            " · background paused"
        };
        format!("insync — {head}{bg}")
    }

    /// Secondary detail line: last run time and conflict count.
    pub fn detail_line(&self) -> String {
        let mut parts = Vec::new();
        match self.last_run.as_ref() {
            Some(run) => {
                let when = run
                    .finished_at
                    .as_deref()
                    .unwrap_or(run.started_at.as_str());
                parts.push(format!("Last run {}", relative_time(when)));
            }
            None => parts.push("No runs yet".to_string()),
        }
        if self.conflict_count > 0 {
            parts.push(format!("{} conflict(s)", self.conflict_count));
        }
        parts.join(" · ")
    }
}

/// Resolve the database path without touching credentials.
///
/// Falls back to `None` when the config cannot be loaded yet (e.g. before
/// setup), in which case the tray runs in an unconfigured state.
pub fn resolve_db_path(config_path: &Path) -> Option<PathBuf> {
    let config = insync_config::load_config(config_path).ok()?;
    let engine = insync_engine::SyncEngine::with_config_path(config, config_path);
    Some(engine.db_path())
}

/// Render an SQLite timestamp ("YYYY-MM-DD HH:MM:SS" UTC) as a coarse relative
/// time like "just now", "4m ago", "2d ago". Falls back to the raw value.
fn relative_time(value: &str) -> String {
    use chrono::{NaiveDateTime, Utc};

    let parsed = NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S")
        .or_else(|_| NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S"));
    let Ok(naive) = parsed else {
        return value.to_string();
    };
    let then = naive.and_utc();
    let delta = Utc::now().signed_duration_since(then);
    let secs = delta.num_seconds();
    if secs < 0 {
        return "just now".to_string();
    }
    if secs < 45 {
        "just now".to_string()
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86_400 {
        format!("{}h ago", secs / 3600)
    } else {
        format!("{}d ago", secs / 86_400)
    }
}
