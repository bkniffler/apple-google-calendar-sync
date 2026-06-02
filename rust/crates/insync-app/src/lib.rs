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
    pub selected_conflict_index: Option<usize>,
    pub run_filter: AppRunFilter,
    pub background_paused: bool,
    pub conflict_count: usize,
    pub last_message: Option<String>,
    pub last_run_at: Option<String>,
    pub last_run_status: Option<String>,
    pub next_run_at: Option<String>,
    pub recent_error: Option<String>,
    pub pairs: Vec<AppPair>,
    pub runs: Vec<AppRun>,
    pub conflict_summaries: Vec<AppConflictSummary>,
    pub conflict_details: Vec<AppConflictDetail>,
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
    pub conflict_summaries: Vec<AppConflictSummary>,
    pub conflict_details: Vec<AppConflictDetail>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppShellSnapshot {
    pub status: AppStatus,
    pub view: AppView,
    pub selected_pair_id: Option<String>,
    pub background_paused: bool,
    pub conflict_count: usize,
    pub enabled_pair_count: usize,
    pub total_pair_count: usize,
    pub last_run_at: Option<String>,
    pub last_run_status: Option<String>,
    pub next_run_at: Option<String>,
    pub recent_error: Option<String>,
    pub last_message: Option<String>,
    pub actions: Vec<AppShellAction>,
    pub notifications: Vec<AppShellNotification>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppShellAction {
    pub command: AppCommand,
    pub label: String,
    pub description: String,
    pub enabled: bool,
    pub destructive: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppShellNotification {
    pub severity: AppNotificationSeverity,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppNotificationSeverity {
    Info,
    Warning,
    Error,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppConflictSummary {
    pub pair_id: String,
    pub reason: String,
    pub count: usize,
    pub first_seen_at: String,
    pub last_seen_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppConflictDetail {
    pub id: String,
    pub pair_id: String,
    pub canonical_uid: Option<String>,
    pub reason: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppView {
    Dashboard,
    Runs,
    Conflicts,
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
    ShowConflicts,
    OpenSetup,
    ShowPairs,
    ShowRuns,
    ToggleBackgroundPause,
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
    ShowConflicts,
    CycleRunFilter,
    SelectNextRun,
    SelectPreviousRun,
    SelectNextConflict,
    SelectPreviousConflict,
    OpenCommandPalette,
    CloseCommandPalette,
    SelectNextCommand,
    SelectPreviousCommand,
    ExecuteCommand(AppCommand),
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
            selected_conflict_index: None,
            run_filter: AppRunFilter::All,
            background_paused: false,
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
            conflict_summaries: Vec::new(),
            conflict_details: Vec::new(),
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

    pub fn selected_conflict_summary(&self) -> Option<&AppConflictSummary> {
        self.selected_conflict_index
            .and_then(|index| self.conflict_summaries.get(index))
    }

    pub fn selected_conflict_details(&self) -> Vec<&AppConflictDetail> {
        let Some(summary) = self.selected_conflict_summary() else {
            return Vec::new();
        };
        self.conflict_details
            .iter()
            .filter(|detail| detail.pair_id == summary.pair_id && detail.reason == summary.reason)
            .collect()
    }

    pub fn shell_snapshot(&self) -> AppShellSnapshot {
        AppShellSnapshot {
            status: self.status,
            view: self.view,
            selected_pair_id: self.selected_pair_id.clone(),
            background_paused: self.background_paused,
            conflict_count: self.conflict_count,
            enabled_pair_count: self.enabled_pair_count(),
            total_pair_count: self.pairs.len(),
            last_run_at: self.last_run_at.clone(),
            last_run_status: self.last_run_status.clone(),
            next_run_at: self.next_run_at.clone(),
            recent_error: self.recent_error.clone(),
            last_message: self.last_message.clone(),
            actions: AppCommand::all()
                .iter()
                .map(|command| AppShellAction {
                    command: *command,
                    label: command.label_for(self).to_string(),
                    description: command.description_for(self).to_string(),
                    enabled: command.is_enabled(self),
                    destructive: command.is_destructive(),
                })
                .collect(),
            notifications: self.shell_notifications(),
        }
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
        self.conflict_summaries = snapshot.conflict_summaries;
        self.conflict_details = snapshot.conflict_details;
        self.ensure_selected_conflict();
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

    pub fn select_next_conflict(&mut self) {
        self.select_conflict_by_offset(1);
    }

    pub fn select_previous_conflict(&mut self) {
        self.select_conflict_by_offset(-1);
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
                self.background_paused = false;
                vec![AppEffect::StartBackgroundScheduler]
            }
            AppEvent::StopDaemon => {
                self.status = AppStatus::Idle;
                self.background_paused = true;
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
            AppEvent::ShowConflicts => {
                self.view = AppView::Conflicts;
                self.ensure_selected_conflict();
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
            AppEvent::SelectNextConflict => {
                self.select_next_conflict();
                Vec::new()
            }
            AppEvent::SelectPreviousConflict => {
                self.select_previous_conflict();
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
            AppEvent::ExecuteCommand(command) => self.execute_command(command),
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

    fn select_conflict_by_offset(&mut self, offset: isize) {
        if self.conflict_summaries.is_empty() {
            self.selected_conflict_index = None;
            return;
        }

        let current = self.selected_conflict_index.unwrap_or(0);
        let next =
            (current as isize + offset).rem_euclid(self.conflict_summaries.len() as isize) as usize;
        self.selected_conflict_index = Some(next);
    }

    fn ensure_selected_conflict(&mut self) {
        if self.conflict_summaries.is_empty() {
            self.selected_conflict_index = None;
            return;
        }

        let selected = self
            .selected_conflict_index
            .filter(|index| *index < self.conflict_summaries.len())
            .unwrap_or(0);
        self.selected_conflict_index = Some(selected);
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
        if !command.is_enabled(self) {
            return Vec::new();
        }
        match command {
            AppCommand::DryRun => self.update(AppEvent::StartDryRun),
            AppCommand::ApplyRun => self.update(AppEvent::StartApplyRun),
            AppCommand::RefreshConflicts => self.update(AppEvent::RefreshConflicts),
            AppCommand::ShowConflicts => self.update(AppEvent::ShowConflicts),
            AppCommand::OpenSetup => self.update(AppEvent::OpenSetup),
            AppCommand::ShowPairs => self.update(AppEvent::ShowDashboard),
            AppCommand::ShowRuns => self.update(AppEvent::ShowRuns),
            AppCommand::ToggleBackgroundPause => {
                if self.background_paused {
                    self.update(AppEvent::StartDaemon)
                } else {
                    self.update(AppEvent::StopDaemon)
                }
            }
            AppCommand::ExportReport => vec![AppEffect::ExportDryRunReport],
            AppCommand::Quit => vec![AppEffect::Quit],
        }
    }

    fn shell_notifications(&self) -> Vec<AppShellNotification> {
        let mut notifications = Vec::new();
        if let Some(error) = self.recent_error.as_ref().filter(|error| !error.is_empty()) {
            notifications.push(AppShellNotification {
                severity: AppNotificationSeverity::Error,
                message: error.clone(),
            });
        }
        if self.conflict_count > 0 {
            notifications.push(AppShellNotification {
                severity: AppNotificationSeverity::Warning,
                message: format!("{} unresolved conflict(s)", self.conflict_count),
            });
        }
        if self.pairs.is_empty() {
            notifications.push(AppShellNotification {
                severity: AppNotificationSeverity::Info,
                message: "No calendar pairs configured".to_string(),
            });
        }
        if self.background_paused {
            notifications.push(AppShellNotification {
                severity: AppNotificationSeverity::Info,
                message: "Background sync is paused".to_string(),
            });
        }
        notifications
    }
}

impl AppCommand {
    const ALL: [Self; 10] = [
        Self::DryRun,
        Self::ApplyRun,
        Self::RefreshConflicts,
        Self::ShowConflicts,
        Self::OpenSetup,
        Self::ShowPairs,
        Self::ShowRuns,
        Self::ToggleBackgroundPause,
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
            Self::ShowConflicts => "Show conflicts",
            Self::OpenSetup => "Open setup",
            Self::ShowPairs => "Show pairs",
            Self::ShowRuns => "Show sync runs",
            Self::ToggleBackgroundPause => "Pause background",
            Self::ExportReport => "Export report",
            Self::Quit => "Quit",
        }
    }

    pub fn label_for(self, model: &AppModel) -> &'static str {
        match self {
            Self::ToggleBackgroundPause if model.background_paused => "Resume background",
            _ => self.label(),
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::DryRun => "Plan provider changes without writing events",
            Self::ApplyRun => "Execute writes using the current sync plan",
            Self::RefreshConflicts => "Reload unresolved conflict state",
            Self::ShowConflicts => "Inspect unresolved conflict groups",
            Self::OpenSetup => "Start the guided configuration flow",
            Self::ShowPairs => "Return to the calendar-pair dashboard",
            Self::ShowRuns => "Open recent sync-run history",
            Self::ToggleBackgroundPause => "Pause the background scheduler",
            Self::ExportReport => "Export the latest dry-run report",
            Self::Quit => "Close the terminal dashboard",
        }
    }

    pub fn description_for(self, model: &AppModel) -> &'static str {
        match self {
            Self::ToggleBackgroundPause if model.background_paused => {
                "Resume the background scheduler"
            }
            _ => self.description(),
        }
    }

    pub fn is_enabled(self, model: &AppModel) -> bool {
        match self {
            Self::DryRun | Self::ApplyRun => model.enabled_pair_count() > 0,
            Self::RefreshConflicts
            | Self::ShowConflicts
            | Self::OpenSetup
            | Self::ShowPairs
            | Self::ShowRuns
            | Self::ToggleBackgroundPause
            | Self::ExportReport
            | Self::Quit => true,
        }
    }

    pub fn is_destructive(self) -> bool {
        matches!(self, Self::ApplyRun | Self::Quit)
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
            selected_conflict_index: None,
            run_filter: AppRunFilter::All,
            background_paused: false,
            conflict_count: 0,
            last_message: None,
            last_run_at: None,
            last_run_status: None,
            next_run_at: None,
            recent_error: None,
            pairs: Vec::new(),
            runs: Vec::new(),
            conflict_summaries: Vec::new(),
            conflict_details: Vec::new(),
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
            selected_conflict_index: None,
            run_filter: AppRunFilter::All,
            background_paused: false,
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
            conflict_summaries: Vec::new(),
            conflict_details: Vec::new(),
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
            selected_conflict_index: None,
            run_filter: AppRunFilter::All,
            background_paused: false,
            conflict_count: 0,
            last_message: None,
            last_run_at: None,
            last_run_status: None,
            next_run_at: None,
            recent_error: None,
            pairs: Vec::new(),
            runs: Vec::new(),
            conflict_summaries: Vec::new(),
            conflict_details: Vec::new(),
        };

        model.apply_runtime_snapshot(AppRuntimeSnapshot {
            conflict_count: 3,
            last_run_at: Some("2026-06-02 12:00:00".to_string()),
            last_run_status: Some("failed".to_string()),
            next_run_at: Some("2026-06-02 12:05:00".to_string()),
            recent_error: Some("auth failed".to_string()),
            pairs: Vec::new(),
            runs: Vec::new(),
            conflict_summaries: Vec::new(),
            conflict_details: Vec::new(),
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
            selected_conflict_index: None,
            run_filter: AppRunFilter::All,
            background_paused: false,
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
            conflict_summaries: Vec::new(),
            conflict_details: Vec::new(),
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
            conflict_summaries: Vec::new(),
            conflict_details: Vec::new(),
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
            selected_conflict_index: None,
            run_filter: AppRunFilter::All,
            background_paused: false,
            conflict_count: 0,
            last_message: None,
            last_run_at: None,
            last_run_status: None,
            next_run_at: None,
            recent_error: None,
            pairs: Vec::new(),
            runs: Vec::new(),
            conflict_summaries: Vec::new(),
            conflict_details: Vec::new(),
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
    fn conflict_view_selects_groups_and_filters_details() {
        let mut model = AppModel {
            status: AppStatus::Idle,
            view: AppView::Dashboard,
            command_palette_open: false,
            selected_command_index: 0,
            selected_pair_id: None,
            selected_run_id: None,
            selected_conflict_index: None,
            run_filter: AppRunFilter::All,
            background_paused: false,
            conflict_count: 0,
            last_message: None,
            last_run_at: None,
            last_run_status: None,
            next_run_at: None,
            recent_error: None,
            pairs: Vec::new(),
            runs: Vec::new(),
            conflict_summaries: Vec::new(),
            conflict_details: Vec::new(),
        };

        model.apply_runtime_snapshot(AppRuntimeSnapshot {
            conflict_count: 3,
            conflict_summaries: vec![
                AppConflictSummary {
                    pair_id: "personal".to_string(),
                    reason: "both_sides_changed".to_string(),
                    count: 2,
                    first_seen_at: "2026-06-02 12:00:00".to_string(),
                    last_seen_at: "2026-06-02 12:01:00".to_string(),
                },
                AppConflictSummary {
                    pair_id: "work".to_string(),
                    reason: "icloud_uid_exists".to_string(),
                    count: 1,
                    first_seen_at: "2026-06-02 13:00:00".to_string(),
                    last_seen_at: "2026-06-02 13:00:00".to_string(),
                },
            ],
            conflict_details: vec![
                AppConflictDetail {
                    id: "a".to_string(),
                    pair_id: "personal".to_string(),
                    canonical_uid: Some("uid-a".to_string()),
                    reason: "both_sides_changed".to_string(),
                    created_at: "2026-06-02 12:01:00".to_string(),
                },
                AppConflictDetail {
                    id: "b".to_string(),
                    pair_id: "work".to_string(),
                    canonical_uid: Some("uid-b".to_string()),
                    reason: "icloud_uid_exists".to_string(),
                    created_at: "2026-06-02 13:00:00".to_string(),
                },
            ],
            ..AppRuntimeSnapshot::default()
        });

        model.update(AppEvent::ShowConflicts);
        assert_eq!(model.view, AppView::Conflicts);
        assert_eq!(model.selected_conflict_index, Some(0));
        assert_eq!(model.selected_conflict_details().len(), 1);
        assert_eq!(
            model.selected_conflict_details()[0]
                .canonical_uid
                .as_deref(),
            Some("uid-a")
        );

        model.update(AppEvent::SelectNextConflict);
        assert_eq!(model.selected_conflict_summary().unwrap().pair_id, "work");
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
            selected_conflict_index: None,
            run_filter: AppRunFilter::All,
            background_paused: false,
            conflict_count: 0,
            last_message: None,
            last_run_at: None,
            last_run_status: None,
            next_run_at: None,
            recent_error: None,
            pairs: Vec::new(),
            runs: Vec::new(),
            conflict_summaries: Vec::new(),
            conflict_details: Vec::new(),
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
        model.selected_command_index = 6;
        let effects = model.update(AppEvent::ExecuteSelectedCommand);

        assert_eq!(model.view, AppView::Runs);
        assert_eq!(effects, Vec::<AppEffect>::new());
    }

    #[test]
    fn background_pause_command_toggles_scheduler_and_shell_action() {
        let mut model = AppModel::from_config(&ServiceConfig::default());

        let action = model
            .shell_snapshot()
            .actions
            .into_iter()
            .find(|action| action.command == AppCommand::ToggleBackgroundPause)
            .unwrap();
        assert_eq!(action.label, "Pause background");
        assert_eq!(action.description, "Pause the background scheduler");
        assert!(action.enabled);

        let effects = model.update(AppEvent::ExecuteCommand(AppCommand::ToggleBackgroundPause));
        assert_eq!(effects, vec![AppEffect::StopBackgroundScheduler]);
        assert!(model.background_paused);

        let snapshot = model.shell_snapshot();
        assert!(snapshot.background_paused);
        let action = snapshot
            .actions
            .iter()
            .find(|action| action.command == AppCommand::ToggleBackgroundPause)
            .unwrap();
        assert_eq!(action.label, "Resume background");
        assert_eq!(action.description, "Resume the background scheduler");
        assert!(
            snapshot
                .notifications
                .iter()
                .any(|notification| notification.message == "Background sync is paused")
        );

        let effects = model.update(AppEvent::ExecuteCommand(AppCommand::ToggleBackgroundPause));
        assert_eq!(effects, vec![AppEffect::StartBackgroundScheduler]);
        assert!(!model.background_paused);
    }

    #[test]
    fn shell_snapshot_exposes_actions_status_and_notifications() {
        let mut model = AppModel {
            status: AppStatus::Idle,
            view: AppView::Dashboard,
            command_palette_open: false,
            selected_command_index: 0,
            selected_pair_id: None,
            selected_run_id: None,
            selected_conflict_index: None,
            run_filter: AppRunFilter::All,
            background_paused: false,
            conflict_count: 2,
            last_message: Some("ready".to_string()),
            last_run_at: Some("2026-06-02 12:00:00".to_string()),
            last_run_status: Some("failed".to_string()),
            next_run_at: Some("2026-06-02 12:05:00".to_string()),
            recent_error: Some("auth failed".to_string()),
            pairs: Vec::new(),
            runs: Vec::new(),
            conflict_summaries: Vec::new(),
            conflict_details: Vec::new(),
        };

        let snapshot = model.shell_snapshot();

        assert_eq!(snapshot.status, AppStatus::Idle);
        assert_eq!(snapshot.conflict_count, 2);
        assert_eq!(snapshot.enabled_pair_count, 0);
        assert_eq!(snapshot.last_message.as_deref(), Some("ready"));
        let dry_run = snapshot
            .actions
            .iter()
            .find(|action| action.command == AppCommand::DryRun)
            .unwrap();
        assert!(!dry_run.enabled);
        let apply = snapshot
            .actions
            .iter()
            .find(|action| action.command == AppCommand::ApplyRun)
            .unwrap();
        assert!(apply.destructive);
        let setup = snapshot
            .actions
            .iter()
            .find(|action| action.command == AppCommand::OpenSetup)
            .unwrap();
        assert!(setup.enabled);
        assert_eq!(
            snapshot
                .notifications
                .iter()
                .map(|notification| notification.severity)
                .collect::<Vec<_>>(),
            vec![
                AppNotificationSeverity::Error,
                AppNotificationSeverity::Warning,
                AppNotificationSeverity::Info
            ]
        );
        assert_eq!(
            snapshot
                .notifications
                .iter()
                .map(|notification| notification.message.as_str())
                .collect::<Vec<_>>(),
            vec![
                "auth failed",
                "2 unresolved conflict(s)",
                "No calendar pairs configured"
            ]
        );

        let effects = model.update(AppEvent::ExecuteCommand(AppCommand::DryRun));
        assert!(effects.is_empty());
        assert_eq!(model.status, AppStatus::Idle);

        let effects = model.update(AppEvent::ExecuteCommand(AppCommand::OpenSetup));
        assert_eq!(effects, vec![AppEffect::ShowSetup]);
    }
}
