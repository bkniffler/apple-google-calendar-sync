use std::collections::BTreeMap;

use insync_config::{SecretStoreKind, ServiceConfig};
use insync_core::SyncDirection;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppModel {
    pub status: AppStatus,
    pub view: AppView,
    pub command_palette_open: bool,
    pub confirm_apply: bool,
    pub selected_command_index: usize,
    pub selected_pair_id: Option<String>,
    pub selected_run_id: Option<String>,
    pub selected_conflict_index: Option<usize>,
    pub run_filter: AppRunFilter,
    pub setup: AppSetupState,
    pub report_filter: AppReportFilter,
    pub report_sort: AppReportSort,
    pub selected_report_index: Option<usize>,
    pub background_paused: bool,
    pub conflict_count: usize,
    pub last_message: Option<String>,
    pub last_run_at: Option<String>,
    pub last_run_status: Option<String>,
    pub next_run_at: Option<String>,
    pub recent_error: Option<String>,
    pub pairs: Vec<AppPair>,
    pub runs: Vec<AppRun>,
    pub report_rows: Vec<AppReportRow>,
    pub plan: Option<AppPlanSummary>,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppSetupState {
    pub secret_store: String,
    pub db_path: String,
    pub log_level: String,
    pub google_account_label: String,
    pub google_client_id_configured: bool,
    pub google_client_secret_inline: bool,
    pub google_refresh_token_inline: bool,
    pub icloud_account_label: String,
    pub icloud_username_configured: bool,
    pub icloud_app_password_inline: bool,
    pub icloud_caldav_url: String,
    pub poll_interval_seconds: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppSetupStep {
    pub label: String,
    pub status: AppSetupStepStatus,
    pub detail: String,
    pub next_action: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppSetupStepStatus {
    Complete,
    Attention,
    Missing,
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
    pub report_rows: Vec<AppReportRow>,
    pub plan: Option<AppPlanSummary>,
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
pub struct AppReportRow {
    pub pair_id: String,
    pub action: String,
    pub reason: String,
    pub resolution: String,
    pub title: String,
    pub canonical_uid: String,
    pub google_present: String,
    pub icloud_present: String,
    pub google_title: String,
    pub icloud_title: String,
    pub google_start: String,
    pub icloud_start: String,
    pub google_end: String,
    pub icloud_end: String,
    pub google_status: String,
    pub icloud_status: String,
    pub diff_fields: String,
}

/// Aggregate snapshot of the most recent dry-run/apply plan, surfaced as the
/// summary band on the Plan screen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppPlanSummary {
    pub mode: String,
    pub total_actions: usize,
    pub action_counts: BTreeMap<String, usize>,
    pub pair_counts: BTreeMap<String, usize>,
    pub generated_at: String,
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
    pub event_link_id: Option<String>,
    pub canonical_uid: Option<String>,
    pub reason: String,
    pub resolution_policy: String,
    pub google_title: Option<String>,
    pub icloud_title: Option<String>,
    pub google_start: Option<String>,
    pub icloud_start: Option<String>,
    pub google_status: Option<String>,
    pub icloud_status: Option<String>,
    pub google_event_id: Option<String>,
    pub icloud_href: Option<String>,
    pub diff_fields: String,
    pub created_at: String,
    pub queued_resolution: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppView {
    Dashboard,
    Setup,
    Runs,
    Reports,
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
pub enum AppReportFilter {
    All,
    Creates,
    Updates,
    Deletes,
    Conflicts,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppReportSort {
    Pair,
    Action,
    Title,
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
    ShowReports,
    ToggleBackgroundPause,
    ExportReport,
    Quit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppResolution {
    GoogleWins,
    IcloudWins,
    DeleteWins,
    UpdateWins,
}

impl AppResolution {
    pub fn label(self) -> &'static str {
        match self {
            Self::GoogleWins => "Google wins",
            Self::IcloudWins => "iCloud wins",
            Self::DeleteWins => "delete wins",
            Self::UpdateWins => "update wins",
        }
    }
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
    RequestApplyRun,
    ConfirmApplyRun,
    CancelApplyRun,
    StartDaemon,
    StopDaemon,
    OpenSetup,
    RefreshConflicts,
    SelectPair(String),
    ShowDashboard,
    ShowSetup,
    ShowRuns,
    ShowReports,
    ShowConflicts,
    CycleRunFilter,
    CycleReportFilter,
    CycleReportSort,
    SelectNextRun,
    SelectPreviousRun,
    SelectNextReportRow,
    SelectPreviousReportRow,
    SelectNextConflict,
    SelectPreviousConflict,
    ResolveSelectedConflict(AppResolution),
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
    ResolveConflict {
        conflict_ids: Vec<String>,
        resolution: AppResolution,
    },
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
            confirm_apply: false,
            selected_command_index: 0,
            selected_pair_id: config.sync.pairs.first().map(|pair| pair.id.clone()),
            selected_run_id: None,
            selected_conflict_index: None,
            run_filter: AppRunFilter::All,
            setup: AppSetupState::from_config(config),
            report_filter: AppReportFilter::All,
            report_sort: AppReportSort::Pair,
            selected_report_index: None,
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
            report_rows: Vec::new(),
            plan: None,
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

    pub fn setup_steps(&self) -> Vec<AppSetupStep> {
        self.setup.steps(self)
    }

    pub fn setup_ready_count(&self) -> usize {
        self.setup_steps()
            .iter()
            .filter(|step| step.status == AppSetupStepStatus::Complete)
            .count()
    }

    pub fn visible_report_rows(&self) -> Vec<&AppReportRow> {
        let mut rows = self
            .report_rows
            .iter()
            .filter(|row| self.report_filter.matches(row))
            .collect::<Vec<_>>();
        rows.sort_by(|left, right| self.report_sort.cmp(left, right));
        rows
    }

    pub fn selected_report_row(&self) -> Option<&AppReportRow> {
        let selected_index = self.selected_report_index?;
        self.visible_report_rows().get(selected_index).copied()
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
        self.report_rows = snapshot.report_rows;
        self.plan = snapshot.plan;
        self.ensure_selected_report_row();
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

    pub fn select_next_report_row(&mut self) {
        self.select_report_row_by_offset(1);
    }

    pub fn select_previous_report_row(&mut self) {
        self.select_report_row_by_offset(-1);
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
                self.view = AppView::Reports;
                self.status = AppStatus::Syncing;
                vec![AppEffect::RunDrySync]
            }
            AppEvent::StartApplyRun => {
                self.view = AppView::Reports;
                self.status = AppStatus::Syncing;
                vec![AppEffect::RunApplySync]
            }
            AppEvent::RequestApplyRun => {
                self.confirm_apply = true;
                Vec::new()
            }
            AppEvent::ConfirmApplyRun => {
                self.confirm_apply = false;
                self.update(AppEvent::StartApplyRun)
            }
            AppEvent::CancelApplyRun => {
                self.confirm_apply = false;
                Vec::new()
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
            AppEvent::RefreshConflicts => vec![AppEffect::LoadConflicts],
            AppEvent::SelectPair(pair_id) => {
                self.selected_pair_id = Some(pair_id);
                Vec::new()
            }
            AppEvent::ShowDashboard => {
                self.view = AppView::Dashboard;
                Vec::new()
            }
            AppEvent::ShowSetup | AppEvent::OpenSetup => {
                self.view = AppView::Setup;
                Vec::new()
            }
            AppEvent::ShowRuns => {
                self.view = AppView::Runs;
                self.ensure_selected_run();
                Vec::new()
            }
            AppEvent::ShowReports => {
                self.view = AppView::Reports;
                self.ensure_selected_report_row();
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
            AppEvent::CycleReportFilter => {
                self.report_filter = self.report_filter.next();
                self.ensure_selected_report_row();
                Vec::new()
            }
            AppEvent::CycleReportSort => {
                self.report_sort = self.report_sort.next();
                self.ensure_selected_report_row();
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
            AppEvent::SelectNextReportRow => {
                self.select_next_report_row();
                Vec::new()
            }
            AppEvent::SelectPreviousReportRow => {
                self.select_previous_report_row();
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
            AppEvent::ResolveSelectedConflict(resolution) => {
                let conflict_ids: Vec<String> = self
                    .selected_conflict_details()
                    .iter()
                    .map(|detail| detail.id.clone())
                    .collect();
                if conflict_ids.is_empty() {
                    Vec::new()
                } else {
                    vec![AppEffect::ResolveConflict {
                        conflict_ids,
                        resolution,
                    }]
                }
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
                self.recent_error = Some(message.clone());
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

    fn select_report_row_by_offset(&mut self, offset: isize) {
        let visible_count = self.visible_report_rows().len();
        if visible_count == 0 {
            self.selected_report_index = None;
            return;
        }

        let current = self
            .selected_report_index
            .filter(|index| *index < visible_count)
            .unwrap_or(0);
        let next = (current as isize + offset).rem_euclid(visible_count as isize) as usize;
        self.selected_report_index = Some(next);
    }

    fn ensure_selected_report_row(&mut self) {
        let visible_count = self.visible_report_rows().len();
        if visible_count == 0 {
            self.selected_report_index = None;
            return;
        }
        let selected = self
            .selected_report_index
            .filter(|index| *index < visible_count)
            .unwrap_or(0);
        self.selected_report_index = Some(selected);
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
            AppCommand::ApplyRun => self.update(AppEvent::RequestApplyRun),
            AppCommand::RefreshConflicts => self.update(AppEvent::RefreshConflicts),
            AppCommand::ShowConflicts => self.update(AppEvent::ShowConflicts),
            AppCommand::OpenSetup => self.update(AppEvent::ShowSetup),
            AppCommand::ShowPairs => self.update(AppEvent::ShowDashboard),
            AppCommand::ShowRuns => self.update(AppEvent::ShowRuns),
            AppCommand::ShowReports => self.update(AppEvent::ShowReports),
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

impl AppSetupState {
    fn from_config(config: &ServiceConfig) -> Self {
        Self {
            secret_store: match config.secret_store {
                SecretStoreKind::None => "none".to_string(),
                SecretStoreKind::Os => "os".to_string(),
            },
            db_path: config.db_path.display().to_string(),
            log_level: config.log_level.clone(),
            google_account_label: config.google.account_label.clone(),
            google_client_id_configured: config.google.client_id.as_deref().is_some_and(non_empty),
            google_client_secret_inline: config
                .google
                .client_secret
                .as_deref()
                .is_some_and(non_empty),
            google_refresh_token_inline: config
                .google
                .refresh_token
                .as_deref()
                .is_some_and(non_empty),
            icloud_account_label: config.icloud.account_label.clone(),
            icloud_username_configured: config.icloud.username.as_deref().is_some_and(non_empty),
            icloud_app_password_inline: config
                .icloud
                .app_specific_password
                .as_deref()
                .is_some_and(non_empty),
            icloud_caldav_url: config.icloud.caldav_url.clone(),
            poll_interval_seconds: config.sync.poll_interval_seconds,
        }
    }

    fn steps(&self, model: &AppModel) -> Vec<AppSetupStep> {
        vec![
            AppSetupStep {
                label: "Config".to_string(),
                status: AppSetupStepStatus::Complete,
                detail: format!(
                    "db {}, log {}, secrets {}",
                    self.db_path, self.log_level, self.secret_store
                ),
                next_action: "Run insync doctor after changes".to_string(),
            },
            self.google_step(),
            self.icloud_step(),
            self.discovery_step(model),
            self.pair_step(model),
            self.doctor_step(model),
        ]
    }

    fn google_step(&self) -> AppSetupStep {
        let has_os_secret_hint = self.secret_store == "os";
        let status = if self.google_client_id_configured
            && (self.google_refresh_token_inline || has_os_secret_hint)
        {
            AppSetupStepStatus::Complete
        } else if self.google_client_id_configured {
            AppSetupStepStatus::Attention
        } else {
            AppSetupStepStatus::Missing
        };
        AppSetupStep {
            label: "Google OAuth".to_string(),
            status,
            detail: format!(
                "account {}, client id {}, refresh token {}",
                self.google_account_label,
                readiness(self.google_client_id_configured),
                secret_readiness(self.google_refresh_token_inline, has_os_secret_hint)
            ),
            next_action: "Run insync setup --google-callback or --google-code".to_string(),
        }
    }

    fn icloud_step(&self) -> AppSetupStep {
        let has_os_secret_hint = self.secret_store == "os";
        let status = if self.icloud_username_configured
            && (self.icloud_app_password_inline || has_os_secret_hint)
        {
            AppSetupStepStatus::Complete
        } else if self.icloud_username_configured {
            AppSetupStepStatus::Attention
        } else {
            AppSetupStepStatus::Missing
        };
        AppSetupStep {
            label: "iCloud".to_string(),
            status,
            detail: format!(
                "account {}, username {}, password {}, CalDAV {}",
                self.icloud_account_label,
                readiness(self.icloud_username_configured),
                secret_readiness(self.icloud_app_password_inline, has_os_secret_hint),
                self.icloud_caldav_url
            ),
            next_action: "Run insync setup --icloud-username ... --icloud-app-password ..."
                .to_string(),
        }
    }

    fn discovery_step(&self, model: &AppModel) -> AppSetupStep {
        let cached_pairs = model
            .pairs
            .iter()
            .filter(|pair| {
                pair.google_calendar_name.is_some() || pair.icloud_calendar_name.is_some()
            })
            .count();
        let status = if cached_pairs == model.pairs.len() && !model.pairs.is_empty() {
            AppSetupStepStatus::Complete
        } else if !model.pairs.is_empty() {
            AppSetupStepStatus::Attention
        } else {
            AppSetupStepStatus::Missing
        };
        AppSetupStep {
            label: "Discovery".to_string(),
            status,
            detail: format!(
                "{cached_pairs}/{} pairs have cached calendar metadata",
                model.pairs.len()
            ),
            next_action: "Run insync setup --discover".to_string(),
        }
    }

    fn pair_step(&self, model: &AppModel) -> AppSetupStep {
        let status = if model.enabled_pair_count() > 0 {
            AppSetupStepStatus::Complete
        } else if model.pairs.is_empty() {
            AppSetupStepStatus::Missing
        } else {
            AppSetupStepStatus::Attention
        };
        AppSetupStep {
            label: "Calendar Pair".to_string(),
            status,
            detail: format!(
                "{} configured, {} enabled",
                model.pairs.len(),
                model.enabled_pair_count()
            ),
            next_action:
                "Run insync setup --pair-id ... --google-calendar-id ... --icloud-calendar-id ..."
                    .to_string(),
        }
    }

    fn doctor_step(&self, model: &AppModel) -> AppSetupStep {
        let status = if model.recent_error.is_some() {
            AppSetupStepStatus::Attention
        } else if model.last_run_status.as_deref() == Some("completed") {
            AppSetupStepStatus::Complete
        } else {
            AppSetupStepStatus::Missing
        };
        AppSetupStep {
            label: "Doctor / Dry Run".to_string(),
            status,
            detail: model
                .last_run_status
                .as_ref()
                .map(|status| format!("last run {status}"))
                .unwrap_or_else(|| "no successful dry-run recorded in this session".to_string()),
            next_action: "Run insync doctor, then insync sync --report .insync/reports/dry-run.csv"
                .to_string(),
        }
    }
}

impl Default for AppSetupState {
    fn default() -> Self {
        Self::from_config(&ServiceConfig::default())
    }
}

fn non_empty(value: &str) -> bool {
    !value.trim().is_empty()
}

fn readiness(ready: bool) -> &'static str {
    if ready { "ready" } else { "missing" }
}

fn secret_readiness(inline: bool, os_store: bool) -> &'static str {
    if inline {
        "inline"
    } else if os_store {
        "os store"
    } else {
        "missing"
    }
}

impl AppCommand {
    const ALL: [Self; 11] = [
        Self::DryRun,
        Self::ApplyRun,
        Self::RefreshConflicts,
        Self::ShowConflicts,
        Self::OpenSetup,
        Self::ShowPairs,
        Self::ShowRuns,
        Self::ShowReports,
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
            Self::OpenSetup => "Setup wizard",
            Self::ShowPairs => "Show pairs",
            Self::ShowRuns => "Show sync runs",
            Self::ShowReports => "Show dry-run report",
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
            Self::OpenSetup => "Open the guided setup checklist",
            Self::ShowPairs => "Return to the calendar-pair dashboard",
            Self::ShowRuns => "Open recent sync-run history",
            Self::ShowReports => "Open the latest dry-run report rows",
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
            | Self::ShowReports
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

impl AppReportFilter {
    pub fn label(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Creates => "creates",
            Self::Updates => "updates",
            Self::Deletes => "deletes",
            Self::Conflicts => "conflicts",
        }
    }

    fn matches(self, row: &AppReportRow) -> bool {
        let action = row.action.as_str();
        let reason = row.reason.as_str();
        let resolution = row.resolution.as_str();
        match self {
            Self::All => true,
            Self::Creates => action.contains("create"),
            Self::Updates => action.contains("update"),
            Self::Deletes => action.contains("delete"),
            Self::Conflicts => {
                action.contains("conflict")
                    || reason.contains("conflict")
                    || resolution.contains("manual")
            }
        }
    }

    fn next(self) -> Self {
        match self {
            Self::All => Self::Creates,
            Self::Creates => Self::Updates,
            Self::Updates => Self::Deletes,
            Self::Deletes => Self::Conflicts,
            Self::Conflicts => Self::All,
        }
    }
}

impl AppReportSort {
    pub fn label(self) -> &'static str {
        match self {
            Self::Pair => "pair",
            Self::Action => "action",
            Self::Title => "title",
        }
    }

    fn cmp(self, left: &AppReportRow, right: &AppReportRow) -> std::cmp::Ordering {
        match self {
            Self::Pair => left
                .pair_id
                .cmp(&right.pair_id)
                .then_with(|| left.action.cmp(&right.action))
                .then_with(|| left.title.cmp(&right.title)),
            Self::Action => left
                .action
                .cmp(&right.action)
                .then_with(|| left.pair_id.cmp(&right.pair_id))
                .then_with(|| left.title.cmp(&right.title)),
            Self::Title => left
                .title
                .cmp(&right.title)
                .then_with(|| left.pair_id.cmp(&right.pair_id))
                .then_with(|| left.action.cmp(&right.action)),
        }
    }

    fn next(self) -> Self {
        match self {
            Self::Pair => Self::Action,
            Self::Action => Self::Title,
            Self::Title => Self::Pair,
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
            confirm_apply: false,
            selected_command_index: 0,
            selected_pair_id: None,
            selected_run_id: None,
            selected_conflict_index: None,
            run_filter: AppRunFilter::All,
            setup: AppSetupState::default(),
            report_filter: AppReportFilter::All,
            report_sort: AppReportSort::Pair,
            selected_report_index: None,
            background_paused: false,
            conflict_count: 0,
            last_message: None,
            last_run_at: None,
            last_run_status: None,
            next_run_at: None,
            recent_error: None,
            pairs: Vec::new(),
            runs: Vec::new(),
            report_rows: Vec::new(),
            plan: None,
            conflict_summaries: Vec::new(),
            conflict_details: Vec::new(),
        };

        let effects = model.update(AppEvent::StartDryRun);

        assert_eq!(model.status, AppStatus::Syncing);
        assert_eq!(model.view, AppView::Reports);
        assert_eq!(effects, vec![AppEffect::RunDrySync]);
    }

    #[test]
    fn pair_navigation_wraps() {
        let mut model = AppModel {
            status: AppStatus::Idle,
            view: AppView::Dashboard,
            command_palette_open: false,
            confirm_apply: false,
            selected_command_index: 0,
            selected_pair_id: Some("a".to_string()),
            selected_run_id: None,
            selected_conflict_index: None,
            run_filter: AppRunFilter::All,
            setup: AppSetupState::default(),
            report_filter: AppReportFilter::All,
            report_sort: AppReportSort::Pair,
            selected_report_index: None,
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
            report_rows: Vec::new(),
            plan: None,
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
            confirm_apply: false,
            selected_command_index: 0,
            selected_pair_id: None,
            selected_run_id: None,
            selected_conflict_index: None,
            run_filter: AppRunFilter::All,
            setup: AppSetupState::default(),
            report_filter: AppReportFilter::All,
            report_sort: AppReportSort::Pair,
            selected_report_index: None,
            background_paused: false,
            conflict_count: 0,
            last_message: None,
            last_run_at: None,
            last_run_status: None,
            next_run_at: None,
            recent_error: None,
            pairs: Vec::new(),
            runs: Vec::new(),
            report_rows: Vec::new(),
            plan: None,
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
            report_rows: Vec::new(),
            plan: None,
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
            confirm_apply: false,
            selected_command_index: 0,
            selected_pair_id: Some("a".to_string()),
            selected_run_id: None,
            selected_conflict_index: None,
            run_filter: AppRunFilter::All,
            setup: AppSetupState::default(),
            report_filter: AppReportFilter::All,
            report_sort: AppReportSort::Pair,
            selected_report_index: None,
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
            report_rows: Vec::new(),
            plan: None,
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
            report_rows: Vec::new(),
            plan: None,
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
            confirm_apply: false,
            selected_command_index: 0,
            selected_pair_id: None,
            selected_run_id: None,
            selected_conflict_index: None,
            run_filter: AppRunFilter::All,
            setup: AppSetupState::default(),
            report_filter: AppReportFilter::All,
            report_sort: AppReportSort::Pair,
            selected_report_index: None,
            background_paused: false,
            conflict_count: 0,
            last_message: None,
            last_run_at: None,
            last_run_status: None,
            next_run_at: None,
            recent_error: None,
            pairs: Vec::new(),
            runs: Vec::new(),
            report_rows: Vec::new(),
            plan: None,
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
    fn report_view_filters_sorts_and_selects_rows() {
        let mut model = AppModel::from_config(&ServiceConfig::default());

        model.apply_runtime_snapshot(AppRuntimeSnapshot {
            report_rows: vec![
                AppReportRow {
                    pair_id: "work".to_string(),
                    action: "update_google".to_string(),
                    reason: "google_changed".to_string(),
                    resolution: "apply".to_string(),
                    title: "Budget review".to_string(),
                    canonical_uid: "uid-work".to_string(),
                    google_present: "yes".to_string(),
                    icloud_present: "yes".to_string(),
                    google_title: "Budget review".to_string(),
                    icloud_title: "Budget review".to_string(),
                    google_start: String::new(),
                    icloud_start: String::new(),
                    google_end: String::new(),
                    icloud_end: String::new(),
                    google_status: String::new(),
                    icloud_status: String::new(),
                    diff_fields: "title,start".to_string(),
                },
                AppReportRow {
                    pair_id: "personal".to_string(),
                    action: "create_icloud".to_string(),
                    reason: "missing_icloud".to_string(),
                    resolution: "apply".to_string(),
                    title: "Dentist".to_string(),
                    canonical_uid: "uid-dentist".to_string(),
                    google_present: "yes".to_string(),
                    icloud_present: "no".to_string(),
                    google_title: "Dentist".to_string(),
                    icloud_title: String::new(),
                    google_start: String::new(),
                    icloud_start: String::new(),
                    google_end: String::new(),
                    icloud_end: String::new(),
                    google_status: String::new(),
                    icloud_status: String::new(),
                    diff_fields: String::new(),
                },
                AppReportRow {
                    pair_id: "personal".to_string(),
                    action: "manual_conflict".to_string(),
                    reason: "both_sides_changed".to_string(),
                    resolution: "manual".to_string(),
                    title: "Planning".to_string(),
                    canonical_uid: "uid-planning".to_string(),
                    google_present: "yes".to_string(),
                    icloud_present: "yes".to_string(),
                    google_title: "Planning".to_string(),
                    icloud_title: "Planning".to_string(),
                    google_start: String::new(),
                    icloud_start: String::new(),
                    google_end: String::new(),
                    icloud_end: String::new(),
                    google_status: String::new(),
                    icloud_status: String::new(),
                    diff_fields: "title".to_string(),
                },
            ],
            ..AppRuntimeSnapshot::default()
        });

        model.update(AppEvent::ShowReports);
        assert_eq!(model.view, AppView::Reports);
        assert_eq!(model.selected_report_row().unwrap().pair_id, "personal");

        model.update(AppEvent::CycleReportFilter);
        assert_eq!(model.report_filter, AppReportFilter::Creates);
        assert_eq!(model.visible_report_rows().len(), 1);
        assert_eq!(model.selected_report_row().unwrap().title, "Dentist");

        model.update(AppEvent::CycleReportFilter);
        assert_eq!(model.report_filter, AppReportFilter::Updates);
        assert_eq!(model.selected_report_row().unwrap().title, "Budget review");

        model.update(AppEvent::CycleReportSort);
        assert_eq!(model.report_sort, AppReportSort::Action);
        model.update(AppEvent::SelectNextReportRow);
        assert_eq!(model.selected_report_index, Some(0));
    }

    #[test]
    fn conflict_view_selects_groups_and_filters_details() {
        let mut model = AppModel {
            status: AppStatus::Idle,
            view: AppView::Dashboard,
            command_palette_open: false,
            confirm_apply: false,
            selected_command_index: 0,
            selected_pair_id: None,
            selected_run_id: None,
            selected_conflict_index: None,
            run_filter: AppRunFilter::All,
            setup: AppSetupState::default(),
            report_filter: AppReportFilter::All,
            report_sort: AppReportSort::Pair,
            selected_report_index: None,
            background_paused: false,
            conflict_count: 0,
            last_message: None,
            last_run_at: None,
            last_run_status: None,
            next_run_at: None,
            recent_error: None,
            pairs: Vec::new(),
            runs: Vec::new(),
            report_rows: Vec::new(),
            plan: None,
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
                    event_link_id: Some("link-a".to_string()),
                    canonical_uid: Some("uid-a".to_string()),
                    reason: "both_sides_changed".to_string(),
                    resolution_policy: "manual review".to_string(),
                    google_title: Some("Google title".to_string()),
                    icloud_title: Some("iCloud title".to_string()),
                    google_start: Some("2026-06-02T12:00:00Z".to_string()),
                    icloud_start: Some("2026-06-02T13:00:00Z".to_string()),
                    google_status: Some("confirmed".to_string()),
                    icloud_status: Some("tentative".to_string()),
                    google_event_id: Some("google-a".to_string()),
                    icloud_href: Some("/icloud-a.ics".to_string()),
                    diff_fields: "title|start|status".to_string(),
                    created_at: "2026-06-02 12:01:00".to_string(),
                    queued_resolution: None,
                },
                AppConflictDetail {
                    id: "b".to_string(),
                    pair_id: "work".to_string(),
                    event_link_id: None,
                    canonical_uid: Some("uid-b".to_string()),
                    reason: "icloud_uid_exists".to_string(),
                    resolution_policy: "ignore known collision or choose manual".to_string(),
                    google_title: None,
                    icloud_title: None,
                    google_start: None,
                    icloud_start: None,
                    google_status: None,
                    icloud_status: None,
                    google_event_id: None,
                    icloud_href: None,
                    diff_fields: "metadata".to_string(),
                    created_at: "2026-06-02 13:00:00".to_string(),
                    queued_resolution: None,
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

        let effects = model.update(AppEvent::ResolveSelectedConflict(AppResolution::GoogleWins));
        assert_eq!(
            effects,
            vec![AppEffect::ResolveConflict {
                conflict_ids: vec!["b".to_string()],
                resolution: AppResolution::GoogleWins,
            }]
        );
    }

    #[test]
    fn command_palette_selects_and_executes_actions() {
        let mut model = AppModel {
            status: AppStatus::Idle,
            view: AppView::Dashboard,
            command_palette_open: false,
            confirm_apply: false,
            selected_command_index: 0,
            selected_pair_id: None,
            selected_run_id: None,
            selected_conflict_index: None,
            run_filter: AppRunFilter::All,
            setup: AppSetupState::default(),
            report_filter: AppReportFilter::All,
            report_sort: AppReportSort::Pair,
            selected_report_index: None,
            background_paused: false,
            conflict_count: 0,
            last_message: None,
            last_run_at: None,
            last_run_status: None,
            next_run_at: None,
            recent_error: None,
            pairs: Vec::new(),
            runs: Vec::new(),
            report_rows: Vec::new(),
            plan: None,
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
            confirm_apply: false,
            selected_command_index: 0,
            selected_pair_id: None,
            selected_run_id: None,
            selected_conflict_index: None,
            run_filter: AppRunFilter::All,
            setup: AppSetupState::default(),
            report_filter: AppReportFilter::All,
            report_sort: AppReportSort::Pair,
            selected_report_index: None,
            background_paused: false,
            conflict_count: 2,
            last_message: Some("ready".to_string()),
            last_run_at: Some("2026-06-02 12:00:00".to_string()),
            last_run_status: Some("failed".to_string()),
            next_run_at: Some("2026-06-02 12:05:00".to_string()),
            recent_error: Some("auth failed".to_string()),
            pairs: Vec::new(),
            runs: Vec::new(),
            report_rows: Vec::new(),
            plan: None,
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
        assert_eq!(effects, Vec::<AppEffect>::new());
        assert_eq!(model.view, AppView::Setup);
    }

    #[test]
    fn apply_command_opens_confirmation_then_runs() {
        let mut config = ServiceConfig::default();
        config.sync.pairs = vec![insync_config::SyncPairConfig {
            id: "personal".to_string(),
            enabled: true,
            direction: SyncDirection::TwoWay,
            google_calendar_id: "primary".to_string(),
            icloud_calendar_id: "https://caldav.example/cal".to_string(),
        }];
        let mut model = AppModel::from_config(&config);

        let effects = model.update(AppEvent::ExecuteCommand(AppCommand::ApplyRun));
        assert!(effects.is_empty());
        assert!(model.confirm_apply);
        assert_eq!(model.status, AppStatus::Idle);

        model.update(AppEvent::CancelApplyRun);
        assert!(!model.confirm_apply);
        assert_eq!(model.status, AppStatus::Idle);

        model.update(AppEvent::ExecuteCommand(AppCommand::ApplyRun));
        let effects = model.update(AppEvent::ConfirmApplyRun);
        assert!(!model.confirm_apply);
        assert_eq!(model.status, AppStatus::Syncing);
        assert_eq!(effects, vec![AppEffect::RunApplySync]);
    }

    #[test]
    fn setup_steps_surface_readiness_and_next_actions() {
        let mut config = ServiceConfig::default();
        config.secret_store = SecretStoreKind::Os;
        config.google.client_id = Some("client-id".to_string());
        config.icloud.username = Some("me@icloud.com".to_string());
        config.sync.pairs = vec![insync_config::SyncPairConfig {
            id: "personal".to_string(),
            enabled: true,
            direction: SyncDirection::TwoWay,
            google_calendar_id: "primary".to_string(),
            icloud_calendar_id: "https://caldav.example/cal".to_string(),
        }];
        let mut model = AppModel::from_config(&config);

        let steps = model.setup_steps();
        assert_eq!(steps.len(), 6);
        assert_eq!(steps[0].status, AppSetupStepStatus::Complete);
        assert_eq!(steps[1].status, AppSetupStepStatus::Complete);
        assert_eq!(steps[2].status, AppSetupStepStatus::Complete);
        assert_eq!(steps[3].status, AppSetupStepStatus::Attention);
        assert_eq!(steps[4].status, AppSetupStepStatus::Complete);
        assert_eq!(steps[5].status, AppSetupStepStatus::Missing);

        model.pairs[0].google_calendar_name = Some("Primary".to_string());
        model.pairs[0].icloud_calendar_name = Some("Home".to_string());
        model.last_run_status = Some("completed".to_string());
        let steps = model.setup_steps();
        assert_eq!(steps[3].status, AppSetupStepStatus::Complete);
        assert_eq!(steps[5].status, AppSetupStepStatus::Complete);
        assert_eq!(model.setup_ready_count(), 6);
    }
}
