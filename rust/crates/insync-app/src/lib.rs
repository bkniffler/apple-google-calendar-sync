use insync_config::ServiceConfig;
use insync_core::SyncDirection;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppModel {
    pub status: AppStatus,
    pub selected_pair_id: Option<String>,
    pub conflict_count: usize,
    pub last_message: Option<String>,
    pub last_run_at: Option<String>,
    pub last_run_status: Option<String>,
    pub recent_error: Option<String>,
    pub pairs: Vec<AppPair>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppPair {
    pub id: String,
    pub enabled: bool,
    pub direction: SyncDirection,
    pub google_calendar_id: String,
    pub icloud_calendar_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AppRuntimeSnapshot {
    pub conflict_count: usize,
    pub last_run_at: Option<String>,
    pub last_run_status: Option<String>,
    pub recent_error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppStatus {
    Idle,
    Checking,
    Syncing,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppEvent {
    StartDryRun,
    StartApplyRun,
    StartDaemon,
    StopDaemon,
    OpenSetup,
    RefreshConflicts,
    SelectPair(String),
    EngineFinished { message: String },
    EngineFailed { message: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppEffect {
    RunDrySync,
    RunApplySync,
    StartBackgroundScheduler,
    StopBackgroundScheduler,
    LoadConflicts,
    ShowSetup,
}

impl AppModel {
    pub fn from_config(config: &ServiceConfig) -> Self {
        Self {
            status: AppStatus::Idle,
            selected_pair_id: config.sync.pairs.first().map(|pair| pair.id.clone()),
            conflict_count: 0,
            last_message: None,
            last_run_at: None,
            last_run_status: None,
            recent_error: None,
            pairs: config
                .sync
                .pairs
                .iter()
                .map(|pair| AppPair {
                    id: pair.id.clone(),
                    enabled: pair.enabled,
                    direction: pair.direction,
                    google_calendar_id: pair.google_calendar_id.clone(),
                    icloud_calendar_id: pair.icloud_calendar_id.clone(),
                })
                .collect(),
        }
    }

    pub fn selected_pair(&self) -> Option<&AppPair> {
        let selected_pair_id = self.selected_pair_id.as_deref()?;
        self.pairs.iter().find(|pair| pair.id == selected_pair_id)
    }

    pub fn enabled_pair_count(&self) -> usize {
        self.pairs.iter().filter(|pair| pair.enabled).count()
    }

    pub fn apply_runtime_snapshot(&mut self, snapshot: AppRuntimeSnapshot) {
        self.conflict_count = snapshot.conflict_count;
        self.last_run_at = snapshot.last_run_at;
        self.last_run_status = snapshot.last_run_status;
        self.recent_error = snapshot.recent_error;
    }

    pub fn select_next_pair(&mut self) {
        self.select_pair_by_offset(1);
    }

    pub fn select_previous_pair(&mut self) {
        self.select_pair_by_offset(-1);
    }

    pub fn update(&mut self, event: AppEvent) -> Vec<AppEffect> {
        match event {
            AppEvent::StartDryRun => {
                self.status = AppStatus::Syncing;
                vec![AppEffect::RunDrySync]
            }
            AppEvent::StartApplyRun => {
                self.status = AppStatus::Syncing;
                vec![AppEffect::RunApplySync]
            }
            AppEvent::StartDaemon => {
                self.status = AppStatus::Syncing;
                vec![AppEffect::StartBackgroundScheduler]
            }
            AppEvent::StopDaemon => {
                self.status = AppStatus::Idle;
                vec![AppEffect::StopBackgroundScheduler]
            }
            AppEvent::OpenSetup => vec![AppEffect::ShowSetup],
            AppEvent::RefreshConflicts => vec![AppEffect::LoadConflicts],
            AppEvent::SelectPair(pair_id) => {
                self.selected_pair_id = Some(pair_id);
                Vec::new()
            }
            AppEvent::EngineFinished { message } => {
                self.status = AppStatus::Idle;
                self.last_message = Some(message);
                Vec::new()
            }
            AppEvent::EngineFailed { message } => {
                self.status = AppStatus::Error;
                self.last_message = Some(message);
                Vec::new()
            }
        }
    }

    fn select_pair_by_offset(&mut self, offset: isize) {
        if self.pairs.is_empty() {
            self.selected_pair_id = None;
            return;
        }

        let current = self
            .selected_pair_id
            .as_ref()
            .and_then(|selected| self.pairs.iter().position(|pair| &pair.id == selected))
            .unwrap_or(0);
        let next = (current as isize + offset).rem_euclid(self.pairs.len() as isize) as usize;
        self.selected_pair_id = Some(self.pairs[next].id.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dry_run_event_requests_sync_effect() {
        let mut model = AppModel {
            status: AppStatus::Idle,
            selected_pair_id: None,
            conflict_count: 0,
            last_message: None,
            last_run_at: None,
            last_run_status: None,
            recent_error: None,
            pairs: Vec::new(),
        };

        let effects = model.update(AppEvent::StartDryRun);

        assert_eq!(model.status, AppStatus::Syncing);
        assert_eq!(effects, vec![AppEffect::RunDrySync]);
    }

    #[test]
    fn pair_navigation_wraps() {
        let mut model = AppModel {
            status: AppStatus::Idle,
            selected_pair_id: Some("a".to_string()),
            conflict_count: 0,
            last_message: None,
            last_run_at: None,
            last_run_status: None,
            recent_error: None,
            pairs: vec![
                AppPair {
                    id: "a".to_string(),
                    enabled: true,
                    direction: SyncDirection::TwoWay,
                    google_calendar_id: "ga".to_string(),
                    icloud_calendar_id: "ia".to_string(),
                },
                AppPair {
                    id: "b".to_string(),
                    enabled: false,
                    direction: SyncDirection::LeftToRight,
                    google_calendar_id: "gb".to_string(),
                    icloud_calendar_id: "ib".to_string(),
                },
            ],
        };

        model.select_previous_pair();
        assert_eq!(model.selected_pair_id.as_deref(), Some("b"));
        model.select_next_pair();
        assert_eq!(model.selected_pair_id.as_deref(), Some("a"));
    }

    #[test]
    fn runtime_snapshot_updates_dashboard_state() {
        let mut model = AppModel {
            status: AppStatus::Idle,
            selected_pair_id: None,
            conflict_count: 0,
            last_message: None,
            last_run_at: None,
            last_run_status: None,
            recent_error: None,
            pairs: Vec::new(),
        };

        model.apply_runtime_snapshot(AppRuntimeSnapshot {
            conflict_count: 3,
            last_run_at: Some("2026-06-02 12:00:00".to_string()),
            last_run_status: Some("failed".to_string()),
            recent_error: Some("auth failed".to_string()),
        });

        assert_eq!(model.conflict_count, 3);
        assert_eq!(model.last_run_status.as_deref(), Some("failed"));
        assert_eq!(model.recent_error.as_deref(), Some("auth failed"));
    }
}
