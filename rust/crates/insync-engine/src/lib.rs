use chrono::Utc;
use insync_config::{ConfigError, ServiceConfig, SyncPairConfig, credentials::resolve_credentials};
use insync_core::{
    CanonicalEvent, ConflictPolicy, EventDateTime, EventStatus, MutatingAction, PlannedAction,
    PlannerConflictPolicies, ProviderEventMeta, ProviderName, hash_canonical_event,
    plan_two_way_actions,
};
use insync_db::{
    DbError, migrate, open,
    repositories::{
        configured_pairs::{configured_calendar_ids, seed_configured_pairs},
        event_links::{EventLinkUpsert, load_event_links, upsert_event_link},
        sync_conflicts::{
            ActiveConflict, RecordConflictInput, dedupe_unresolved_conflicts,
            list_unresolved_conflict_summaries, list_unresolved_conflicts,
            load_unresolved_conflict_uids, record_conflict, resolve_stale_conflicts,
        },
        sync_runs::{
            SyncRun, SyncRunStatus, complete_sync_run, fail_sync_run, latest_sync_run,
            start_sync_run,
        },
        sync_state::{
            clear_calendar_sync_token, load_calendar_sync_token, update_calendar_sync_token,
        },
    },
};
use insync_providers::{CalendarProvider, ProviderChangeSet, ProviderError, ProviderSyncCursor};
use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    time::Duration,
};
use thiserror::Error;
use tokio::time;

pub use insync_db::repositories::sync_conflicts::{
    ConflictFilter, UnresolvedConflictRow, UnresolvedConflictSummary,
};

#[derive(Debug, Error)]
pub enum EngineError {
    #[error("database error: {0}")]
    Db(#[from] DbError),
    #[error("config error: {0}")]
    Config(#[from] ConfigError),
    #[error("provider error: {0}")]
    Provider(#[from] ProviderError),
    #[error("failed to write report {path}: {source}")]
    ReportWrite {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("sync lock is already held: {0}")]
    LockAlreadyHeld(PathBuf),
    #[error("failed to create sync lock {path}: {source}")]
    LockCreate {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("cannot apply {action} for {canonical_uid}: missing {field}")]
    MissingApplyMetadata {
        action: &'static str,
        canonical_uid: String,
        field: &'static str,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunMode {
    DryRun,
    Apply,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportMode {
    ActionsOnly,
    AllActions,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DoctorSummary {
    pub db_path: PathBuf,
    pub configured_pair_count: usize,
    pub enabled_pair_count: usize,
    pub unresolved_conflict_count: i64,
    pub google_credentials_configured: bool,
    pub icloud_credentials_configured: bool,
    pub latest_run: Option<SyncRunSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncRunSummary {
    pub id: String,
    pub sync_pair_id: Option<String>,
    pub status: String,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncSummary {
    pub db_path: PathBuf,
    pub configured_pair_count: usize,
    pub enabled_pair_count: usize,
    pub unresolved_conflict_count: i64,
    pub known_icloud_uid_collision_count: usize,
    pub google_credentials_configured: bool,
    pub icloud_credentials_configured: bool,
    pub mode: RunMode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedSyncSummary {
    pub db_path: PathBuf,
    pub configured_pair_count: usize,
    pub enabled_pair_count: usize,
    pub pair_summaries: Vec<PairPlanSummary>,
    pub action_counts: BTreeMap<String, usize>,
    pub resolution_counts: BTreeMap<String, usize>,
    pub report_rows: Vec<ReportRow>,
    pub mode: RunMode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportRow {
    pub pair_id: String,
    pub action: String,
    pub canonical_uid: String,
    pub reason: String,
    pub resolution: String,
    pub conflict_policy: String,
    pub title: String,
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
    pub google_hash: String,
    pub icloud_hash: String,
    pub diff_fields: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairPlanSummary {
    pub pair_id: String,
    pub google_events: usize,
    pub icloud_events: usize,
    pub actions: usize,
    pub action_counts: BTreeMap<String, usize>,
    pub resolution_counts: BTreeMap<String, usize>,
}

#[derive(Clone, Copy)]
pub struct SyncProviders<'a> {
    pub google: &'a dyn CalendarProvider,
    pub icloud: &'a dyn CalendarProvider,
}

#[derive(Debug, Clone)]
pub struct SyncEngine {
    config: ServiceConfig,
    config_path: Option<PathBuf>,
}

impl SyncEngine {
    pub fn new(config: ServiceConfig) -> Self {
        Self {
            config,
            config_path: None,
        }
    }

    pub fn with_config_path(config: ServiceConfig, config_path: impl Into<PathBuf>) -> Self {
        Self {
            config,
            config_path: Some(config_path.into()),
        }
    }

    pub fn db_path(&self) -> PathBuf {
        resolve_db_path(&self.config.db_path, self.config_path.as_deref())
    }

    pub fn doctor(&self) -> Result<DoctorSummary, EngineError> {
        let (db_path, conn) = self.prepare_database()?;
        let unresolved_conflict_count = list_unresolved_conflict_summaries(&conn)?
            .into_iter()
            .map(|summary| summary.count)
            .sum();
        let credential_summary = self.credential_summary()?;

        Ok(DoctorSummary {
            db_path,
            configured_pair_count: self.config.sync.pairs.len(),
            enabled_pair_count: enabled_pair_count(&self.config),
            unresolved_conflict_count,
            google_credentials_configured: credential_summary.google,
            icloud_credentials_configured: credential_summary.icloud,
            latest_run: latest_sync_run(&conn)?.map(sync_run_summary),
        })
    }

    pub async fn run_once(&self, mode: RunMode) -> Result<SyncSummary, EngineError> {
        let (db_path, conn) = self.prepare_database()?;
        let _lock = acquire_sync_lock(&db_path)?;
        let run = start_sync_run(&conn, None)?;

        let result: Result<SyncSummary, EngineError> = (|| {
            let unresolved_conflict_count = list_unresolved_conflict_summaries(&conn)?
                .into_iter()
                .map(|summary| summary.count)
                .sum();
            let mut known_icloud_uid_collision_count = 0;

            for pair in self.config.sync.pairs.iter().filter(|pair| pair.enabled) {
                known_icloud_uid_collision_count += load_unresolved_conflict_uids(
                    &conn,
                    &pair.id,
                    "icloud_uid_exists_in_different_calendar",
                )?
                .len();
            }
            let credential_summary = self.credential_summary()?;

            tracing::info!(
                configured_pair_count = self.config.sync.pairs.len(),
                enabled_pair_count = enabled_pair_count(&self.config),
                unresolved_conflict_count,
                known_icloud_uid_collision_count,
                google_credentials_configured = credential_summary.google,
                icloud_credentials_configured = credential_summary.icloud,
                ?mode,
                db_path = %db_path.display(),
                "sync engine scaffold prepared database"
            );

            Ok(SyncSummary {
                db_path,
                configured_pair_count: self.config.sync.pairs.len(),
                enabled_pair_count: enabled_pair_count(&self.config),
                unresolved_conflict_count,
                known_icloud_uid_collision_count,
                google_credentials_configured: credential_summary.google,
                icloud_credentials_configured: credential_summary.icloud,
                mode,
            })
        })();

        match result {
            Ok(summary) => {
                complete_sync_run(&conn, &run.id)?;
                Ok(summary)
            }
            Err(error) => {
                fail_sync_run(&conn, &run.id, &error.to_string())?;
                Err(error)
            }
        }
    }

    pub async fn plan_once_with_providers(
        &self,
        mode: RunMode,
        providers: SyncProviders<'_>,
    ) -> Result<PlannedSyncSummary, EngineError> {
        self.plan_once_with_providers_and_report_mode(mode, providers, ReportMode::ActionsOnly)
            .await
    }

    pub async fn plan_once_with_providers_and_report_mode(
        &self,
        mode: RunMode,
        providers: SyncProviders<'_>,
        report_mode: ReportMode,
    ) -> Result<PlannedSyncSummary, EngineError> {
        let (db_path, conn) = self.prepare_database()?;
        let _lock = acquire_sync_lock(&db_path)?;
        let run = if mode == RunMode::Apply {
            Some(start_sync_run(&conn, None)?)
        } else {
            None
        };
        let mut pair_summaries = Vec::new();
        let mut action_counts = BTreeMap::new();
        let mut resolution_counts = BTreeMap::new();
        let mut report_rows = Vec::new();

        let result: Result<PlannedSyncSummary, EngineError> = async {
            for pair in self.config.sync.pairs.iter().filter(|pair| pair.enabled) {
                let ids = configured_calendar_ids(&self.config, pair);
                let google_sync_token = load_calendar_sync_token(&conn, &ids.google_calendar_id)?;
                let icloud_sync_token = load_calendar_sync_token(&conn, &ids.icloud_calendar_id)?;
                let google_changes = get_changes_with_token_recovery(
                    &conn,
                    providers.google,
                    &ids.google_calendar_id,
                    &pair.google_calendar_id,
                    google_sync_token,
                )
                .await?;
                let icloud_changes = get_changes_with_token_recovery(
                    &conn,
                    providers.icloud,
                    &ids.icloud_calendar_id,
                    &pair.icloud_calendar_id,
                    icloud_sync_token,
                )
                .await?;
                let links = load_event_links(&conn, &pair.id)?;
                let known_icloud_uid_collisions = load_unresolved_conflict_uids(
                    &conn,
                    &pair.id,
                    "icloud_uid_exists_in_different_calendar",
                )?;
                let actions = plan_two_way_actions(insync_core::PlanTwoWayInput {
                    links: &links,
                    google_events: &google_changes.events,
                    icloud_events: &icloud_changes.events,
                    known_icloud_uid_collisions,
                    direction: pair.direction,
                    conflict_policy: planner_conflict_policies(&self.config),
                });

                if mode == RunMode::Apply {
                    apply_actions(&conn, &self.config, pair, providers, &actions).await?;
                    resolve_stale_conflicts(&conn, &pair.id, &active_manual_conflicts(&actions))?;
                    update_calendar_sync_token(
                        &conn,
                        &ids.google_calendar_id,
                        google_changes.sync_token.as_deref(),
                    )?;
                    update_calendar_sync_token(
                        &conn,
                        &ids.icloud_calendar_id,
                        icloud_changes.sync_token.as_deref(),
                    )?;
                }

                let pair_action_counts = count_actions(&actions);
                let pair_resolution_counts = count_resolutions(&actions);
                merge_counts(&mut action_counts, &pair_action_counts);
                merge_counts(&mut resolution_counts, &pair_resolution_counts);
                report_rows.extend(
                    actions
                        .iter()
                        .filter(|action| should_include_report_action(action, report_mode))
                        .map(|action| action_to_report_row(&pair.id, action)),
                );

                pair_summaries.push(PairPlanSummary {
                    pair_id: pair.id.clone(),
                    google_events: google_changes.events.len(),
                    icloud_events: icloud_changes.events.len(),
                    actions: actions.len(),
                    action_counts: pair_action_counts,
                    resolution_counts: pair_resolution_counts,
                });
            }

            Ok(PlannedSyncSummary {
                db_path,
                configured_pair_count: self.config.sync.pairs.len(),
                enabled_pair_count: enabled_pair_count(&self.config),
                pair_summaries,
                action_counts,
                resolution_counts,
                report_rows,
                mode,
            })
        }
        .await;

        match (result, run) {
            (Ok(summary), Some(run)) => {
                complete_sync_run(&conn, &run.id)?;
                Ok(summary)
            }
            (Err(error), Some(run)) => {
                fail_sync_run(&conn, &run.id, &error.to_string())?;
                Err(error)
            }
            (result, None) => result,
        }
    }

    pub async fn run_forever(
        &self,
        mode: RunMode,
        shutdown: impl std::future::Future<Output = ()>,
    ) -> Result<(), EngineError> {
        let interval = Duration::from_secs(self.config.sync.poll_interval_seconds);
        tokio::pin!(shutdown);

        loop {
            self.run_once(mode).await?;

            tokio::select! {
                _ = &mut shutdown => return Ok(()),
                _ = time::sleep(interval) => {}
            }
        }
    }

    pub fn conflict_summaries(&self) -> Result<Vec<UnresolvedConflictSummary>, EngineError> {
        let (_, conn) = self.prepare_database()?;
        Ok(list_unresolved_conflict_summaries(&conn)?)
    }

    pub fn conflict_details(
        &self,
        filter: ConflictFilter,
    ) -> Result<Vec<UnresolvedConflictRow>, EngineError> {
        let (_, conn) = self.prepare_database()?;
        Ok(list_unresolved_conflicts(&conn, filter)?)
    }

    pub fn dedupe_conflicts(&self) -> Result<usize, EngineError> {
        let (_, conn) = self.prepare_database()?;
        Ok(dedupe_unresolved_conflicts(&conn)?)
    }

    pub fn write_dry_run_report(
        &self,
        path: impl AsRef<Path>,
        rows: &[ReportRow],
    ) -> Result<(), EngineError> {
        write_dry_run_report(path, rows)
    }

    fn prepare_database(&self) -> Result<(PathBuf, rusqlite::Connection), EngineError> {
        let db_path = self.db_path();
        let conn = open(&db_path)?;
        migrate(&conn)?;
        seed_configured_pairs(&conn, &self.config)?;
        Ok((db_path, conn))
    }

    fn credential_summary(&self) -> Result<CredentialSummary, EngineError> {
        let mut config = self.config.clone();
        let config_path = self
            .config_path
            .as_deref()
            .unwrap_or_else(|| Path::new("insync.local.json"));
        let credentials = resolve_credentials(&mut config, config_path)?;

        Ok(CredentialSummary {
            google: credentials.google.client_id.is_some()
                && credentials.google.client_secret.is_some()
                && credentials.google.refresh_token.is_some(),
            icloud: credentials.icloud.username.is_some()
                && credentials.icloud.app_specific_password.is_some(),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CredentialSummary {
    google: bool,
    icloud: bool,
}

async fn get_changes_with_token_recovery(
    conn: &rusqlite::Connection,
    provider: &dyn CalendarProvider,
    db_calendar_id: &str,
    provider_calendar_id: &str,
    sync_token: Option<String>,
) -> Result<ProviderChangeSet, EngineError> {
    let first = provider
        .get_changes(
            provider_calendar_id,
            ProviderSyncCursor {
                full_sync: sync_token.is_none(),
                sync_token: sync_token.clone(),
            },
        )
        .await;

    match first {
        Err(ProviderError::SyncTokenExpired(_)) if sync_token.is_some() => {
            clear_calendar_sync_token(conn, db_calendar_id)?;
            Ok(provider
                .get_changes(
                    provider_calendar_id,
                    ProviderSyncCursor {
                        full_sync: true,
                        sync_token: None,
                    },
                )
                .await?)
        }
        result => Ok(result?),
    }
}

async fn apply_actions(
    conn: &rusqlite::Connection,
    config: &ServiceConfig,
    pair: &SyncPairConfig,
    providers: SyncProviders<'_>,
    actions: &[PlannedAction],
) -> Result<(), EngineError> {
    for action in actions {
        match action {
            PlannedAction::Snapshot {
                canonical_uid,
                link,
                google,
                icloud,
                google_hash,
                icloud_hash,
                ..
            } => {
                upsert_event_link(
                    conn,
                    event_link_upsert(
                        &pair.id,
                        canonical_uid,
                        link.as_ref(),
                        google.as_deref().map(|event| &event.provider_meta),
                        icloud.as_deref().map(|event| &event.provider_meta),
                        google_hash
                            .clone()
                            .or_else(|| google.as_deref().map(hash_canonical_event)),
                        icloud_hash
                            .clone()
                            .or_else(|| icloud.as_deref().map(hash_canonical_event)),
                        synced_hash(google_hash.as_deref(), icloud_hash.as_deref()),
                        None,
                        None,
                    ),
                )?;
            }
            PlannedAction::Noop { .. } => {}
            PlannedAction::CreateGoogle(action) => {
                let meta = providers
                    .google
                    .create_event(&pair.google_calendar_id, &action.event)
                    .await?;
                upsert_event_link(conn, link_after_google_write(&pair.id, action, meta))?;
            }
            PlannedAction::CreateIcloud(action) => {
                let meta = providers
                    .icloud
                    .create_event(&pair.icloud_calendar_id, &action.event)
                    .await?;
                upsert_event_link(conn, link_after_icloud_write(&pair.id, action, meta))?;
            }
            PlannedAction::UpdateGoogle(action) => {
                let remote_event_id = google_remote_event_id(action, "update_google")?;
                let meta = providers
                    .google
                    .update_event(
                        &pair.google_calendar_id,
                        &remote_event_id,
                        &action.event,
                        action
                            .link
                            .as_ref()
                            .and_then(|link| link.google_etag.as_deref()),
                    )
                    .await?;
                upsert_event_link(conn, link_after_google_write(&pair.id, action, meta))?;
            }
            PlannedAction::UpdateIcloud(action) => {
                let remote_event_id = icloud_remote_event_id(action, "update_icloud")?;
                let meta = providers
                    .icloud
                    .update_event(
                        &pair.icloud_calendar_id,
                        &remote_event_id,
                        &action.event,
                        action
                            .link
                            .as_ref()
                            .and_then(|link| link.icloud_etag.as_deref()),
                    )
                    .await?;
                upsert_event_link(conn, link_after_icloud_write(&pair.id, action, meta))?;
            }
            PlannedAction::DeleteGoogle(action) => {
                let remote_event_id = google_remote_event_id(action, "delete_google")?;
                providers
                    .google
                    .delete_event(
                        &pair.google_calendar_id,
                        &remote_event_id,
                        action
                            .link
                            .as_ref()
                            .and_then(|link| link.google_etag.as_deref()),
                    )
                    .await?;
                upsert_event_link(
                    conn,
                    event_link_upsert(
                        &pair.id,
                        &action.canonical_uid,
                        action.link.as_ref(),
                        action.google.as_deref().map(|event| &event.provider_meta),
                        action.icloud.as_deref().map(|event| &event.provider_meta),
                        action.google_hash.clone(),
                        action.icloud_hash.clone(),
                        None,
                        Some(now_string()),
                        None,
                    ),
                )?;
            }
            PlannedAction::DeleteIcloud(action) => {
                let remote_event_id = icloud_remote_event_id(action, "delete_icloud")?;
                providers
                    .icloud
                    .delete_event(
                        &pair.icloud_calendar_id,
                        &remote_event_id,
                        action
                            .link
                            .as_ref()
                            .and_then(|link| link.icloud_etag.as_deref()),
                    )
                    .await?;
                upsert_event_link(
                    conn,
                    event_link_upsert(
                        &pair.id,
                        &action.canonical_uid,
                        action.link.as_ref(),
                        action.google.as_deref().map(|event| &event.provider_meta),
                        action.icloud.as_deref().map(|event| &event.provider_meta),
                        action.google_hash.clone(),
                        action.icloud_hash.clone(),
                        None,
                        None,
                        Some(now_string()),
                    ),
                )?;
            }
            PlannedAction::Conflict {
                canonical_uid,
                reason,
                resolution,
                link,
                google,
                icloud,
            } => {
                if *resolution == insync_core::ConflictResolution::Manual {
                    record_conflict(
                        conn,
                        RecordConflictInput {
                            sync_pair_id: pair.id.clone(),
                            event_link_id: link.as_ref().map(|link| link.id.clone()),
                            canonical_uid: canonical_uid.clone(),
                            reason: reason.clone(),
                            google_snapshot: google
                                .as_deref()
                                .and_then(|event| serde_json::to_value(event).ok()),
                            icloud_snapshot: icloud
                                .as_deref()
                                .and_then(|event| serde_json::to_value(event).ok()),
                        },
                    )?;
                }
            }
        }
    }

    let _ = config;
    Ok(())
}

fn link_after_google_write(
    pair_id: &str,
    action: &MutatingAction,
    meta: ProviderEventMeta,
) -> EventLinkUpsert {
    let event_hash = hash_canonical_event(&action.event);
    event_link_upsert(
        pair_id,
        &action.canonical_uid,
        action.link.as_ref(),
        Some(&meta),
        action.icloud.as_deref().map(|event| &event.provider_meta),
        Some(event_hash.clone()),
        action
            .icloud_hash
            .clone()
            .or_else(|| Some(event_hash.clone())),
        Some(event_hash),
        None,
        None,
    )
}

fn link_after_icloud_write(
    pair_id: &str,
    action: &MutatingAction,
    meta: ProviderEventMeta,
) -> EventLinkUpsert {
    let event_hash = hash_canonical_event(&action.event);
    event_link_upsert(
        pair_id,
        &action.canonical_uid,
        action.link.as_ref(),
        action.google.as_deref().map(|event| &event.provider_meta),
        Some(&meta),
        action
            .google_hash
            .clone()
            .or_else(|| Some(event_hash.clone())),
        Some(event_hash.clone()),
        Some(event_hash),
        None,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
fn event_link_upsert(
    pair_id: &str,
    canonical_uid: &str,
    existing: Option<&insync_core::EventLink>,
    google_meta: Option<&ProviderEventMeta>,
    icloud_meta: Option<&ProviderEventMeta>,
    google_hash: Option<String>,
    icloud_hash: Option<String>,
    last_synced_hash: Option<String>,
    deleted_google_at: Option<String>,
    deleted_icloud_at: Option<String>,
) -> EventLinkUpsert {
    EventLinkUpsert {
        sync_pair_id: pair_id.to_string(),
        canonical_uid: canonical_uid.to_string(),
        google_event_id: google_meta
            .filter(|meta| meta.provider == ProviderName::Google)
            .and_then(|meta| meta.event_id.clone())
            .or_else(|| existing.and_then(|link| link.google_event_id.clone())),
        google_ical_uid: google_meta
            .filter(|meta| meta.provider == ProviderName::Google)
            .and_then(|meta| meta.ical_uid.clone())
            .or_else(|| existing.and_then(|link| link.google_ical_uid.clone())),
        google_etag: google_meta
            .filter(|meta| meta.provider == ProviderName::Google)
            .and_then(|meta| meta.etag.clone())
            .or_else(|| existing.and_then(|link| link.google_etag.clone())),
        icloud_href: icloud_meta
            .filter(|meta| meta.provider == ProviderName::Icloud)
            .and_then(|meta| meta.href.clone())
            .or_else(|| existing.and_then(|link| link.icloud_href.clone())),
        icloud_uid: icloud_meta
            .filter(|meta| meta.provider == ProviderName::Icloud)
            .and_then(|meta| meta.ical_uid.clone())
            .or_else(|| existing.and_then(|link| link.icloud_uid.clone())),
        icloud_etag: icloud_meta
            .filter(|meta| meta.provider == ProviderName::Icloud)
            .and_then(|meta| meta.etag.clone())
            .or_else(|| existing.and_then(|link| link.icloud_etag.clone())),
        google_hash: google_hash.or_else(|| existing.and_then(|link| link.google_hash.clone())),
        icloud_hash: icloud_hash.or_else(|| existing.and_then(|link| link.icloud_hash.clone())),
        last_synced_hash: last_synced_hash
            .or_else(|| existing.and_then(|link| link.last_synced_hash.clone())),
        deleted_google_at,
        deleted_icloud_at,
    }
}

fn google_remote_event_id(
    action: &MutatingAction,
    action_name: &'static str,
) -> Result<String, EngineError> {
    action
        .link
        .as_ref()
        .and_then(|link| link.google_event_id.clone())
        .or_else(|| {
            action
                .google
                .as_deref()
                .and_then(|event| event.provider_meta.event_id.clone())
        })
        .ok_or_else(|| EngineError::MissingApplyMetadata {
            action: action_name,
            canonical_uid: action.canonical_uid.clone(),
            field: "google_event_id",
        })
}

fn icloud_remote_event_id(
    action: &MutatingAction,
    action_name: &'static str,
) -> Result<String, EngineError> {
    action
        .link
        .as_ref()
        .and_then(|link| link.icloud_href.clone())
        .or_else(|| {
            action
                .icloud
                .as_deref()
                .and_then(|event| event.provider_meta.href.clone())
        })
        .ok_or_else(|| EngineError::MissingApplyMetadata {
            action: action_name,
            canonical_uid: action.canonical_uid.clone(),
            field: "icloud_href",
        })
}

fn synced_hash(google_hash: Option<&str>, icloud_hash: Option<&str>) -> Option<String> {
    match (google_hash, icloud_hash) {
        (Some(google_hash), Some(icloud_hash)) if google_hash == icloud_hash => {
            Some(google_hash.to_string())
        }
        (Some(hash), None) | (None, Some(hash)) => Some(hash.to_string()),
        _ => None,
    }
}

fn now_string() -> String {
    Utc::now().to_rfc3339()
}

#[derive(Debug)]
struct SyncLock {
    path: PathBuf,
}

impl Drop for SyncLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn acquire_sync_lock(db_path: &Path) -> Result<SyncLock, EngineError> {
    let lock_path = db_path.with_extension(format!(
        "{}lock",
        db_path
            .extension()
            .and_then(|value| value.to_str())
            .map(|value| format!("{value}."))
            .unwrap_or_default()
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&lock_path)
        .map_err(|source| {
            if source.kind() == std::io::ErrorKind::AlreadyExists {
                EngineError::LockAlreadyHeld(lock_path.clone())
            } else {
                EngineError::LockCreate {
                    path: lock_path.clone(),
                    source,
                }
            }
        })?;
    writeln!(file, "pid={}", std::process::id()).map_err(|source| EngineError::LockCreate {
        path: lock_path.clone(),
        source,
    })?;

    Ok(SyncLock { path: lock_path })
}

fn active_manual_conflicts(actions: &[PlannedAction]) -> Vec<ActiveConflict> {
    actions
        .iter()
        .filter_map(|action| {
            if let PlannedAction::Conflict {
                canonical_uid,
                reason,
                resolution: insync_core::ConflictResolution::Manual,
                ..
            } = action
            {
                Some(ActiveConflict {
                    canonical_uid: canonical_uid.clone(),
                    reason: reason.clone(),
                })
            } else {
                None
            }
        })
        .collect()
}

fn enabled_pair_count(config: &ServiceConfig) -> usize {
    config.sync.pairs.iter().filter(|pair| pair.enabled).count()
}

fn sync_run_summary(run: SyncRun) -> SyncRunSummary {
    SyncRunSummary {
        id: run.id,
        sync_pair_id: run.sync_pair_id,
        status: match run.status {
            SyncRunStatus::Running => "running",
            SyncRunStatus::Completed => "completed",
            SyncRunStatus::Failed => "failed",
        }
        .to_string(),
        started_at: run.started_at,
        finished_at: run.finished_at,
        error: run.error,
    }
}

fn planner_conflict_policies(config: &ServiceConfig) -> PlannerConflictPolicies {
    let default_policy = config.sync.conflicts.default;
    PlannerConflictPolicies {
        both_sides_changed: non_default_policy(
            config.sync.conflicts.both_sides_changed,
            default_policy,
        ),
        unlinked_same_uid: non_default_policy(
            config.sync.conflicts.unlinked_same_uid,
            default_policy,
        ),
        delete_vs_update: config.sync.conflicts.delete_vs_update,
        icloud_uid_collision: config.sync.conflicts.icloud_uid_collision,
    }
}

fn non_default_policy(value: ConflictPolicy, default_policy: ConflictPolicy) -> ConflictPolicy {
    if value == ConflictPolicy::Manual {
        default_policy
    } else {
        value
    }
}

fn count_actions(actions: &[PlannedAction]) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for action in actions {
        *counts.entry(action_name(action).to_string()).or_insert(0) += 1;
    }
    counts
}

fn count_resolutions(actions: &[PlannedAction]) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for action in actions {
        if let Some(resolution) = action_resolution_name(action) {
            *counts.entry(resolution.to_string()).or_insert(0) += 1;
        }
    }
    counts
}

fn merge_counts(target: &mut BTreeMap<String, usize>, source: &BTreeMap<String, usize>) {
    for (key, value) in source {
        *target.entry(key.clone()).or_insert(0) += value;
    }
}

fn action_name(action: &PlannedAction) -> &'static str {
    match action {
        PlannedAction::Snapshot { .. } => "snapshot",
        PlannedAction::Noop { .. } => "noop",
        PlannedAction::CreateGoogle(_) => "create_google",
        PlannedAction::CreateIcloud(_) => "create_icloud",
        PlannedAction::UpdateGoogle(_) => "update_google",
        PlannedAction::UpdateIcloud(_) => "update_icloud",
        PlannedAction::DeleteGoogle(_) => "delete_google",
        PlannedAction::DeleteIcloud(_) => "delete_icloud",
        PlannedAction::Conflict { .. } => "conflict",
    }
}

fn action_resolution_name(action: &PlannedAction) -> Option<&str> {
    match action {
        PlannedAction::CreateGoogle(action)
        | PlannedAction::CreateIcloud(action)
        | PlannedAction::UpdateGoogle(action)
        | PlannedAction::UpdateIcloud(action)
        | PlannedAction::DeleteGoogle(action)
        | PlannedAction::DeleteIcloud(action) => {
            action.resolution.as_ref().map(|_| "auto_resolved")
        }
        PlannedAction::Conflict { resolution, .. } => Some(match resolution {
            insync_core::ConflictResolution::Manual => "conflict_manual",
            insync_core::ConflictResolution::Ignored => "conflict_ignored",
        }),
        _ => None,
    }
}

fn should_include_report_action(action: &PlannedAction, mode: ReportMode) -> bool {
    match mode {
        ReportMode::ActionsOnly => !matches!(action, PlannedAction::Snapshot { .. }),
        ReportMode::AllActions => true,
    }
}

pub fn write_dry_run_report(path: impl AsRef<Path>, rows: &[ReportRow]) -> Result<(), EngineError> {
    let path = path.as_ref();
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(|source| EngineError::ReportWrite {
            path: parent.to_path_buf(),
            source,
        })?;
    }

    let mut lines = vec![REPORT_HEADERS.join(",")];
    lines.extend(rows.iter().map(report_row_to_csv));
    fs::write(path, format!("{}\n", lines.join("\n"))).map_err(|source| {
        EngineError::ReportWrite {
            path: path.to_path_buf(),
            source,
        }
    })?;
    Ok(())
}

const REPORT_HEADERS: &[&str] = &[
    "pair_id",
    "action",
    "canonical_uid",
    "reason",
    "resolution",
    "conflict_policy",
    "title",
    "google_present",
    "icloud_present",
    "google_title",
    "icloud_title",
    "google_start",
    "icloud_start",
    "google_end",
    "icloud_end",
    "google_status",
    "icloud_status",
    "google_hash",
    "icloud_hash",
    "diff_fields",
];

fn action_to_report_row(pair_id: &str, action: &PlannedAction) -> ReportRow {
    let google = action_google(action);
    let icloud = action_icloud(action);
    let event = action_event(action).or(google).or(icloud);
    let google_hash = google.map(hash_canonical_event).unwrap_or_default();
    let icloud_hash = icloud.map(hash_canonical_event).unwrap_or_default();
    let (reason, resolution, conflict_policy) = report_resolution(action);

    ReportRow {
        pair_id: pair_id.to_string(),
        action: action_name(action).to_string(),
        canonical_uid: action_uid(action).to_string(),
        reason,
        resolution,
        conflict_policy,
        title: event.map(|event| event.title.clone()).unwrap_or_default(),
        google_present: present_value(google),
        icloud_present: present_value(icloud),
        google_title: google.map(|event| event.title.clone()).unwrap_or_default(),
        icloud_title: icloud.map(|event| event.title.clone()).unwrap_or_default(),
        google_start: google
            .map(|event| format_event_date(&event.start))
            .unwrap_or_default(),
        icloud_start: icloud
            .map(|event| format_event_date(&event.start))
            .unwrap_or_default(),
        google_end: google
            .map(|event| format_event_date(&event.end))
            .unwrap_or_default(),
        icloud_end: icloud
            .map(|event| format_event_date(&event.end))
            .unwrap_or_default(),
        google_status: google
            .map(|event| status_name(event.status).to_string())
            .unwrap_or_default(),
        icloud_status: icloud
            .map(|event| status_name(event.status).to_string())
            .unwrap_or_default(),
        google_hash,
        icloud_hash,
        diff_fields: google
            .zip(icloud)
            .map(|(google, icloud)| diff_event_fields(google, icloud).join("|"))
            .unwrap_or_default(),
    }
}

fn action_uid(action: &PlannedAction) -> &str {
    match action {
        PlannedAction::Snapshot { canonical_uid, .. }
        | PlannedAction::Noop { canonical_uid, .. }
        | PlannedAction::Conflict { canonical_uid, .. } => canonical_uid,
        PlannedAction::CreateGoogle(action)
        | PlannedAction::CreateIcloud(action)
        | PlannedAction::UpdateGoogle(action)
        | PlannedAction::UpdateIcloud(action)
        | PlannedAction::DeleteGoogle(action)
        | PlannedAction::DeleteIcloud(action) => &action.canonical_uid,
    }
}

fn action_event(action: &PlannedAction) -> Option<&CanonicalEvent> {
    match action {
        PlannedAction::CreateGoogle(action)
        | PlannedAction::CreateIcloud(action)
        | PlannedAction::UpdateGoogle(action)
        | PlannedAction::UpdateIcloud(action)
        | PlannedAction::DeleteGoogle(action)
        | PlannedAction::DeleteIcloud(action) => Some(&action.event),
        _ => None,
    }
}

fn action_google(action: &PlannedAction) -> Option<&CanonicalEvent> {
    match action {
        PlannedAction::Snapshot { google, .. } | PlannedAction::Conflict { google, .. } => {
            google.as_deref()
        }
        PlannedAction::CreateGoogle(action)
        | PlannedAction::CreateIcloud(action)
        | PlannedAction::UpdateGoogle(action)
        | PlannedAction::UpdateIcloud(action)
        | PlannedAction::DeleteGoogle(action)
        | PlannedAction::DeleteIcloud(action) => action.google.as_deref(),
        _ => None,
    }
}

fn action_icloud(action: &PlannedAction) -> Option<&CanonicalEvent> {
    match action {
        PlannedAction::Snapshot { icloud, .. } | PlannedAction::Conflict { icloud, .. } => {
            icloud.as_deref()
        }
        PlannedAction::CreateGoogle(action)
        | PlannedAction::CreateIcloud(action)
        | PlannedAction::UpdateGoogle(action)
        | PlannedAction::UpdateIcloud(action)
        | PlannedAction::DeleteGoogle(action)
        | PlannedAction::DeleteIcloud(action) => action.icloud.as_deref(),
        _ => None,
    }
}

fn report_resolution(action: &PlannedAction) -> (String, String, String) {
    match action {
        PlannedAction::Conflict {
            reason, resolution, ..
        } => (
            reason.clone(),
            match resolution {
                insync_core::ConflictResolution::Manual => "manual".to_string(),
                insync_core::ConflictResolution::Ignored => "ignored".to_string(),
            },
            String::new(),
        ),
        PlannedAction::CreateGoogle(action)
        | PlannedAction::CreateIcloud(action)
        | PlannedAction::UpdateGoogle(action)
        | PlannedAction::UpdateIcloud(action)
        | PlannedAction::DeleteGoogle(action)
        | PlannedAction::DeleteIcloud(action) => action
            .resolution
            .as_ref()
            .map(|resolution| {
                (
                    resolution.reason.clone(),
                    "auto_resolved".to_string(),
                    resolution.policy.clone(),
                )
            })
            .unwrap_or_default(),
        _ => (String::new(), String::new(), String::new()),
    }
}

fn present_value(event: Option<&CanonicalEvent>) -> String {
    if event.is_some() {
        "yes".to_string()
    } else {
        "no".to_string()
    }
}

fn format_event_date(value: &EventDateTime) -> String {
    match value {
        EventDateTime::Date { value } => value.format("%Y-%m-%d").to_string(),
        EventDateTime::DateTime { value, timezone } => {
            let instant = value.to_rfc3339();
            timezone
                .as_ref()
                .map(|timezone| format!("{instant} [{timezone}]"))
                .unwrap_or(instant)
        }
    }
}

fn status_name(status: EventStatus) -> &'static str {
    match status {
        EventStatus::Confirmed => "confirmed",
        EventStatus::Tentative => "tentative",
        EventStatus::Cancelled => "cancelled",
    }
}

fn diff_event_fields(google: &CanonicalEvent, icloud: &CanonicalEvent) -> Vec<&'static str> {
    let mut fields = Vec::new();

    if normalize_text(Some(&google.title)) != normalize_text(Some(&icloud.title)) {
        fields.push("title");
    }
    if normalize_text(google.description.as_ref()) != normalize_text(icloud.description.as_ref()) {
        fields.push("description");
    }
    if normalize_location(google.location.as_ref()) != normalize_location(icloud.location.as_ref())
    {
        fields.push("location");
    }
    if google.status != icloud.status {
        fields.push("status");
    }
    if report_date_key(&google.start) != report_date_key(&icloud.start) {
        fields.push("start");
    }
    if report_date_key(&google.end) != report_date_key(&icloud.end) {
        fields.push("end");
    }
    if google.recurrence != icloud.recurrence {
        fields.push("recurrence");
    }

    fields
}

fn normalize_text(value: Option<&String>) -> String {
    value
        .map(|item| item.replace("\r\n", "\n").trim().to_string())
        .unwrap_or_default()
}

fn normalize_location(value: Option<&String>) -> String {
    normalize_text(value).replace("\\n", "\n")
}

fn report_date_key(value: &EventDateTime) -> String {
    match value {
        EventDateTime::Date { value } => format!("date:{}", value.format("%Y-%m-%d")),
        EventDateTime::DateTime { value, .. } => format!("dateTime:{}", value.to_rfc3339()),
    }
}

fn report_row_to_csv(row: &ReportRow) -> String {
    [
        &row.pair_id,
        &row.action,
        &row.canonical_uid,
        &row.reason,
        &row.resolution,
        &row.conflict_policy,
        &row.title,
        &row.google_present,
        &row.icloud_present,
        &row.google_title,
        &row.icloud_title,
        &row.google_start,
        &row.icloud_start,
        &row.google_end,
        &row.icloud_end,
        &row.google_status,
        &row.icloud_status,
        &row.google_hash,
        &row.icloud_hash,
        &row.diff_fields,
    ]
    .into_iter()
    .map(|value| csv_escape(value))
    .collect::<Vec<_>>()
    .join(",")
}

fn csv_escape(value: &str) -> String {
    if value.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

fn resolve_db_path(db_path: &Path, config_path: Option<&Path>) -> PathBuf {
    if db_path.is_absolute() {
        return db_path.to_path_buf();
    }

    config_path
        .and_then(Path::parent)
        .filter(|parent| !parent.as_os_str().is_empty())
        .map(|parent| parent.join(db_path))
        .unwrap_or_else(|| db_path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use insync_config::{GoogleConfig, IcloudConfig, SyncConfig, SyncPairConfig};
    use insync_core::{
        CanonicalEvent, EventDateTime, EventStatus, EventVisibility, ProviderEventMeta,
        ProviderName, SyncDirection,
    };
    use insync_db::repositories::event_links::{
        EventLinkUpsert, get_event_link, upsert_event_link,
    };
    use insync_providers::{ProviderCalendar, ProviderChangeSet, ProviderSyncCursor};
    use std::sync::{Arc, Mutex};

    #[tokio::test]
    async fn doctor_and_run_once_prepare_database() {
        let temp_dir = std::env::temp_dir().join(format!("insync-engine-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();
        let config_path = temp_dir.join("insync.json");
        let config = config_with_db(".state/insync.db");
        let engine = SyncEngine::with_config_path(config, &config_path);

        let doctor = engine.doctor().unwrap();
        assert_eq!(doctor.configured_pair_count, 1);
        assert_eq!(doctor.enabled_pair_count, 1);
        assert_eq!(doctor.db_path, temp_dir.join(".state/insync.db"));

        let summary = engine.run_once(RunMode::DryRun).await.unwrap();
        assert_eq!(summary.configured_pair_count, 1);
        assert_eq!(summary.enabled_pair_count, 1);
        assert_eq!(summary.mode, RunMode::DryRun);
        assert!(summary.db_path.exists());

        let doctor = engine.doctor().unwrap();
        let latest_run = doctor.latest_run.unwrap();
        assert_eq!(latest_run.status, "completed");
        assert!(latest_run.finished_at.is_some());

        std::fs::remove_dir_all(&temp_dir).unwrap();
    }

    #[test]
    fn sync_lock_blocks_second_holder_until_released() {
        let temp_dir =
            std::env::temp_dir().join(format!("insync-engine-lock-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();
        let db_path = temp_dir.join("insync.db");

        let lock = acquire_sync_lock(&db_path).unwrap();
        let second = acquire_sync_lock(&db_path).unwrap_err();
        assert!(matches!(second, EngineError::LockAlreadyHeld(_)));
        drop(lock);
        let _third = acquire_sync_lock(&db_path).unwrap();

        std::fs::remove_dir_all(&temp_dir).unwrap();
    }

    #[tokio::test]
    async fn provider_planning_creates_missing_icloud_event() {
        let temp_dir =
            std::env::temp_dir().join(format!("insync-engine-plan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();
        let config_path = temp_dir.join("insync.json");
        let config = config_with_db(".state/insync.db");
        let engine = SyncEngine::with_config_path(config, &config_path);
        let google = FakeProvider {
            name: ProviderName::Google,
            events: vec![event("uid-1", "Planning", ProviderName::Google)],
        };
        let icloud = FakeProvider {
            name: ProviderName::Icloud,
            events: Vec::new(),
        };

        let summary = engine
            .plan_once_with_providers(
                RunMode::DryRun,
                SyncProviders {
                    google: &google,
                    icloud: &icloud,
                },
            )
            .await
            .unwrap();

        assert_eq!(summary.enabled_pair_count, 1);
        assert_eq!(summary.pair_summaries[0].google_events, 1);
        assert_eq!(summary.pair_summaries[0].icloud_events, 0);
        assert_eq!(summary.action_counts.get("create_icloud"), Some(&1));
        assert_eq!(summary.report_rows.len(), 1);
        assert_eq!(summary.report_rows[0].pair_id, "personal");
        assert_eq!(summary.report_rows[0].action, "create_icloud");
        assert_eq!(summary.report_rows[0].canonical_uid, "uid-1");
        assert_eq!(summary.report_rows[0].google_present, "yes");
        assert_eq!(summary.report_rows[0].icloud_present, "no");

        std::fs::remove_dir_all(&temp_dir).unwrap();
    }

    #[tokio::test]
    async fn writes_dry_run_report_csv() {
        let temp_dir =
            std::env::temp_dir().join(format!("insync-engine-report-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();
        let config_path = temp_dir.join("insync.json");
        let config = config_with_db(".state/insync.db");
        let engine = SyncEngine::with_config_path(config, &config_path);
        let google = FakeProvider {
            name: ProviderName::Google,
            events: vec![event("uid-1", "Planning, with comma", ProviderName::Google)],
        };
        let icloud = FakeProvider {
            name: ProviderName::Icloud,
            events: Vec::new(),
        };
        let summary = engine
            .plan_once_with_providers(
                RunMode::DryRun,
                SyncProviders {
                    google: &google,
                    icloud: &icloud,
                },
            )
            .await
            .unwrap();
        let report_path = temp_dir.join("reports/dry-run.csv");

        engine
            .write_dry_run_report(&report_path, &summary.report_rows)
            .unwrap();
        let body = std::fs::read_to_string(&report_path).unwrap();

        assert!(body.starts_with("pair_id,action,canonical_uid"));
        assert!(body.contains("create_icloud"));
        assert!(body.contains("\"Planning, with comma\""));

        std::fs::remove_dir_all(&temp_dir).unwrap();
    }

    #[tokio::test]
    async fn snapshots_are_reported_only_when_requested() {
        let temp_dir =
            std::env::temp_dir().join(format!("insync-engine-snapshots-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();
        let config_path = temp_dir.join("insync.json");
        let config = config_with_db(".state/insync.db");
        let engine = SyncEngine::with_config_path(config, &config_path);
        let google_event = event("uid-1", "Planning", ProviderName::Google);
        let icloud_event = event("uid-1", "Planning", ProviderName::Icloud);
        let event_hash = hash_canonical_event(&google_event);
        engine.doctor().unwrap();
        let conn = insync_db::open(temp_dir.join(".state/insync.db")).unwrap();
        upsert_event_link(
            &conn,
            EventLinkUpsert {
                sync_pair_id: "personal".to_string(),
                canonical_uid: "uid-1".to_string(),
                google_event_id: Some("uid-1".to_string()),
                google_ical_uid: Some("uid-1".to_string()),
                icloud_href: Some("uid-1.ics".to_string()),
                icloud_uid: Some("uid-1".to_string()),
                google_hash: Some(event_hash.clone()),
                icloud_hash: Some(event_hash.clone()),
                last_synced_hash: Some(event_hash),
                ..EventLinkUpsert::default()
            },
        )
        .unwrap();
        drop(conn);
        let google = FakeProvider {
            name: ProviderName::Google,
            events: vec![google_event],
        };
        let icloud = FakeProvider {
            name: ProviderName::Icloud,
            events: vec![icloud_event],
        };

        let default_summary = engine
            .plan_once_with_providers(
                RunMode::DryRun,
                SyncProviders {
                    google: &google,
                    icloud: &icloud,
                },
            )
            .await
            .unwrap();
        let full_summary = engine
            .plan_once_with_providers_and_report_mode(
                RunMode::DryRun,
                SyncProviders {
                    google: &google,
                    icloud: &icloud,
                },
                ReportMode::AllActions,
            )
            .await
            .unwrap();

        assert_eq!(default_summary.action_counts.get("snapshot"), Some(&1));
        assert!(default_summary.report_rows.is_empty());
        assert_eq!(full_summary.report_rows.len(), 1);
        assert_eq!(full_summary.report_rows[0].action, "snapshot");

        std::fs::remove_dir_all(&temp_dir).unwrap();
    }

    #[tokio::test]
    async fn provider_apply_creates_missing_icloud_event_and_records_link() {
        let temp_dir =
            std::env::temp_dir().join(format!("insync-engine-apply-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();
        let config_path = temp_dir.join("insync.json");
        let config = config_with_db(".state/insync.db");
        let engine = SyncEngine::with_config_path(config, &config_path);
        let google = FakeProvider {
            name: ProviderName::Google,
            events: vec![event("uid-1", "Apply", ProviderName::Google)],
        };
        let icloud = FakeProvider {
            name: ProviderName::Icloud,
            events: Vec::new(),
        };

        let summary = engine
            .plan_once_with_providers(
                RunMode::Apply,
                SyncProviders {
                    google: &google,
                    icloud: &icloud,
                },
            )
            .await
            .unwrap();
        let conn = insync_db::open(temp_dir.join(".state/insync.db")).unwrap();
        let link = get_event_link(&conn, "personal", "uid-1").unwrap().unwrap();
        let latest_run = engine.doctor().unwrap().latest_run.unwrap();

        assert_eq!(summary.action_counts.get("create_icloud"), Some(&1));
        assert_eq!(summary.mode, RunMode::Apply);
        assert_eq!(link.google_event_id.as_deref(), Some("uid-1"));
        assert_eq!(
            link.icloud_href.as_deref(),
            Some("https://caldav.example/cal/uid-1.ics")
        );
        assert!(link.last_synced_hash.is_some());
        assert_eq!(latest_run.status, "completed");

        std::fs::remove_dir_all(&temp_dir).unwrap();
    }

    #[tokio::test]
    async fn provider_apply_records_manual_conflicts() {
        let temp_dir = std::env::temp_dir().join(format!(
            "insync-engine-apply-conflict-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();
        let config_path = temp_dir.join("insync.json");
        let config = config_with_db(".state/insync.db");
        let engine = SyncEngine::with_config_path(config, &config_path);
        let google = FakeProvider {
            name: ProviderName::Google,
            events: vec![event("uid-1", "Google title", ProviderName::Google)],
        };
        let icloud = FakeProvider {
            name: ProviderName::Icloud,
            events: vec![event("uid-1", "iCloud title", ProviderName::Icloud)],
        };

        let summary = engine
            .plan_once_with_providers(
                RunMode::Apply,
                SyncProviders {
                    google: &google,
                    icloud: &icloud,
                },
            )
            .await
            .unwrap();
        let conflicts = engine.conflict_summaries().unwrap();

        assert_eq!(summary.action_counts.get("conflict"), Some(&1));
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].sync_pair_id, "personal");
        assert_eq!(
            conflicts[0].reason,
            "unlinked_events_have_same_uid_but_differ"
        );

        std::fs::remove_dir_all(&temp_dir).unwrap();
    }

    #[tokio::test]
    async fn provider_apply_resolves_stale_conflicts() {
        let temp_dir = std::env::temp_dir().join(format!(
            "insync-engine-apply-resolve-conflict-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();
        let config_path = temp_dir.join("insync.json");
        let config = config_with_db(".state/insync.db");
        let engine = SyncEngine::with_config_path(config, &config_path);
        engine.doctor().unwrap();
        let conn = insync_db::open(temp_dir.join(".state/insync.db")).unwrap();
        record_conflict(
            &conn,
            RecordConflictInput {
                sync_pair_id: "personal".to_string(),
                canonical_uid: "uid-1".to_string(),
                reason: "unlinked_events_have_same_uid_but_differ".to_string(),
                ..RecordConflictInput::default()
            },
        )
        .unwrap();
        drop(conn);
        let google = FakeProvider {
            name: ProviderName::Google,
            events: vec![event("uid-1", "Same title", ProviderName::Google)],
        };
        let icloud = FakeProvider {
            name: ProviderName::Icloud,
            events: vec![event("uid-1", "Same title", ProviderName::Icloud)],
        };

        let summary = engine
            .plan_once_with_providers(
                RunMode::Apply,
                SyncProviders {
                    google: &google,
                    icloud: &icloud,
                },
            )
            .await
            .unwrap();
        let conflicts = engine.conflict_summaries().unwrap();

        assert_eq!(summary.action_counts.get("snapshot"), Some(&1));
        assert!(conflicts.is_empty());

        std::fs::remove_dir_all(&temp_dir).unwrap();
    }

    #[tokio::test]
    async fn provider_planning_uses_stored_sync_tokens() {
        let temp_dir =
            std::env::temp_dir().join(format!("insync-engine-sync-token-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();
        let config_path = temp_dir.join("insync.json");
        let config = config_with_db(".state/insync.db");
        let engine = SyncEngine::with_config_path(config, &config_path);
        let google = CursorRecordingProvider::new(
            ProviderName::Google,
            vec![event("uid-1", "Token", ProviderName::Google)],
            Some("google-token-1".to_string()),
        );
        let icloud = CursorRecordingProvider::new(
            ProviderName::Icloud,
            Vec::new(),
            Some("icloud-token-1".to_string()),
        );

        engine
            .plan_once_with_providers(
                RunMode::Apply,
                SyncProviders {
                    google: &google,
                    icloud: &icloud,
                },
            )
            .await
            .unwrap();
        google.clear_cursors();
        icloud.clear_cursors();
        engine
            .plan_once_with_providers(
                RunMode::DryRun,
                SyncProviders {
                    google: &google,
                    icloud: &icloud,
                },
            )
            .await
            .unwrap();

        let google_cursors = google.cursors();
        let icloud_cursors = icloud.cursors();
        assert_eq!(google_cursors.len(), 1);
        assert_eq!(
            google_cursors[0].sync_token.as_deref(),
            Some("google-token-1")
        );
        assert!(!google_cursors[0].full_sync);
        assert_eq!(icloud_cursors.len(), 1);
        assert_eq!(
            icloud_cursors[0].sync_token.as_deref(),
            Some("icloud-token-1")
        );
        assert!(!icloud_cursors[0].full_sync);

        std::fs::remove_dir_all(&temp_dir).unwrap();
    }

    #[tokio::test]
    async fn provider_planning_recovers_from_expired_sync_token() {
        let temp_dir = std::env::temp_dir().join(format!(
            "insync-engine-expired-token-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();
        let config_path = temp_dir.join("insync.json");
        let config = config_with_db(".state/insync.db");
        let engine = SyncEngine::with_config_path(config, &config_path);
        let google = ExpiringCursorProvider::new(
            ProviderName::Google,
            vec![event("uid-1", "Token", ProviderName::Google)],
            Some("google-token-2".to_string()),
        );
        let icloud = CursorRecordingProvider::new(
            ProviderName::Icloud,
            Vec::new(),
            Some("icloud-token-1".to_string()),
        );
        let seed_google = CursorRecordingProvider::new(
            ProviderName::Google,
            vec![event("uid-1", "Token", ProviderName::Google)],
            Some("google-token-1".to_string()),
        );
        engine
            .plan_once_with_providers(
                RunMode::Apply,
                SyncProviders {
                    google: &seed_google,
                    icloud: &icloud,
                },
            )
            .await
            .unwrap();

        engine
            .plan_once_with_providers(
                RunMode::DryRun,
                SyncProviders {
                    google: &google,
                    icloud: &icloud,
                },
            )
            .await
            .unwrap();

        let cursors = google.cursors();
        assert_eq!(cursors.len(), 2);
        assert_eq!(cursors[0].sync_token.as_deref(), Some("google-token-1"));
        assert!(!cursors[0].full_sync);
        assert_eq!(cursors[1].sync_token, None);
        assert!(cursors[1].full_sync);

        std::fs::remove_dir_all(&temp_dir).unwrap();
    }

    fn config_with_db(db_path: &str) -> ServiceConfig {
        ServiceConfig {
            db_path: db_path.into(),
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

    #[derive(Debug, Clone)]
    struct FakeProvider {
        name: ProviderName,
        events: Vec<CanonicalEvent>,
    }

    #[async_trait::async_trait]
    impl CalendarProvider for FakeProvider {
        fn name(&self) -> ProviderName {
            self.name
        }

        async fn list_calendars(&self) -> Result<Vec<ProviderCalendar>, ProviderError> {
            Ok(Vec::new())
        }

        async fn get_changes(
            &self,
            calendar_id: &str,
            _cursor: ProviderSyncCursor,
        ) -> Result<ProviderChangeSet, ProviderError> {
            Ok(ProviderChangeSet {
                provider: self.name,
                calendar_id: calendar_id.to_string(),
                sync_token: None,
                events: self.events.clone(),
            })
        }

        async fn create_event(
            &self,
            calendar_id: &str,
            event: &CanonicalEvent,
        ) -> Result<ProviderEventMeta, ProviderError> {
            Ok(meta(self.name, calendar_id, event))
        }

        async fn update_event(
            &self,
            calendar_id: &str,
            _remote_event_id: &str,
            event: &CanonicalEvent,
            _etag: Option<&str>,
        ) -> Result<ProviderEventMeta, ProviderError> {
            Ok(meta(self.name, calendar_id, event))
        }

        async fn delete_event(
            &self,
            _calendar_id: &str,
            _remote_event_id: &str,
            _etag: Option<&str>,
        ) -> Result<(), ProviderError> {
            Ok(())
        }
    }

    #[derive(Debug, Clone)]
    struct CursorRecordingProvider {
        name: ProviderName,
        events: Vec<CanonicalEvent>,
        sync_token: Option<String>,
        cursors: Arc<Mutex<Vec<ProviderSyncCursor>>>,
    }

    impl CursorRecordingProvider {
        fn new(
            name: ProviderName,
            events: Vec<CanonicalEvent>,
            sync_token: Option<String>,
        ) -> Self {
            Self {
                name,
                events,
                sync_token,
                cursors: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn cursors(&self) -> Vec<ProviderSyncCursor> {
            self.cursors.lock().unwrap().clone()
        }

        fn clear_cursors(&self) {
            self.cursors.lock().unwrap().clear();
        }
    }

    #[async_trait::async_trait]
    impl CalendarProvider for CursorRecordingProvider {
        fn name(&self) -> ProviderName {
            self.name
        }

        async fn list_calendars(&self) -> Result<Vec<ProviderCalendar>, ProviderError> {
            Ok(Vec::new())
        }

        async fn get_changes(
            &self,
            calendar_id: &str,
            cursor: ProviderSyncCursor,
        ) -> Result<ProviderChangeSet, ProviderError> {
            self.cursors.lock().unwrap().push(cursor);
            Ok(ProviderChangeSet {
                provider: self.name,
                calendar_id: calendar_id.to_string(),
                sync_token: self.sync_token.clone(),
                events: self.events.clone(),
            })
        }

        async fn create_event(
            &self,
            calendar_id: &str,
            event: &CanonicalEvent,
        ) -> Result<ProviderEventMeta, ProviderError> {
            Ok(meta(self.name, calendar_id, event))
        }

        async fn update_event(
            &self,
            calendar_id: &str,
            _remote_event_id: &str,
            event: &CanonicalEvent,
            _etag: Option<&str>,
        ) -> Result<ProviderEventMeta, ProviderError> {
            Ok(meta(self.name, calendar_id, event))
        }

        async fn delete_event(
            &self,
            _calendar_id: &str,
            _remote_event_id: &str,
            _etag: Option<&str>,
        ) -> Result<(), ProviderError> {
            Ok(())
        }
    }

    #[derive(Debug, Clone)]
    struct ExpiringCursorProvider {
        inner: CursorRecordingProvider,
        expired_once: Arc<Mutex<bool>>,
    }

    impl ExpiringCursorProvider {
        fn new(
            name: ProviderName,
            events: Vec<CanonicalEvent>,
            sync_token: Option<String>,
        ) -> Self {
            Self {
                inner: CursorRecordingProvider::new(name, events, sync_token),
                expired_once: Arc::new(Mutex::new(false)),
            }
        }

        fn cursors(&self) -> Vec<ProviderSyncCursor> {
            self.inner.cursors()
        }
    }

    #[async_trait::async_trait]
    impl CalendarProvider for ExpiringCursorProvider {
        fn name(&self) -> ProviderName {
            self.inner.name
        }

        async fn list_calendars(&self) -> Result<Vec<ProviderCalendar>, ProviderError> {
            Ok(Vec::new())
        }

        async fn get_changes(
            &self,
            calendar_id: &str,
            cursor: ProviderSyncCursor,
        ) -> Result<ProviderChangeSet, ProviderError> {
            self.inner.cursors.lock().unwrap().push(cursor.clone());
            let mut expired_once = self.expired_once.lock().unwrap();
            if !*expired_once && cursor.sync_token.is_some() {
                *expired_once = true;
                return Err(ProviderError::SyncTokenExpired(self.inner.name));
            }

            Ok(ProviderChangeSet {
                provider: self.inner.name,
                calendar_id: calendar_id.to_string(),
                sync_token: self.inner.sync_token.clone(),
                events: self.inner.events.clone(),
            })
        }

        async fn create_event(
            &self,
            calendar_id: &str,
            event: &CanonicalEvent,
        ) -> Result<ProviderEventMeta, ProviderError> {
            Ok(meta(self.inner.name, calendar_id, event))
        }

        async fn update_event(
            &self,
            calendar_id: &str,
            _remote_event_id: &str,
            event: &CanonicalEvent,
            _etag: Option<&str>,
        ) -> Result<ProviderEventMeta, ProviderError> {
            Ok(meta(self.inner.name, calendar_id, event))
        }

        async fn delete_event(
            &self,
            _calendar_id: &str,
            _remote_event_id: &str,
            _etag: Option<&str>,
        ) -> Result<(), ProviderError> {
            Ok(())
        }
    }

    fn event(uid: &str, title: &str, provider: ProviderName) -> CanonicalEvent {
        CanonicalEvent {
            canonical_uid: uid.to_string(),
            title: title.to_string(),
            description: None,
            location: None,
            status: EventStatus::Confirmed,
            visibility: EventVisibility::Default,
            start: EventDateTime::DateTime {
                value: Utc.with_ymd_and_hms(2026, 6, 1, 12, 0, 0).unwrap(),
                timezone: Some("UTC".to_string()),
            },
            end: EventDateTime::DateTime {
                value: Utc.with_ymd_and_hms(2026, 6, 1, 13, 0, 0).unwrap(),
                timezone: Some("UTC".to_string()),
            },
            recurrence: None,
            attendees: Vec::new(),
            reminders: Vec::new(),
            provider_meta: ProviderEventMeta {
                provider,
                calendar_id: "calendar".to_string(),
                event_id: Some(uid.to_string()),
                href: None,
                etag: None,
                ical_uid: Some(uid.to_string()),
                updated_at: None,
                deleted: false,
            },
            raw: serde_json::json!({}),
        }
    }

    fn meta(
        provider: ProviderName,
        calendar_id: &str,
        event: &CanonicalEvent,
    ) -> ProviderEventMeta {
        ProviderEventMeta {
            provider,
            calendar_id: calendar_id.to_string(),
            event_id: (provider == ProviderName::Google).then(|| event.canonical_uid.clone()),
            href: (provider == ProviderName::Icloud)
                .then(|| format!("{calendar_id}/{}.ics", event.canonical_uid)),
            etag: None,
            ical_uid: Some(event.canonical_uid.clone()),
            updated_at: None,
            deleted: false,
        }
    }
}
