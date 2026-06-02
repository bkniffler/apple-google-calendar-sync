use insync_config::ServiceConfig;
use insync_core::SyncDirection;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppModel {
    pub status: AppStatus,
    pub view: AppView,
    pub command_palette_open: bool,
    pub selected_command_index: usize,
    pub selected_pair_id: Option<String>,
    pub selected_run_id: Option<String>,
    pub run_filter: AppRunFilter,
    pub conflict_count: usize,
    pub last_message: Option<String>,
    pub last_run_at: Option<String>,
    pub last_run_status: Option<String>,
    pub next_run_at: Option<String>,
    pub recent_error: Option<String>,
    pub pairs: Vec<AppPair>,
    pub runs: Vec<AppRun>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppPair {
    pub id: String,
    pub enabled: bool,
    pub direction: SyncDirection,
    pub google_calendar_id: String,
    pub icloud_calendar_id: String,
    pub google_calendar_name: Option<String>,
    pub icloud_calendar_name: Option<String>,
    pub google_account_label: Option<String>,
    pub icloud_account_label: Option<String>,
    pub google_last_sync_at: Option<String>,
    pub icloud_last_sync_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AppRuntimeSnapshot {
    pub conflict_count: usize,
    pub last_run_at: Option<String>,
    pub last_run_status: Option<String>,
    pub next_run_at: Option<String>,
    pub recent_error: Option<String>,
    pub pairs: Vec<AppPairRuntimeSnapshot>,
    pub runs: Vec<AppRun>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppPairRuntimeSnapshot {
    pub pair_id: String,
    pub google_calendar_name: Option<String>,
    pub icloud_calendar_name: Option<String>,
    pub google_account_label: Option<String>,
    pub icloud_account_label: Option<String>,
    pub google_last_sync_at: Option<String>,
    pub icloud_last_sync_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppRun {
    pub id: String,
    pub pair_id: Option<String>,
    pub status: String,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppView {
    Dashboard,
    Runs,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppRunFilter {
    All,
    Running,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppCommand {
    DryRun,
    ApplyRun,
    RefreshConflicts,
    OpenSetup,
    ShowPairs,
    ShowRuns,
    ExportReport,
    Quit,
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
    ShowDashboard,
    ShowRuns,
    CycleRunFilter,
    SelectNextRun,
    SelectPreviousRun,
    OpenCommandPalette,
    CloseCommandPalette,
    SelectNextCommand,
    SelectPreviousCommand,
    ExecuteSelectedCommand,
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
    ExportDryRunReport,
    Quit,
}

impl AppModel {
    pub fn from_config(config: &ServiceConfig) -> Self {
        Self {
            status: AppStatus::Idle,
            view: AppView::Dashboard,
            command_palette_open: false,
            selected_command_index: 0,
            selected_pair_id: config.sync.pairs.first().map(|pair| pair.id.clone()),
            selected_run_id: None,
            run_filter: AppRunFilter::All,
            conflict_count: 0,
            last_message: None,
            last_run_at: None,
            last_run_status: None,
            next_run_at: None,
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
                    google_calendar_name: None,
                    icloud_calendar_name: None,
                    google_account_label: None,
                    icloud_account_label: None,
                    google_last_sync_at: None,
                    icloud_last_sync_at: None,
                })
                .collect(),
            runs: Vec::new(),
        }
    }

    pub fn selected_pair(&self) -> Option<&AppPair> {
        let selected_pair_id = self.selected_pair_id.as_deref()?;
        self.pairs.iter().find(|pair| pair.id == selected_pair_id)
    }

    pub fn enabled_pair_count(&self) -> usize {
        self.pairs.iter().filter(|pair| pair.enabled).count()
    }

    pub fn visible_runs(&self) -> Vec<&AppRun> {
        self.runs
            .iter()
            .filter(|run| self.run_filter.matches(&run.status))
            .collect()
    }

    pub fn selected_run(&self) -> Option<&AppRun> {
        let selected_run_id = self.selected_run_id.as_deref()?;
        self.runs.iter().find(|run| run.id == selected_run_id)
    }

    pub fn selected_command(&self) -> AppCommand {
        AppCommand::all()
            .get(self.selected_command_index)
            .copied()
            .unwrap_or(AppCommand::DryRun)
    }

    pub fn apply_runtime_snapshot(&mut self, snapshot: AppRuntimeSnapshot) {
        self.conflict_count = snapshot.conflict_count;
        self.last_run_at = snapshot.last_run_at;
        self.last_run_status = snapshot.last_run_status;
        self.next_run_at = snapshot.next_run_at;
        self.recent_error = snapshot.recent_error;
        for pair_snapshot in snapshot.pairs {
            if let Some(pair) = self
                .pairs
                .iter_mut()
                .find(|pair| pair.id == pair_snapshot.pair_id)
            {
                pair.google_calendar_name = pair_snapshot.google_calendar_name;
                pair.icloud_calendar_name = pair_snapshot.icloud_calendar_name;
                pair.google_account_label = pair_snapshot.google_account_label;
                pair.icloud_account_label = pair_snapshot.icloud_account_label;
                pair.google_last_sync_at = pair_snapshot.google_last_sync_at;
                pair.icloud_last_sync_at = pair_snapshot.icloud_last_sync_at;
            }
        }
        self.runs = snapshot.runs;
        self.ensure_selected_run();
    }

    pub fn select_next_pair(&mut self) {
        self.select_pair_by_offset(1);
    }

    pub fn select_previous_pair(&mut self) {
        self.select_pair_by_offset(-1);
    }

    pub fn select_next_run(&mut self) {
        self.select_run_by_offset(1);
    }

    pub fn select_previous_run(&mut self) {
        self.select_run_by_offset(-1);
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
            AppEvent::ShowDashboard => {
                self.view = AppView::Dashboard;
                Vec::new()
            }
            AppEvent::ShowRuns => {
                self.view = AppView::Runs;
                self.ensure_selected_run();
                Vec::new()
            }
            AppEvent::CycleRunFilter => {
                self.run_filter = self.run_filter.next();
                self.ensure_selected_run();
                Vec::new()
            }
            AppEvent::SelectNextRun => {
                self.select_next_run();
                Vec::new()
            }
            AppEvent::SelectPreviousRun => {
                self.select_previous_run();
                Vec::new()
            }
            AppEvent::OpenCommandPalette => {
                self.command_palette_open = true;
                Vec::new()
            }
            AppEvent::CloseCommandPalette => {
                self.command_palette_open = false;
                Vec::new()
            }
            AppEvent::SelectNextCommand => {
                self.select_command_by_offset(1);
                Vec::new()
            }
            AppEvent::SelectPreviousCommand => {
                self.select_command_by_offset(-1);
                Vec::new()
            }
            AppEvent::ExecuteSelectedCommand => {
                self.command_palette_open = false;
                self.execute_command(self.selected_command())
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

    fn select_run_by_offset(&mut self, offset: isize) {
        let visible_ids = self.visible_run_ids();
        if visible_ids.is_empty() {
            self.selected_run_id = None;
            return;
        }

        let current = self
            .selected_run_id
            .as_ref()
            .and_then(|selected| visible_ids.iter().position(|id| id == selected))
            .unwrap_or(0);
        let next = (current as isize + offset).rem_euclid(visible_ids.len() as isize) as usize;
        self.selected_run_id = Some(visible_ids[next].clone());
    }

    fn ensure_selected_run(&mut self) {
        let visible_ids = self.visible_run_ids();
        let selected_is_visible = self
            .selected_run_id
            .as_ref()
            .is_some_and(|selected| visible_ids.iter().any(|id| id == selected));
        if selected_is_visible {
            return;
        }
        self.selected_run_id = visible_ids.first().cloned();
    }

    fn visible_run_ids(&self) -> Vec<String> {
        self.runs
            .iter()
            .filter(|run| self.run_filter.matches(&run.status))
            .map(|run| run.id.clone())
            .collect()
    }

    fn select_command_by_offset(&mut self, offset: isize) {
        let command_count = AppCommand::all().len();
        let next =
            (self.selected_command_index as isize + offset).rem_euclid(command_count as isize);
        self.selected_command_index = next as usize;
    }

    fn execute_command(&mut self, command: AppCommand) -> Vec<AppEffect> {
        match command {
            AppCommand::DryRun => self.update(AppEvent::StartDryRun),
            AppCommand::ApplyRun => self.update(AppEvent::StartApplyRun),
            AppCommand::RefreshConflicts => self.update(AppEvent::RefreshConflicts),
            AppCommand::OpenSetup => self.update(AppEvent::OpenSetup),
            AppCommand::ShowPairs => self.update(AppEvent::ShowDashboard),
            AppCommand::ShowRuns => self.update(AppEvent::ShowRuns),
            AppCommand::ExportReport => vec![AppEffect::ExportDryRunReport],
            AppCommand::Quit => vec![AppEffect::Quit],
        }
    }
}

impl AppCommand {
    const ALL: [Self; 8] = [
        Self::DryRun,
        Self::ApplyRun,
        Self::RefreshConflicts,
        Self::OpenSetup,
        Self::ShowPairs,
        Self::ShowRuns,
        Self::ExportReport,
        Self::Quit,
    ];

    pub fn all() -> &'static [Self] {
        &Self::ALL
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::DryRun => "Dry-run sync",
            Self::ApplyRun => "Apply sync",
            Self::RefreshConflicts => "Refresh conflicts",
            Self::OpenSetup => "Open setup",
            Self::ShowPairs => "Show pairs",
            Self::ShowRuns => "Show sync runs",
            Self::ExportReport => "Export report",
            Self::Quit => "Quit",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::DryRun => "Plan provider changes without writing events",
            Self::ApplyRun => "Execute writes using the current sync plan",
            Self::RefreshConflicts => "Reload unresolved conflict state",
            Self::OpenSetup => "Start the guided configuration flow",
            Self::ShowPairs => "Return to the calendar-pair dashboard",
            Self::ShowRuns => "Open recent sync-run history",
            Self::ExportReport => "Export the latest dry-run report",
            Self::Quit => "Close the terminal dashboard",
        }
    }
}

impl AppRunFilter {
    pub fn label(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }

    fn matches(self, status: &str) -> bool {
        match self {
            Self::All => true,
            Self::Running => status == "running",
            Self::Completed => status == "completed",
            Self::Failed => status == "failed",
        }
    }

    fn next(self) -> Self {
        match self {
            Self::All => Self::Failed,
            Self::Failed => Self::Running,
            Self::Running => Self::Completed,
            Self::Completed => Self::All,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dry_run_event_requests_sync_effect() {
        let mut model = AppModel {
            status: AppStatus::Idle,
            view: AppView::Dashboard,
            command_palette_open: false,
            selected_command_index: 0,
            selected_pair_id: None,
            selected_run_id: None,
            run_filter: AppRunFilter::All,
            conflict_count: 0,
            last_message: None,
            last_run_at: None,
            last_run_status: None,
            next_run_at: None,
            recent_error: None,
            pairs: Vec::new(),
            runs: Vec::new(),
        };

        let effects = model.update(AppEvent::StartDryRun);

        assert_eq!(model.status, AppStatus::Syncing);
        assert_eq!(effects, vec![AppEffect::RunDrySync]);
    }

    #[test]
    fn pair_navigation_wraps() {
        let mut model = AppModel {
            status: AppStatus::Idle,
            view: AppView::Dashboard,
            command_palette_open: false,
            selected_command_index: 0,
            selected_pair_id: Some("a".to_string()),
            selected_run_id: None,
            run_filter: AppRunFilter::All,
            conflict_count: 0,
            last_message: None,
            last_run_at: None,
            last_run_status: None,
            next_run_at: None,
            recent_error: None,
            pairs: vec![
                AppPair {
                    id: "a".to_string(),
                    enabled: true,
                    direction: SyncDirection::TwoWay,
                    google_calendar_id: "ga".to_string(),
                    icloud_calendar_id: "ia".to_string(),
                    google_calendar_name: None,
                    icloud_calendar_name: None,
                    google_account_label: None,
                    icloud_account_label: None,
                    google_last_sync_at: None,
                    icloud_last_sync_at: None,
                },
                AppPair {
                    id: "b".to_string(),
                    enabled: false,
                    direction: SyncDirection::LeftToRight,
                    google_calendar_id: "gb".to_string(),
                    icloud_calendar_id: "ib".to_string(),
                    google_calendar_name: None,
                    icloud_calendar_name: None,
                    google_account_label: None,
                    icloud_account_label: None,
                    google_last_sync_at: None,
                    icloud_last_sync_at: None,
                },
            ],
            runs: Vec::new(),
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
            view: AppView::Dashboard,
            command_palette_open: false,
            selected_command_index: 0,
            selected_pair_id: None,
            selected_run_id: None,
            run_filter: AppRunFilter::All,
            conflict_count: 0,
            last_message: None,
            last_run_at: None,
            last_run_status: None,
            next_run_at: None,
            recent_error: None,
            pairs: Vec::new(),
            runs: Vec::new(),
        };

        model.apply_runtime_snapshot(AppRuntimeSnapshot {
            conflict_count: 3,
            last_run_at: Some("2026-06-02 12:00:00".to_string()),
            last_run_status: Some("failed".to_string()),
            next_run_at: Some("2026-06-02 12:05:00".to_string()),
            recent_error: Some("auth failed".to_string()),
            pairs: Vec::new(),
            runs: Vec::new(),
        });

        assert_eq!(model.conflict_count, 3);
        assert_eq!(model.last_run_status.as_deref(), Some("failed"));
        assert_eq!(model.next_run_at.as_deref(), Some("2026-06-02 12:05:00"));
        assert_eq!(model.recent_error.as_deref(), Some("auth failed"));
    }

    #[test]
    fn runtime_snapshot_updates_pair_metadata() {
        let mut model = AppModel {
            status: AppStatus::Idle,
            view: AppView::Dashboard,
            command_palette_open: false,
            selected_command_index: 0,
            selected_pair_id: Some("a".to_string()),
            selected_run_id: None,
            run_filter: AppRunFilter::All,
            conflict_count: 0,
            last_message: None,
            last_run_at: None,
            last_run_status: None,
            next_run_at: None,
            recent_error: None,
            pairs: vec![AppPair {
                id: "a".to_string(),
                enabled: true,
                direction: SyncDirection::TwoWay,
                google_calendar_id: "ga".to_string(),
                icloud_calendar_id: "ia".to_string(),
                google_calendar_name: None,
                icloud_calendar_name: None,
                google_account_label: None,
                icloud_account_label: None,
                google_last_sync_at: None,
                icloud_last_sync_at: None,
            }],
            runs: Vec::new(),
        };

        model.apply_runtime_snapshot(AppRuntimeSnapshot {
            pairs: vec![AppPairRuntimeSnapshot {
                pair_id: "a".to_string(),
                google_calendar_name: Some("Work".to_string()),
                icloud_calendar_name: Some("Home".to_string()),
                google_account_label: Some("me@example.com".to_string()),
                icloud_account_label: Some("me@icloud.com".to_string()),
                google_last_sync_at: Some("2026-06-02 12:00:00".to_string()),
                icloud_last_sync_at: Some("2026-06-02 12:01:00".to_string()),
            }],
            runs: Vec::new(),
            ..AppRuntimeSnapshot::default()
        });

        let pair = model.selected_pair().unwrap();
        assert_eq!(pair.google_calendar_name.as_deref(), Some("Work"));
        assert_eq!(
            pair.icloud_last_sync_at.as_deref(),
            Some("2026-06-02 12:01:00")
        );
    }

    #[test]
    fn run_view_filters_and_selects_runs() {
        let mut model = AppModel {
            status: AppStatus::Idle,
            view: AppView::Dashboard,
            command_palette_open: false,
            selected_command_index: 0,
            selected_pair_id: None,
            selected_run_id: None,
            run_filter: AppRunFilter::All,
            conflict_count: 0,
            last_message: None,
            last_run_at: None,
            last_run_status: None,
            next_run_at: None,
            recent_error: None,
            pairs: Vec::new(),
            runs: Vec::new(),
        };

        model.apply_runtime_snapshot(AppRuntimeSnapshot {
            runs: vec![
                AppRun {
                    id: "failed".to_string(),
                    pair_id: Some("personal".to_string()),
                    status: "failed".to_string(),
                    started_at: "2026-06-02 12:00:00".to_string(),
                    finished_at: Some("2026-06-02 12:01:00".to_string()),
                    error: Some("auth failed".to_string()),
                },
                AppRun {
                    id: "completed".to_string(),
                    pair_id: Some("personal".to_string()),
                    status: "completed".to_string(),
                    started_at: "2026-06-02 11:00:00".to_string(),
                    finished_at: Some("2026-06-02 11:01:00".to_string()),
                    error: None,
                },
            ],
            ..AppRuntimeSnapshot::default()
        });

        model.update(AppEvent::ShowRuns);
        assert_eq!(model.view, AppView::Runs);
        assert_eq!(model.selected_run_id.as_deref(), Some("failed"));

        model.update(AppEvent::CycleRunFilter);
        assert_eq!(model.run_filter, AppRunFilter::Failed);
        assert_eq!(model.visible_runs().len(), 1);

        model.update(AppEvent::CycleRunFilter);
        model.update(AppEvent::CycleRunFilter);
        assert_eq!(model.run_filter, AppRunFilter::Completed);
        assert_eq!(model.selected_run_id.as_deref(), Some("completed"));
    }

    #[test]
    fn command_palette_selects_and_executes_actions() {
        let mut model = AppModel {
            status: AppStatus::Idle,
            view: AppView::Dashboard,
            command_palette_open: false,
            selected_command_index: 0,
            selected_pair_id: None,
            selected_run_id: None,
            run_filter: AppRunFilter::All,
            conflict_count: 0,
            last_message: None,
            last_run_at: None,
            last_run_status: None,
            next_run_at: None,
            recent_error: None,
            pairs: Vec::new(),
            runs: Vec::new(),
        };

        model.update(AppEvent::OpenCommandPalette);
        assert!(model.command_palette_open);
        assert_eq!(model.selected_command(), AppCommand::DryRun);

        model.update(AppEvent::SelectPreviousCommand);
        assert_eq!(model.selected_command(), AppCommand::Quit);
        let effects = model.update(AppEvent::ExecuteSelectedCommand);

        assert!(!model.command_palette_open);
        assert_eq!(effects, vec![AppEffect::Quit]);

        model.update(AppEvent::OpenCommandPalette);
        model.selected_command_index = 5;
        let effects = model.update(AppEvent::ExecuteSelectedCommand);

        assert_eq!(model.view, AppView::Runs);
        assert_eq!(effects, Vec::<AppEffect>::new());
    }
}
