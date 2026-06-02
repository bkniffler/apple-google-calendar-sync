use chrono::{DateTime, Duration as ChronoDuration, Utc};
use clap::{Parser, Subcommand, ValueEnum};
use color_eyre::eyre::{Context, Result, bail};
use crossterm::{
    event::{self, Event, KeyCode},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use insync_app::{
    AppCommand, AppConflictDetail, AppConflictSummary, AppEffect, AppEvent, AppModel,
    AppNotificationSeverity, AppPairRuntimeSnapshot, AppReportRow, AppRun, AppRuntimeSnapshot,
    AppSetupStep, AppSetupStepStatus, AppShellAction, AppStatus, AppView,
};
#[cfg(test)]
use insync_app::{AppReportFilter, AppReportSort, AppRunFilter, AppSetupState};
use insync_config::{
    LOCAL_CONFIG_FILE, SecretStoreKind, SyncPairConfig, app_config_path,
    credentials::{
        resolve_credentials, store_google_client_secret, store_google_refresh_token,
        store_icloud_app_password,
    },
    load_config, resolve_config_path, save_config, validate_config,
};
use insync_core::{CanonicalEvent, ProviderEventMeta, ProviderName, SyncDirection};
use insync_db::{
    backup_database, export_database_json, import_database_json, migrate, open,
    repositories::{
        calendars::{CalendarRow, list_calendars},
        configured_pairs::{configured_calendar_ids, seed_configured_pairs},
        sync_conflicts::{
            ConflictFilter as DbConflictFilter, list_unresolved_conflict_summaries,
            list_unresolved_conflicts,
        },
        sync_runs::{SyncRunStatus, recent_sync_runs},
    },
};
use insync_engine::{
    ConflictFilter, DoctorSummary, ManualResolution, ReportMode, RunMode, SyncEngine,
    SyncProviders, UnresolvedConflictRow, UnresolvedConflictSummary,
};
use insync_providers::{
    CalendarProvider, ProviderCalendar, ProviderChangeSet, ProviderError, ProviderSyncCursor,
    google::{
        GoogleAuthCodeExchange, GoogleCalendarProvider, GoogleEvent, GoogleProviderOptions,
        exchange_google_auth_code, google_auth_url, google_to_canonical,
    },
    icloud::{
        CalendarObject, IcloudCalDavProvider, IcloudProviderOptions, ical_object_to_canonical,
    },
};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, BorderType, Borders, Clear, Gauge, List, ListItem, Paragraph, Row, Table, Wrap,
    },
};
use serde::Deserialize;
use serde_json::Value;
use std::{
    collections::HashMap,
    env, fs,
    io::{self, Read, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::Command as ProcessCommand,
    time::Duration,
};
use tracing_subscriber::EnvFilter;
use url::Url;

#[derive(Debug, Parser)]
#[command(name = "insync", version, about = "iCloud <-> Google Calendar sync")]
struct Cli {
    #[arg(
        long,
        value_name = "PATH",
        help = "Config path; also supports INSYNC_CONFIG"
    )]
    config: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Setup {
        #[arg(long, value_enum, default_value_t = SetupLocation::Local)]
        location: SetupLocation,
        #[arg(long, value_enum, default_value_t = SetupSecretStore::Os)]
        secret_store: SetupSecretStore,
        #[arg(long)]
        force: bool,
        #[arg(long)]
        discover: bool,
        #[arg(long)]
        csv: Option<PathBuf>,
        #[arg(long)]
        pair_id: Option<String>,
        #[arg(long)]
        google_calendar_id: Option<String>,
        #[arg(long)]
        icloud_calendar_id: Option<String>,
        #[arg(long, value_enum, default_value_t = SetupDirection::TwoWay)]
        direction: SetupDirection,
        #[arg(long)]
        disabled: bool,
        #[arg(long)]
        google_auth_url: bool,
        #[arg(long)]
        google_callback: bool,
        #[arg(long)]
        google_code: Option<String>,
        #[arg(long, default_value = "http://127.0.0.1:8787/oauth2/callback")]
        redirect_uri: String,
        #[arg(long, default_value = "insync")]
        state: String,
        #[arg(long)]
        google_account_label: Option<String>,
        #[arg(long)]
        google_client_id: Option<String>,
        #[arg(long)]
        google_client_secret: Option<String>,
        #[arg(long)]
        icloud_account_label: Option<String>,
        #[arg(long)]
        icloud_username: Option<String>,
        #[arg(long)]
        icloud_app_password: Option<String>,
        #[arg(long)]
        icloud_caldav_url: Option<String>,
        #[arg(long)]
        interactive: bool,
    },
    #[command(about = "Validate config, credentials, database, and latest sync state")]
    Doctor,
    #[command(about = "Inspect, backup, export, or import the SQLite sync database")]
    Db {
        #[command(subcommand)]
        command: DbCommand,
    },
    #[command(about = "Inspect, export, dedupe, or retire recorded manual conflicts")]
    Conflicts {
        #[arg(
            long,
            value_name = "ID",
            help = "Queue a manual resolution for a conflict row"
        )]
        resolve: Option<String>,
        #[arg(
            long,
            value_enum,
            requires = "resolve",
            help = "Resolution to queue for --resolve"
        )]
        resolution: Option<ConflictResolutionArg>,
        #[arg(long, help = "Show individual conflict rows instead of pair summaries")]
        details: bool,
        #[arg(long, help = "Filter conflicts by planner reason")]
        reason: Option<String>,
        #[arg(long, help = "Filter conflicts by sync pair ID")]
        pair: Option<String>,
        #[arg(long, default_value_t = 100, help = "Maximum conflict rows to print")]
        limit: u32,
        #[arg(long, help = "Write conflict output as CSV")]
        csv: Option<PathBuf>,
        #[arg(long, help = "Resolve duplicate unresolved conflict rows in SQLite")]
        dedupe: bool,
        #[arg(
            long,
            help = "Fetch providers and resolve conflict rows that are no longer active"
        )]
        resolve_stale: bool,
        #[arg(long, help = "Use fixture provider data instead of live calendars")]
        fixtures: Option<PathBuf>,
    },
    #[command(about = "Plan a live sync by default, or execute writes with --apply")]
    Sync {
        #[arg(long, help = "Execute planned provider writes; omit for a dry-run")]
        apply: bool,
        #[arg(long, help = "Use fixture provider data instead of live calendars")]
        fixtures: Option<PathBuf>,
        #[arg(long, help = "Write planned actions as CSV")]
        report: Option<PathBuf>,
        #[arg(long, help = "Include no-op snapshots in the CSV report")]
        report_all: bool,
        #[arg(long, help = "Write the structured sync summary as JSON")]
        summary_json: Option<PathBuf>,
    },
    #[command(about = "Run sync repeatedly until Ctrl-C")]
    Daemon {
        #[arg(long, help = "Execute provider writes on each daemon tick")]
        apply: bool,
    },
    #[command(about = "Install, remove, inspect, or print background runner services")]
    Background {
        #[command(subcommand)]
        command: BackgroundCommand,
    },
    #[command(about = "Open the terminal dashboard")]
    Tui,
}

#[derive(Debug, Subcommand)]
enum BackgroundCommand {
    #[command(about = "Install and start a macOS launchd or Linux systemd user service")]
    Install {
        #[arg(long, help = "Execute provider writes on each daemon tick")]
        apply: bool,
        #[arg(long, help = "Replace an existing service definition")]
        force: bool,
    },
    #[command(about = "Stop and remove the installed background service")]
    Uninstall,
    #[command(about = "Print background service health and scheduler status")]
    Status,
    #[command(about = "Print the service definition without installing it")]
    Print {
        #[arg(long, value_enum, default_value_t = BackgroundTemplate::Auto)]
        template: BackgroundTemplate,
        #[arg(long, help = "Render daemon arguments with --apply")]
        apply: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum ConflictResolutionArg {
    GoogleWins,
    IcloudWins,
    DeleteWins,
    UpdateWins,
}

impl From<ConflictResolutionArg> for ManualResolution {
    fn from(value: ConflictResolutionArg) -> Self {
        match value {
            ConflictResolutionArg::GoogleWins => Self::GoogleWins,
            ConflictResolutionArg::IcloudWins => Self::IcloudWins,
            ConflictResolutionArg::DeleteWins => Self::DeleteWins,
            ConflictResolutionArg::UpdateWins => Self::UpdateWins,
        }
    }
}

#[derive(Debug, Subcommand)]
enum DbCommand {
    #[command(about = "List cached provider calendars from SQLite")]
    Calendars,
    #[command(about = "Create a compact SQLite backup with VACUUM INTO")]
    Backup {
        #[arg(value_name = "PATH")]
        output: PathBuf,
        #[arg(long, help = "Replace an existing output file")]
        force: bool,
    },
    #[command(about = "Write a JSON support export of known SQLite tables")]
    Export {
        #[arg(value_name = "PATH")]
        output: PathBuf,
        #[arg(long, help = "Replace an existing output file")]
        force: bool,
    },
    #[command(about = "Import a JSON support export into a SQLite database")]
    Import {
        #[arg(value_name = "PATH")]
        input: PathBuf,
        #[arg(
            long,
            value_name = "PATH",
            help = "Destination DB path; defaults to configured DB"
        )]
        to: Option<PathBuf>,
        #[arg(long, help = "Replace an existing destination database")]
        force: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum BackgroundTemplate {
    Auto,
    Launchd,
    Systemd,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum SetupLocation {
    Local,
    App,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum SetupSecretStore {
    None,
    Os,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum SetupDirection {
    #[value(alias = "two_way")]
    TwoWay,
    #[value(alias = "left-to-right", alias = "left_to_right")]
    GoogleToIcloud,
    #[value(alias = "right-to-left", alias = "right_to_left")]
    IcloudToGoogle,
}

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;
    init_tracing();

    let cli = Cli::parse();
    let Cli { config, command } = cli;

    match command {
        Command::Setup {
            location,
            secret_store,
            force,
            discover,
            csv,
            pair_id,
            google_calendar_id,
            icloud_calendar_id,
            direction,
            disabled,
            google_auth_url,
            google_callback,
            google_code,
            redirect_uri,
            state,
            google_account_label,
            google_client_id,
            google_client_secret,
            icloud_account_label,
            icloud_username,
            icloud_app_password,
            icloud_caldav_url,
            interactive,
        } => {
            let add_pair = pair_id.is_some()
                || google_calendar_id.is_some()
                || icloud_calendar_id.is_some()
                || disabled
                || direction != SetupDirection::TwoWay;
            let google_auth = google_auth_url || google_callback || google_code.is_some();
            let credentials = google_account_label.is_some()
                || google_client_id.is_some()
                || google_client_secret.is_some()
                || icloud_account_label.is_some()
                || icloud_username.is_some()
                || icloud_app_password.is_some()
                || icloud_caldav_url.is_some();
            if [discover, add_pair, google_auth, credentials, interactive]
                .into_iter()
                .filter(|active| *active)
                .count()
                > 1
            {
                bail!("combine only one setup action at a time");
            }

            if interactive {
                setup_interactive(config, location, secret_store, force, redirect_uri, state)
                    .await?;
            } else if discover {
                let (config_path, config) = load_validated_config(config)?;
                let providers = live_providers(config, &config_path)?;
                let calendars = discover_calendars(&providers).await?;
                if let Some(path) = csv {
                    write_calendar_discovery_csv(&path, &calendars)?;
                    println!(
                        "wrote {} calendar row(s) to {}",
                        calendars.len(),
                        path.display()
                    );
                } else {
                    print_calendar_discovery(&calendars);
                }
            } else if add_pair {
                setup_add_pair(
                    config,
                    SetupPairInput {
                        pair_id,
                        google_calendar_id,
                        icloud_calendar_id,
                        direction,
                        enabled: !disabled,
                        force,
                    },
                )?;
            } else if google_auth {
                setup_google_auth(
                    config,
                    SetupGoogleAuthInput {
                        print_url: google_auth_url,
                        callback: google_callback,
                        code: google_code,
                        redirect_uri,
                        state,
                    },
                )
                .await?;
            } else if credentials {
                setup_store_credentials(
                    config,
                    SetupCredentialsInput {
                        google_account_label,
                        google_client_id,
                        google_client_secret,
                        icloud_account_label,
                        icloud_username,
                        icloud_app_password,
                        icloud_caldav_url,
                    },
                )?;
            } else {
                setup_init(config, location, secret_store, force)?;
            }
        }
        Command::Doctor => {
            let (config_path, config) = load_validated_config(config)?;
            let engine = SyncEngine::with_config_path(config.clone(), &config_path);
            let summary = engine.doctor()?;
            println!("config: {}", config_path.display());
            println!("db: {}", summary.db_path.display());
            println!("configured pairs: {}", summary.configured_pair_count);
            println!("enabled pairs: {}", summary.enabled_pair_count);
            println!(
                "unresolved conflicts: {}",
                summary.unresolved_conflict_count
            );
            println!(
                "google credentials: {}",
                if summary.google_credentials_configured {
                    "configured"
                } else {
                    "missing"
                }
            );
            println!(
                "icloud credentials: {}",
                if summary.icloud_credentials_configured {
                    "configured"
                } else {
                    "missing"
                }
            );
            if let Some(run) = summary.latest_run {
                println!(
                    "latest run: status={}, started_at={}, finished_at={}, error={}",
                    run.status,
                    run.started_at,
                    run.finished_at.as_deref().unwrap_or("-"),
                    run.error.as_deref().unwrap_or("-")
                );
            } else {
                println!("latest run: none");
            }
        }
        Command::Db { command } => {
            let (config_path, config) = load_validated_config(config)?;
            run_db_command(&config_path, config, command)?;
        }
        Command::Conflicts {
            resolve,
            resolution,
            details,
            reason,
            pair,
            limit,
            csv,
            dedupe,
            resolve_stale,
            fixtures,
        } => {
            let (config_path, config) = load_validated_config(config)?;
            let engine = SyncEngine::with_config_path(config.clone(), &config_path);

            if let Some(conflict_id) = resolve {
                let Some(resolution) = resolution else {
                    bail!("--resolve requires --resolution");
                };
                let queued = engine.request_conflict_resolution(&conflict_id, resolution.into())?;
                let Some(conflict) = queued else {
                    bail!("unresolved conflict not found: {conflict_id}");
                };
                println!(
                    "queued manual resolution: id={}, pair={}, uid={}, reason={}, resolution={}",
                    conflict.id,
                    conflict.sync_pair_id,
                    conflict.canonical_uid.as_deref().unwrap_or(""),
                    conflict.reason,
                    conflict
                        .manual_resolution
                        .map(|resolution| resolution.as_str())
                        .unwrap_or("")
                );
                println!("run `insync sync --apply` to execute this resolution");
                return Ok(());
            }

            if dedupe {
                let resolved = engine.dedupe_conflicts()?;
                println!("deduped unresolved conflicts: {resolved}");
                return Ok(());
            }

            if resolve_stale {
                if let Some(fixtures) = fixtures {
                    let providers = fixture_providers(&fixtures)?;
                    let summary = engine
                        .resolve_stale_conflicts_with_providers(SyncProviders {
                            google: &providers.google,
                            icloud: &providers.icloud,
                        })
                        .await?;
                    print_stale_conflict_cleanup(&summary);
                } else {
                    let providers = live_providers(config.clone(), &config_path)?;
                    let summary = engine
                        .resolve_stale_conflicts_with_providers(SyncProviders {
                            google: &providers.google,
                            icloud: &providers.icloud,
                        })
                        .await?;
                    print_stale_conflict_cleanup(&summary);
                }
                return Ok(());
            }

            if details || reason.is_some() || pair.is_some() {
                let rows = engine.conflict_details(ConflictFilter {
                    sync_pair_id: pair,
                    reason,
                    limit: Some(limit),
                })?;

                if let Some(path) = csv {
                    write_conflict_details_csv(&path, &rows)?;
                    println!(
                        "wrote {} conflict detail row(s) to {}",
                        rows.len(),
                        path.display()
                    );
                } else {
                    print_conflict_details(&rows);
                }
            } else {
                let rows = engine.conflict_summaries()?;

                if let Some(path) = csv {
                    write_conflict_summary_csv(&path, &rows)?;
                    println!(
                        "wrote {} conflict summary row(s) to {}",
                        rows.len(),
                        path.display()
                    );
                } else {
                    print_conflict_summaries(&rows);
                }
            }
        }
        Command::Sync {
            apply,
            fixtures,
            report,
            report_all,
            summary_json,
        } => {
            let (config_path, config) = load_validated_config(config)?;
            let engine = SyncEngine::with_config_path(config.clone(), &config_path);
            if let Some(fixtures) = fixtures {
                let providers = fixture_providers(&fixtures)?;
                let summary = engine
                    .plan_once_with_providers_and_report_mode(
                        if apply {
                            RunMode::Apply
                        } else {
                            RunMode::DryRun
                        },
                        SyncProviders {
                            google: &providers.google,
                            icloud: &providers.icloud,
                        },
                        if report_all {
                            ReportMode::AllActions
                        } else {
                            ReportMode::ActionsOnly
                        },
                    )
                    .await?;
                if let Some(path) = report {
                    engine.write_dry_run_report(&path, &summary.report_rows)?;
                    println!(
                        "wrote {} report row(s) to {}",
                        summary.report_rows.len(),
                        path.display()
                    );
                }
                if let Some(path) = summary_json.as_ref() {
                    engine.write_sync_summary_json(path, &summary)?;
                    println!("wrote sync summary to {}", path.display());
                }
                println!(
                    "planned Rust sync: db={}, configured_pairs={}, enabled_pairs={}, action_counts={:?}, resolution_counts={:?}, mode={:?}",
                    summary.db_path.display(),
                    summary.configured_pair_count,
                    summary.enabled_pair_count,
                    summary.action_counts,
                    summary.resolution_counts,
                    summary.mode
                );
                print_pair_plan_summaries(&summary.pair_summaries);
            } else {
                let providers = live_providers(config.clone(), &config_path)?;
                let summary = engine
                    .plan_once_with_providers_and_report_mode(
                        if apply {
                            RunMode::Apply
                        } else {
                            RunMode::DryRun
                        },
                        SyncProviders {
                            google: &providers.google,
                            icloud: &providers.icloud,
                        },
                        if report_all {
                            ReportMode::AllActions
                        } else {
                            ReportMode::ActionsOnly
                        },
                    )
                    .await?;
                if let Some(path) = report {
                    engine.write_dry_run_report(&path, &summary.report_rows)?;
                    println!(
                        "wrote {} report row(s) to {}",
                        summary.report_rows.len(),
                        path.display()
                    );
                }
                if let Some(path) = summary_json.as_ref() {
                    engine.write_sync_summary_json(path, &summary)?;
                    println!("wrote sync summary to {}", path.display());
                }
                println!(
                    "planned live Rust sync: db={}, configured_pairs={}, enabled_pairs={}, action_counts={:?}, resolution_counts={:?}, mode={:?}",
                    summary.db_path.display(),
                    summary.configured_pair_count,
                    summary.enabled_pair_count,
                    summary.action_counts,
                    summary.resolution_counts,
                    summary.mode
                );
                print_pair_plan_summaries(&summary.pair_summaries);
            }
        }
        Command::Daemon { apply } => {
            let (config_path, config) = load_validated_config(config)?;
            let engine = SyncEngine::with_config_path(config.clone(), &config_path);
            let providers = live_providers(config.clone(), &config_path)?;
            println!("starting daemon; press Ctrl-C to stop");
            engine
                .run_forever_with_providers(
                    if apply {
                        RunMode::Apply
                    } else {
                        RunMode::DryRun
                    },
                    SyncProviders {
                        google: &providers.google,
                        icloud: &providers.icloud,
                    },
                    async {
                        let _ = tokio::signal::ctrl_c().await;
                    },
                )
                .await?;
        }
        Command::Background { command } => match command {
            BackgroundCommand::Install { apply, force } => {
                let (config_path, _) = load_validated_config(config)?;
                install_background_service(&config_path, apply, force)?;
            }
            BackgroundCommand::Uninstall => {
                uninstall_background_service()?;
            }
            BackgroundCommand::Status => {
                print_background_status()?;
            }
            BackgroundCommand::Print { template, apply } => {
                let (config_path, _) = load_validated_config(config)?;
                print_background_template(template, &config_path, apply)?;
            }
        },
        Command::Tui => {
            let (config_path, config) = load_validated_config(config)?;
            let engine = SyncEngine::with_config_path(config.clone(), &config_path);
            let doctor = engine.doctor()?;
            let mut model = AppModel::from_config(&config);
            model.apply_runtime_snapshot(runtime_snapshot_from_doctor(&doctor, &config)?);
            run_tui(model)?;
        }
    }

    Ok(())
}

fn load_validated_config(
    config: Option<PathBuf>,
) -> Result<(PathBuf, insync_config::ServiceConfig)> {
    let config_path = resolve_config_path(config)?;
    let config = load_config(&config_path)
        .wrap_err_with(|| format!("loading config {}", config_path.display()))?;
    validate_config(&config)
        .wrap_err_with(|| format!("validating config {}", config_path.display()))?;
    Ok((config_path, config))
}

fn run_db_command(
    config_path: &Path,
    config: insync_config::ServiceConfig,
    command: DbCommand,
) -> Result<()> {
    let db_path = SyncEngine::with_config_path(config.clone(), config_path).db_path();

    match command {
        DbCommand::Calendars => {
            let conn = open(&db_path)?;
            migrate(&conn)?;
            seed_configured_pairs(&conn, &config)?;
            let calendars = list_calendars(&conn)?;
            println!("db: {}", db_path.display());
            if calendars.is_empty() {
                println!("cached calendars: none");
                return Ok(());
            }
            println!("cached calendars: {}", calendars.len());
            for calendar in calendars {
                println!(
                    "{}\t{}\t{}\t{}\t{}\t{}",
                    calendar.provider,
                    calendar.account_email,
                    if calendar.writable {
                        "writable"
                    } else {
                        "read-only"
                    },
                    calendar.name.as_deref().unwrap_or("-"),
                    calendar.timezone.as_deref().unwrap_or("-"),
                    calendar.provider_calendar_id
                );
            }
        }
        DbCommand::Backup { output, force } => {
            prepare_output_path(&output, force)?;
            backup_database(&db_path, &output)?;
            println!("backed up {} to {}", db_path.display(), output.display());
        }
        DbCommand::Export { output, force } => {
            prepare_output_path(&output, force)?;
            let export = export_database_json(&db_path, &output)?;
            let row_count = export
                .tables
                .values()
                .map(std::vec::Vec::len)
                .sum::<usize>();
            println!(
                "exported {} table(s), {} row(s) from {} to {}",
                export.tables.len(),
                row_count,
                db_path.display(),
                output.display()
            );
        }
        DbCommand::Import { input, to, force } => {
            let destination = to.unwrap_or(db_path);
            prepare_output_path(&destination, force)?;
            import_database_json(&input, &destination)?;
            println!("imported {} to {}", input.display(), destination.display());
        }
    }

    Ok(())
}

fn prepare_output_path(path: &Path, force: bool) -> Result<()> {
    if path.exists() {
        if !force {
            bail!(
                "{} already exists; rerun with --force to replace it",
                path.display()
            );
        }
        fs::remove_file(path).wrap_err_with(|| format!("removing {}", path.display()))?;
    }

    Ok(())
}

const BACKGROUND_LABEL: &str = "dev.bkniffler.insync";
const SYSTEMD_UNIT_NAME: &str = "insync.service";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BackgroundPlatform {
    Launchd,
    Systemd,
}

fn install_background_service(config_path: &Path, apply: bool, force: bool) -> Result<()> {
    match current_background_platform()? {
        BackgroundPlatform::Launchd => install_launchd_service(config_path, apply, force),
        BackgroundPlatform::Systemd => install_systemd_service(config_path, apply, force),
    }
}

fn uninstall_background_service() -> Result<()> {
    match current_background_platform()? {
        BackgroundPlatform::Launchd => uninstall_launchd_service(),
        BackgroundPlatform::Systemd => uninstall_systemd_service(),
    }
}

fn print_background_status() -> Result<()> {
    match current_background_platform()? {
        BackgroundPlatform::Launchd => print_launchd_status(),
        BackgroundPlatform::Systemd => print_systemd_status(),
    }
}

fn print_background_template(
    template: BackgroundTemplate,
    config_path: &Path,
    apply: bool,
) -> Result<()> {
    let platform = match template {
        BackgroundTemplate::Auto => current_background_platform()?,
        BackgroundTemplate::Launchd => BackgroundPlatform::Launchd,
        BackgroundTemplate::Systemd => BackgroundPlatform::Systemd,
    };
    let binary_path = current_binary_path()?;
    let config_path = absolute_config_path(config_path)?;

    match platform {
        BackgroundPlatform::Launchd => {
            let logs = launchd_log_paths()?;
            print!(
                "{}",
                render_launchd_plist(&binary_path, &config_path, apply, &logs)
            );
        }
        BackgroundPlatform::Systemd => {
            print!("{}", render_systemd_unit(&binary_path, &config_path, apply));
        }
    }

    Ok(())
}

fn current_background_platform() -> Result<BackgroundPlatform> {
    match env::consts::OS {
        "macos" => Ok(BackgroundPlatform::Launchd),
        "linux" => Ok(BackgroundPlatform::Systemd),
        "windows" => bail!(
            "Windows background install is not implemented yet; use a scheduled task that runs `insync daemon --apply` for now"
        ),
        other => bail!("background install is not supported on {other}"),
    }
}

fn install_launchd_service(config_path: &Path, apply: bool, force: bool) -> Result<()> {
    let plist_path = launchd_plist_path()?;
    if plist_path.exists() && !force {
        bail!(
            "{} already exists; rerun with --force to replace it",
            plist_path.display()
        );
    }

    let binary_path = current_binary_path()?;
    let config_path = absolute_config_path(config_path)?;
    let logs = launchd_log_paths()?;
    fs::create_dir_all(&logs.dir)?;
    write_text(
        &plist_path,
        &render_launchd_plist(&binary_path, &config_path, apply, &logs),
    )?;

    let domain = launchd_domain()?;
    let _ = run_command_output(
        "launchctl",
        vec![
            "bootout".to_string(),
            domain.clone(),
            plist_path_display(&plist_path),
        ],
    );
    run_command_checked(
        "launchctl",
        vec![
            "bootstrap".to_string(),
            domain.clone(),
            plist_path_display(&plist_path),
        ],
    )?;
    run_command_checked(
        "launchctl",
        vec!["enable".to_string(), format!("{domain}/{BACKGROUND_LABEL}")],
    )?;

    println!("installed launchd user agent: {}", plist_path.display());
    println!("logs: {}", logs.dir.display());
    println!("status: insync background status");
    Ok(())
}

fn uninstall_launchd_service() -> Result<()> {
    let plist_path = launchd_plist_path()?;
    let domain = launchd_domain()?;
    let _ = run_command_output(
        "launchctl",
        vec![
            "bootout".to_string(),
            domain,
            plist_path_display(&plist_path),
        ],
    );

    if plist_path.exists() {
        fs::remove_file(&plist_path)
            .wrap_err_with(|| format!("removing {}", plist_path.display()))?;
    }

    println!("removed launchd user agent: {}", plist_path.display());
    Ok(())
}

fn print_launchd_status() -> Result<()> {
    let plist_path = launchd_plist_path()?;
    let logs = launchd_log_paths()?;
    println!("platform: launchd");
    println!("label: {BACKGROUND_LABEL}");
    println!("definition: {}", plist_path.display());
    println!("logs: {}", logs.dir.display());

    let domain = launchd_domain()?;
    let output = run_command_output(
        "launchctl",
        vec!["print".to_string(), format!("{domain}/{BACKGROUND_LABEL}")],
    )?;
    print_command_output(&output);
    Ok(())
}

fn install_systemd_service(config_path: &Path, apply: bool, force: bool) -> Result<()> {
    let unit_path = systemd_unit_path()?;
    if unit_path.exists() && !force {
        bail!(
            "{} already exists; rerun with --force to replace it",
            unit_path.display()
        );
    }

    let binary_path = current_binary_path()?;
    let config_path = absolute_config_path(config_path)?;
    write_text(
        &unit_path,
        &render_systemd_unit(&binary_path, &config_path, apply),
    )?;

    run_command_checked("systemctl", ["--user", "daemon-reload"])?;
    run_command_checked(
        "systemctl",
        ["--user", "enable", "--now", SYSTEMD_UNIT_NAME],
    )?;

    println!("installed systemd user service: {}", unit_path.display());
    println!("logs: journalctl --user -u {SYSTEMD_UNIT_NAME}");
    println!("status: insync background status");
    Ok(())
}

fn uninstall_systemd_service() -> Result<()> {
    let unit_path = systemd_unit_path()?;
    let _ = run_command_output(
        "systemctl",
        ["--user", "disable", "--now", SYSTEMD_UNIT_NAME],
    );

    if unit_path.exists() {
        fs::remove_file(&unit_path)
            .wrap_err_with(|| format!("removing {}", unit_path.display()))?;
    }

    let _ = run_command_output("systemctl", ["--user", "daemon-reload"]);
    println!("removed systemd user service: {}", unit_path.display());
    Ok(())
}

fn print_systemd_status() -> Result<()> {
    let unit_path = systemd_unit_path()?;
    println!("platform: systemd --user");
    println!("unit: {SYSTEMD_UNIT_NAME}");
    println!("definition: {}", unit_path.display());
    println!("logs: journalctl --user -u {SYSTEMD_UNIT_NAME}");

    let output = run_command_output("systemctl", ["--user", "status", SYSTEMD_UNIT_NAME])?;
    print_command_output(&output);
    Ok(())
}

fn render_launchd_plist(
    binary_path: &Path,
    config_path: &Path,
    apply: bool,
    logs: &LaunchdLogPaths,
) -> String {
    let args = daemon_arguments(binary_path, config_path, apply);
    let program_arguments = args
        .iter()
        .map(|arg| format!("        <string>{}</string>", xml_escape(arg)))
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{}</string>
    <key>ProgramArguments</key>
    <array>
{}
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>StandardOutPath</key>
    <string>{}</string>
    <key>StandardErrorPath</key>
    <string>{}</string>
    <key>EnvironmentVariables</key>
    <dict>
        <key>RUST_LOG</key>
        <string>info</string>
    </dict>
</dict>
</plist>
"#,
        xml_escape(BACKGROUND_LABEL),
        program_arguments,
        xml_escape(&logs.stdout.to_string_lossy()),
        xml_escape(&logs.stderr.to_string_lossy())
    )
}

fn render_systemd_unit(binary_path: &Path, config_path: &Path, apply: bool) -> String {
    let exec_start = daemon_arguments(binary_path, config_path, apply)
        .iter()
        .map(|arg| systemd_escape_arg(arg))
        .collect::<Vec<_>>()
        .join(" ");

    format!(
        r#"[Unit]
Description=insync calendar sync daemon
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
ExecStart={}
Restart=on-failure
RestartSec=30
Environment=RUST_LOG=info

[Install]
WantedBy=default.target
"#,
        exec_start
    )
}

fn daemon_arguments(binary_path: &Path, config_path: &Path, apply: bool) -> Vec<String> {
    let mut args = vec![
        binary_path.to_string_lossy().into_owned(),
        "--config".to_string(),
        config_path.to_string_lossy().into_owned(),
        "daemon".to_string(),
    ];
    if apply {
        args.push("--apply".to_string());
    }
    args
}

fn xml_escape(value: impl AsRef<str>) -> String {
    value
        .as_ref()
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn systemd_escape_arg(value: &str) -> String {
    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '/' | '.' | '_' | '-' | ':' | '@'))
    {
        return value.to_string();
    }

    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

fn current_binary_path() -> Result<PathBuf> {
    env::current_exe().wrap_err("resolving current insync executable")
}

fn absolute_config_path(config_path: &Path) -> Result<PathBuf> {
    fs::canonicalize(config_path)
        .wrap_err_with(|| format!("resolving config path {}", config_path.display()))
}

fn home_dir() -> Result<PathBuf> {
    env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .ok_or_else(|| color_eyre::eyre::eyre!("HOME is not set"))
}

fn launchd_plist_path() -> Result<PathBuf> {
    Ok(home_dir()?
        .join("Library")
        .join("LaunchAgents")
        .join(format!("{BACKGROUND_LABEL}.plist")))
}

#[derive(Debug)]
struct LaunchdLogPaths {
    dir: PathBuf,
    stdout: PathBuf,
    stderr: PathBuf,
}

fn launchd_log_paths() -> Result<LaunchdLogPaths> {
    let dir = home_dir()?.join("Library").join("Logs").join("insync");
    Ok(LaunchdLogPaths {
        stdout: dir.join("daemon.out.log"),
        stderr: dir.join("daemon.err.log"),
        dir,
    })
}

fn launchd_domain() -> Result<String> {
    Ok(format!("gui/{}", current_uid()?))
}

fn current_uid() -> Result<String> {
    let output = run_command_output("id", ["-u"])?;
    if !output.status_success {
        bail!("failed to determine current uid: {}", output.stderr.trim());
    }

    Ok(output.stdout.trim().to_string())
}

fn systemd_unit_path() -> Result<PathBuf> {
    let config_home = if let Some(path) = env::var_os("XDG_CONFIG_HOME").map(PathBuf::from) {
        path
    } else {
        home_dir()?.join(".config")
    };
    Ok(config_home
        .join("systemd")
        .join("user")
        .join(SYSTEMD_UNIT_NAME))
}

#[derive(Debug)]
struct CommandOutput {
    args: Vec<String>,
    status_success: bool,
    stdout: String,
    stderr: String,
}

fn run_command_checked<I, S>(program: &str, args: I) -> Result<()>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let args = args.into_iter().map(Into::into).collect::<Vec<_>>();
    let output = run_command_output(program, args)?;
    if output.status_success {
        return Ok(());
    }

    bail!(
        "{} failed: {}",
        command_display(program, &output.args),
        output.stderr.trim()
    )
}

fn run_command_output<I, S>(program: &str, args: I) -> Result<CommandOutput>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let args = args.into_iter().map(Into::into).collect::<Vec<_>>();
    let output = ProcessCommand::new(program)
        .args(&args)
        .output()
        .wrap_err_with(|| format!("running {}", command_display(program, &args)))?;

    Ok(CommandOutput {
        args,
        status_success: output.status.success(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

fn print_command_output(output: &CommandOutput) {
    if !output.stdout.is_empty() {
        print!("{}", output.stdout);
    }
    if !output.stderr.is_empty() {
        eprint!("{}", output.stderr);
    }
    if !output.status_success {
        println!("status: not running or unavailable");
    }
}

fn command_display(program: &str, args: &[String]) -> String {
    std::iter::once(program.to_string())
        .chain(args.iter().cloned())
        .collect::<Vec<_>>()
        .join(" ")
}

fn plist_path_display(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;

    #[test]
    fn systemd_unit_renders_daemon_arguments_and_escapes_paths() {
        let unit = render_systemd_unit(
            Path::new("/opt/insync/bin/insync"),
            Path::new("/home/test/Application Support/insync.json"),
            true,
        );

        assert!(unit.contains("Description=insync calendar sync daemon"));
        assert!(unit.contains(
            "ExecStart=/opt/insync/bin/insync --config \"/home/test/Application Support/insync.json\" daemon --apply"
        ));
        assert!(unit.contains("Restart=on-failure"));
        assert!(unit.contains("WantedBy=default.target"));
    }

    #[test]
    fn launchd_plist_renders_array_arguments_logs_and_xml_escapes() {
        let logs = LaunchdLogPaths {
            dir: PathBuf::from("/Users/test/Library/Logs/insync"),
            stdout: PathBuf::from("/Users/test/Library/Logs/insync/out&sync.log"),
            stderr: PathBuf::from("/Users/test/Library/Logs/insync/err.log"),
        };
        let plist = render_launchd_plist(
            Path::new("/opt/in&sync/bin/insync"),
            Path::new("/Users/test/insync<local>.json"),
            true,
            &logs,
        );

        assert!(plist.contains("<string>dev.bkniffler.insync</string>"));
        assert!(plist.contains("<string>/opt/in&amp;sync/bin/insync</string>"));
        assert!(plist.contains("<string>/Users/test/insync&lt;local&gt;.json</string>"));
        assert!(plist.contains("<string>--apply</string>"));
        assert!(
            plist.contains("<string>/Users/test/Library/Logs/insync/out&amp;sync.log</string>")
        );
        assert!(plist.contains("<key>RunAtLoad</key>"));
    }

    #[test]
    fn tui_dashboard_render_covers_empty_pair_state_and_commands() {
        let output = render_tui_to_text(&test_model(), 120, 34);

        assert!(output.contains("insync"));
        assert!(output.contains("No calendar pairs configured."));
        assert!(output.contains("Run setup or press s"));
        assert!(output.contains(": pal"));
        assert!(output.contains("b pause"));
        assert!(output.contains("No runs yet"));
        assert!(output.contains("Notifications"));
        assert!(output.contains("Info No calendar pairs configured"));
    }

    #[test]
    fn tui_runs_render_covers_history_filter_and_detail() {
        let mut model = test_model();
        model.view = AppView::Runs;
        model.runs = vec![
            AppRun {
                id: "run-failed".to_string(),
                pair_id: Some("personal".to_string()),
                status: "failed".to_string(),
                started_at: "2026-06-02T12:00:00Z".to_string(),
                finished_at: Some("2026-06-02T12:01:00Z".to_string()),
                error: Some("auth failed".to_string()),
            },
            AppRun {
                id: "run-ok".to_string(),
                pair_id: Some("work".to_string()),
                status: "completed".to_string(),
                started_at: "2026-06-02T11:00:00Z".to_string(),
                finished_at: Some("2026-06-02T11:01:00Z".to_string()),
                error: None,
            },
        ];
        model.selected_run_id = Some("run-failed".to_string());
        model.run_filter = AppRunFilter::All;

        let output = render_tui_to_text(&model, 130, 36);

        assert!(output.contains("Sync Runs (2/2)"));
        assert!(output.contains("run-failed"));
        assert!(output.contains("personal"));
        assert!(output.contains("auth failed"));
        assert!(output.contains("f filter"));
    }

    #[test]
    fn tui_command_palette_render_covers_actions_and_selected_command() {
        let mut model = test_model();
        model.command_palette_open = true;
        model.selected_command_index = 1;

        let output = render_tui_to_text(&model, 120, 34);

        assert!(output.contains("Command Palette"));
        assert!(output.contains("> Apply sync"));
        assert!(output.contains("Dry-run sync"));
        assert!(output.contains("Pause background"));
        assert!(output.contains("Show dry-run report"));
        assert!(output.contains("Export report"));
        assert!(output.contains("enter run"));
        assert!(output.contains("esc close"));
    }

    #[test]
    fn tui_setup_render_covers_guided_checklist_and_next_action() {
        let mut model = test_model();
        model.view = AppView::Setup;
        model.setup = AppSetupState {
            secret_store: "os".to_string(),
            db_path: ".insync/insync.db".to_string(),
            log_level: "info".to_string(),
            google_account_label: "personal".to_string(),
            google_client_id_configured: true,
            google_client_secret_inline: false,
            google_refresh_token_inline: false,
            icloud_account_label: "personal".to_string(),
            icloud_username_configured: true,
            icloud_app_password_inline: false,
            icloud_caldav_url: "https://caldav.icloud.com".to_string(),
            poll_interval_seconds: 300,
        };
        model.pairs = vec![insync_app::AppPair {
            id: "personal".to_string(),
            enabled: true,
            direction: insync_core::SyncDirection::TwoWay,
            google_calendar_id: "primary".to_string(),
            icloud_calendar_id: "https://caldav.example/cal".to_string(),
            google_calendar_name: None,
            icloud_calendar_name: None,
            google_account_label: Some("me@gmail.com".to_string()),
            icloud_account_label: Some("me@icloud.com".to_string()),
            google_last_sync_at: None,
            icloud_last_sync_at: None,
        }];
        model.selected_pair_id = Some("personal".to_string());

        let output = render_tui_to_text(&model, 132, 36);

        assert!(output.contains("Setup Wizard"));
        assert!(output.contains("Ready Google OAuth"));
        assert!(output.contains("Ready iCloud"));
        assert!(output.contains("Check Discovery"));
        assert!(output.contains("Next: Discovery"));
        assert!(output.contains("Run insync setup --discover"));
        assert!(output.contains("Selected Pair"));
        assert!(output.contains("primary"));
        assert!(output.contains("s set"));
    }

    #[test]
    fn tui_reports_render_filter_sort_rows_and_detail() {
        let mut model = test_model();
        model.view = AppView::Reports;
        model.report_filter = AppReportFilter::All;
        model.report_sort = AppReportSort::Pair;
        model.selected_report_index = Some(0);
        model.report_rows = vec![
            AppReportRow {
                pair_id: "work".to_string(),
                action: "update_google".to_string(),
                reason: "google_changed".to_string(),
                resolution: "apply".to_string(),
                title: "Budget review".to_string(),
                google_present: "yes".to_string(),
                icloud_present: "yes".to_string(),
                diff_fields: "title,start".to_string(),
            },
            AppReportRow {
                pair_id: "personal".to_string(),
                action: "create_icloud".to_string(),
                reason: "missing_icloud".to_string(),
                resolution: "apply".to_string(),
                title: "Dentist".to_string(),
                google_present: "yes".to_string(),
                icloud_present: "no".to_string(),
                diff_fields: String::new(),
            },
        ];

        let output = render_tui_to_text(&model, 132, 36);

        assert!(output.contains("Dry-Run Report (2/2, filter all, sort pair)"));
        assert!(output.contains("personal"));
        assert!(output.contains("create_icloud"));
        assert!(output.contains("Dentist"));
        assert!(output.contains("Report Detail"));
        assert!(output.contains("Present: Google yes / iCloud no"));
        assert!(output.contains("v report"));
        assert!(output.contains("t sort"));
    }

    #[test]
    fn tui_conflicts_render_covers_group_and_detail_rows() {
        let mut model = test_model();
        model.view = AppView::Conflicts;
        model.conflict_count = 2;
        model.selected_conflict_index = Some(0);
        model.conflict_summaries = vec![AppConflictSummary {
            pair_id: "personal".to_string(),
            reason: "both_sides_changed".to_string(),
            count: 2,
            first_seen_at: "2026-06-02 12:00:00".to_string(),
            last_seen_at: "2026-06-02 12:01:00".to_string(),
        }];
        model.conflict_details = vec![AppConflictDetail {
            id: "conflict-1".to_string(),
            pair_id: "personal".to_string(),
            event_link_id: Some("link-1".to_string()),
            canonical_uid: Some("uid-1".to_string()),
            reason: "both_sides_changed".to_string(),
            resolution_policy: "manual review; optional newest/google/icloud winner".to_string(),
            google_title: Some("Planning Google".to_string()),
            icloud_title: Some("Planning iCloud".to_string()),
            google_start: Some("2026-06-02T12:00:00Z".to_string()),
            icloud_start: Some("2026-06-02T13:00:00Z".to_string()),
            google_status: Some("confirmed".to_string()),
            icloud_status: Some("tentative".to_string()),
            google_event_id: Some("google-1".to_string()),
            icloud_href: Some("/cal/icloud-1.ics".to_string()),
            diff_fields: "title|start|status".to_string(),
            created_at: "2026-06-02 12:01:00".to_string(),
        }];

        let output = render_tui_to_text(&model, 130, 36);

        assert!(output.contains("Conflict Groups"));
        assert!(output.contains("personal"));
        assert!(output.contains("both_sides_changed"));
        assert!(output.contains("Conflict Events (2)"));
        assert!(output.contains("Conflict Comparison"));
        assert!(output.contains("Planning Google"));
        assert!(output.contains("Planning iCloud"));
        assert!(output.contains("Policy: manual review"));
        assert!(output.contains("Audit: unresolved since"));
        assert!(output.contains("uid-1"));
        assert!(output.contains("c conf"));
    }

    #[test]
    fn conflict_snapshot_mapper_extracts_comparison_fields() {
        let detail = conflict_detail_snapshot(UnresolvedConflictRow {
            id: "conflict-1".to_string(),
            sync_pair_id: "personal".to_string(),
            event_link_id: Some("link-1".to_string()),
            canonical_uid: Some("uid-1".to_string()),
            reason: "both_sides_changed".to_string(),
            google_snapshot: Some(serde_json::json!({
                "title": "Google title",
                "status": "confirmed",
                "start": {
                    "kind": "dateTime",
                    "value": "2026-06-02T12:00:00Z",
                    "timezone": "Europe/Berlin"
                },
                "providerMeta": { "eventId": "google-1" }
            })),
            icloud_snapshot: Some(serde_json::json!({
                "title": "iCloud title",
                "status": "tentative",
                "start": {
                    "kind": "date",
                    "value": "2026-06-03"
                },
                "providerMeta": { "href": "/cal/icloud-1.ics" }
            })),
            manual_resolution: None,
            resolution_requested_at: None,
            created_at: "2026-06-02 12:01:00".to_string(),
        });

        assert_eq!(detail.google_title.as_deref(), Some("Google title"));
        assert_eq!(detail.icloud_title.as_deref(), Some("iCloud title"));
        assert_eq!(
            detail.google_start.as_deref(),
            Some("2026-06-02T12:00:00Z [Europe/Berlin]")
        );
        assert_eq!(detail.icloud_start.as_deref(), Some("2026-06-03"));
        assert_eq!(detail.google_event_id.as_deref(), Some("google-1"));
        assert_eq!(detail.icloud_href.as_deref(), Some("/cal/icloud-1.ics"));
        assert_eq!(detail.diff_fields, "title|start|status");
        assert!(detail.resolution_policy.contains("manual review"));
    }

    #[test]
    fn tui_notifications_render_failed_sync_and_conflicts() {
        let mut model = test_model();
        model.conflict_count = 2;
        model.recent_error = Some("auth failed while refreshing Google".to_string());

        let output = render_tui_to_text(&model, 120, 34);

        assert!(output.contains("Notifications"));
        assert!(output.contains("Error auth failed while refreshing Google"));
        assert!(output.contains("Warning 2 unresolved conflict(s)"));
        assert!(output.contains("Info No calendar pairs configured"));
        assert!(output.contains("q quit"));
    }

    #[test]
    fn tui_background_pause_state_renders_resume_action() {
        let mut model = test_model();
        model.background_paused = true;
        model.command_palette_open = true;
        model.selected_command_index = 8;

        let output = render_tui_to_text(&model, 120, 34);

        assert!(output.contains("b resume"));
        assert!(output.contains("> Resume background"));
        assert!(output.contains("Background sync is paused"));
    }

    fn test_model() -> AppModel {
        AppModel {
            status: AppStatus::Idle,
            view: AppView::Dashboard,
            command_palette_open: false,
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
            conflict_summaries: Vec::new(),
            conflict_details: Vec::new(),
        }
    }

    fn render_tui_to_text(model: &AppModel, width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw_tui(frame, model)).unwrap();

        let buffer = terminal.backend().buffer();
        buffer
            .content()
            .chunks(usize::from(buffer.area().width))
            .map(|row| {
                row.iter()
                    .map(|cell| cell.symbol())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn setup_init(
    explicit_config: Option<PathBuf>,
    location: SetupLocation,
    secret_store: SetupSecretStore,
    force: bool,
) -> Result<()> {
    let config_path = setup_config_path(explicit_config, location)?;
    if config_path.exists() && !force {
        bail!(
            "config already exists at {}; pass --force to overwrite",
            config_path.display()
        );
    }

    let config = insync_config::ServiceConfig {
        secret_store: match secret_store {
            SetupSecretStore::None => SecretStoreKind::None,
            SetupSecretStore::Os => SecretStoreKind::Os,
        },
        ..insync_config::ServiceConfig::default()
    };
    validate_config(&config)?;
    save_config(&config_path, &config)
        .wrap_err_with(|| format!("writing config {}", config_path.display()))?;

    println!("created config: {}", config_path.display());
    println!("secret store: {}", secret_store_label(config.secret_store));
    println!("next: edit calendar pairs, then run insync doctor");
    Ok(())
}

fn setup_config_path(explicit_config: Option<PathBuf>, location: SetupLocation) -> Result<PathBuf> {
    if let Some(path) = explicit_config {
        return Ok(path);
    }

    match location {
        SetupLocation::Local => Ok(PathBuf::from(LOCAL_CONFIG_FILE)),
        SetupLocation::App => Ok(app_config_path()?),
    }
}

fn secret_store_label(secret_store: SecretStoreKind) -> &'static str {
    match secret_store {
        SecretStoreKind::None => "none",
        SecretStoreKind::Os => "os",
    }
}

#[derive(Debug, Clone)]
struct SetupPairInput {
    pair_id: Option<String>,
    google_calendar_id: Option<String>,
    icloud_calendar_id: Option<String>,
    direction: SetupDirection,
    enabled: bool,
    force: bool,
}

fn setup_add_pair(explicit_config: Option<PathBuf>, input: SetupPairInput) -> Result<()> {
    let config_path = resolve_config_path(explicit_config)?;
    let mut config = load_config(&config_path)
        .wrap_err_with(|| format!("loading config {}", config_path.display()))?;
    let pair = SyncPairConfig {
        id: required_setup_value(input.pair_id, "--pair-id")?,
        enabled: input.enabled,
        direction: setup_direction(input.direction),
        google_calendar_id: required_setup_value(input.google_calendar_id, "--google-calendar-id")?,
        icloud_calendar_id: required_setup_value(input.icloud_calendar_id, "--icloud-calendar-id")?,
    };

    if let Some(index) = config
        .sync
        .pairs
        .iter()
        .position(|existing| existing.id == pair.id)
    {
        if !input.force {
            bail!(
                "sync pair {} already exists in {}; pass --force to replace it",
                pair.id,
                config_path.display()
            );
        }
        config.sync.pairs[index] = pair;
    } else {
        config.sync.pairs.push(pair);
    }

    validate_config(&config)
        .wrap_err_with(|| format!("validating config {}", config_path.display()))?;
    save_config(&config_path, &config)
        .wrap_err_with(|| format!("writing config {}", config_path.display()))?;
    println!("saved config: {}", config_path.display());
    println!("configured pairs: {}", config.sync.pairs.len());
    Ok(())
}

fn required_setup_value(value: Option<String>, flag: &str) -> Result<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| color_eyre::eyre::eyre!("{flag} is required"))
}

fn setup_direction(direction: SetupDirection) -> SyncDirection {
    match direction {
        SetupDirection::TwoWay => SyncDirection::TwoWay,
        SetupDirection::GoogleToIcloud => SyncDirection::LeftToRight,
        SetupDirection::IcloudToGoogle => SyncDirection::RightToLeft,
    }
}

#[derive(Debug, Clone)]
struct SetupCredentialsInput {
    google_account_label: Option<String>,
    google_client_id: Option<String>,
    google_client_secret: Option<String>,
    icloud_account_label: Option<String>,
    icloud_username: Option<String>,
    icloud_app_password: Option<String>,
    icloud_caldav_url: Option<String>,
}

fn setup_store_credentials(
    explicit_config: Option<PathBuf>,
    input: SetupCredentialsInput,
) -> Result<()> {
    let config_path = resolve_config_path(explicit_config)?;
    let mut config = load_config(&config_path)
        .wrap_err_with(|| format!("loading config {}", config_path.display()))?;

    if let Some(value) = optional_setup_value(input.google_account_label) {
        config.google.account_label = value;
    }
    if let Some(value) = optional_setup_value(input.google_client_id) {
        config.google.client_id = Some(value);
    }
    if let Some(value) = optional_setup_value(input.icloud_account_label) {
        config.icloud.account_label = value;
    }
    if let Some(value) = optional_setup_value(input.icloud_username) {
        config.icloud.username = Some(value);
    }
    if let Some(value) = optional_setup_value(input.icloud_caldav_url) {
        config.icloud.caldav_url = value;
    }

    validate_config(&config)
        .wrap_err_with(|| format!("validating config {}", config_path.display()))?;
    save_config(&config_path, &config)
        .wrap_err_with(|| format!("writing config {}", config_path.display()))?;

    if let Some(value) = optional_setup_value(input.google_client_secret) {
        store_google_client_secret(&mut config, &config_path, &value).wrap_err_with(|| {
            format!("storing Google client secret for {}", config_path.display())
        })?;
    }
    if let Some(value) = optional_setup_value(input.icloud_app_password) {
        store_icloud_app_password(&mut config, &config_path, &value).wrap_err_with(|| {
            format!("storing iCloud app password for {}", config_path.display())
        })?;
    }

    println!("saved config: {}", config_path.display());
    println!("secret store: {}", secret_store_label(config.secret_store));
    Ok(())
}

fn optional_setup_value(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

async fn setup_interactive(
    explicit_config: Option<PathBuf>,
    location: SetupLocation,
    secret_store: SetupSecretStore,
    force: bool,
    redirect_uri: String,
    state: String,
) -> Result<()> {
    let config_path = setup_config_path(explicit_config, location)?;
    if !config_path.exists() {
        setup_init(Some(config_path.clone()), location, secret_store, force)?;
    }

    let mut config = load_config(&config_path)
        .wrap_err_with(|| format!("loading config {}", config_path.display()))?;

    println!("Interactive setup for {}", config_path.display());
    let google_client_id = prompt_text("Google client ID", config.google.client_id.as_deref())?;
    if let Some(value) = optional_setup_value(Some(google_client_id)) {
        config.google.client_id = Some(value);
    }
    let google_client_secret = prompt_secret(
        "Google client secret",
        config.google.client_secret.is_some(),
    )?;
    let icloud_username = prompt_text("iCloud username", config.icloud.username.as_deref())?;
    if let Some(value) = optional_setup_value(Some(icloud_username)) {
        config.icloud.username = Some(value);
    }
    let icloud_app_password = prompt_secret(
        "iCloud app-specific password",
        config.icloud.app_specific_password.is_some(),
    )?;

    validate_config(&config)
        .wrap_err_with(|| format!("validating config {}", config_path.display()))?;
    save_config(&config_path, &config)
        .wrap_err_with(|| format!("writing config {}", config_path.display()))?;
    if let Some(secret) = google_client_secret {
        store_google_client_secret(&mut config, &config_path, &secret).wrap_err_with(|| {
            format!("storing Google client secret for {}", config_path.display())
        })?;
    }
    if let Some(secret) = icloud_app_password {
        store_icloud_app_password(&mut config, &config_path, &secret).wrap_err_with(|| {
            format!("storing iCloud app password for {}", config_path.display())
        })?;
    }

    if prompt_yes_no("Run Google OAuth callback now?", false)? {
        setup_google_auth(
            Some(config_path.clone()),
            SetupGoogleAuthInput {
                print_url: false,
                callback: true,
                code: None,
                redirect_uri,
                state,
            },
        )
        .await?;
    }

    if prompt_yes_no("Discover calendars and add a pair now?", false)? {
        let (_, config) = load_validated_config(Some(config_path.clone()))?;
        let providers = live_providers(config, &config_path)?;
        let calendars = discover_calendars(&providers).await?;
        let google_calendars = calendars
            .iter()
            .filter(|calendar| calendar.provider == ProviderName::Google)
            .cloned()
            .collect::<Vec<_>>();
        let icloud_calendars = calendars
            .iter()
            .filter(|calendar| calendar.provider == ProviderName::Icloud)
            .cloned()
            .collect::<Vec<_>>();
        let google_calendar_id = prompt_calendar_choice("Google calendar", &google_calendars)?;
        let icloud_calendar_id = prompt_calendar_choice("iCloud calendar", &icloud_calendars)?;
        let pair_id = prompt_text("Sync pair ID", Some("personal"))?;
        setup_add_pair(
            Some(config_path.clone()),
            SetupPairInput {
                pair_id: Some(pair_id),
                google_calendar_id: Some(google_calendar_id),
                icloud_calendar_id: Some(icloud_calendar_id),
                direction: SetupDirection::TwoWay,
                enabled: true,
                force: true,
            },
        )?;
    }

    print_setup_doctor(&config_path)?;
    println!("next: insync sync --report .insync/reports/rust-dry-run.csv");
    Ok(())
}

fn prompt_text(label: &str, current: Option<&str>) -> Result<String> {
    let suffix = current
        .filter(|value| !value.is_empty())
        .map(|value| format!(" [{value}]"))
        .unwrap_or_default();
    print!("{label}{suffix}: ");
    io::stdout().flush()?;
    let mut line = String::new();
    io::stdin().read_line(&mut line)?;
    let value = line.trim();
    Ok(if value.is_empty() {
        current.unwrap_or_default().to_string()
    } else {
        value.to_string()
    })
}

fn prompt_secret(label: &str, has_existing: bool) -> Result<Option<String>> {
    let suffix = if has_existing {
        " [stored; leave blank to keep]"
    } else {
        " [leave blank to skip]"
    };
    let value = rpassword::prompt_password(format!("{label}{suffix}: "))?;
    Ok(optional_setup_value(Some(value)))
}

fn prompt_yes_no(label: &str, default: bool) -> Result<bool> {
    let suffix = if default { " [Y/n]" } else { " [y/N]" };
    print!("{label}{suffix}: ");
    io::stdout().flush()?;
    let mut line = String::new();
    io::stdin().read_line(&mut line)?;
    let value = line.trim().to_lowercase();
    if value.is_empty() {
        Ok(default)
    } else {
        Ok(matches!(value.as_str(), "y" | "yes" | "true" | "1"))
    }
}

fn prompt_calendar_choice(label: &str, calendars: &[DiscoveredCalendar]) -> Result<String> {
    if calendars.is_empty() {
        bail!("no {label} calendars discovered");
    }

    println!("{label}s:");
    for (index, calendar) in calendars.iter().enumerate() {
        println!(
            "  {index}: {}{} ({})",
            calendar.name,
            if calendar.writable {
                ""
            } else {
                " [read-only]"
            },
            calendar.id
        );
    }

    loop {
        let answer = prompt_text(&format!("{label} index"), Some("0"))?;
        if let Ok(index) = answer.parse::<usize>()
            && let Some(calendar) = calendars.get(index)
        {
            return Ok(calendar.id.clone());
        }
        println!("invalid selection");
    }
}

fn print_setup_doctor(config_path: &PathBuf) -> Result<()> {
    let config = load_config(config_path)
        .wrap_err_with(|| format!("loading config {}", config_path.display()))?;
    validate_config(&config)
        .wrap_err_with(|| format!("validating config {}", config_path.display()))?;
    let engine = SyncEngine::with_config_path(config, config_path);
    let summary = engine.doctor()?;
    println!("doctor:");
    println!("  db: {}", summary.db_path.display());
    println!("  configured pairs: {}", summary.configured_pair_count);
    println!("  enabled pairs: {}", summary.enabled_pair_count);
    println!(
        "  google credentials: {}",
        if summary.google_credentials_configured {
            "configured"
        } else {
            "missing"
        }
    );
    println!(
        "  icloud credentials: {}",
        if summary.icloud_credentials_configured {
            "configured"
        } else {
            "missing"
        }
    );
    Ok(())
}

#[derive(Debug, Clone)]
struct SetupGoogleAuthInput {
    print_url: bool,
    callback: bool,
    code: Option<String>,
    redirect_uri: String,
    state: String,
}

async fn setup_google_auth(
    explicit_config: Option<PathBuf>,
    input: SetupGoogleAuthInput,
) -> Result<()> {
    let auth_mode_count = [input.print_url, input.callback, input.code.is_some()]
        .into_iter()
        .filter(|active| *active)
        .count();
    if auth_mode_count != 1 {
        bail!("choose exactly one Google auth mode");
    }

    let config_path = resolve_config_path(explicit_config)?;
    let mut config = load_config(&config_path)
        .wrap_err_with(|| format!("loading config {}", config_path.display()))?;
    validate_config(&config)
        .wrap_err_with(|| format!("validating config {}", config_path.display()))?;
    let credentials = resolve_credentials(&mut config, &config_path)?;
    let client_id = credentials
        .google
        .client_id
        .clone()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| color_eyre::eyre::eyre!("google.clientId is required"))?;

    if input.print_url {
        println!(
            "{}",
            google_auth_url(&client_id, &input.redirect_uri, &input.state)
        );
        return Ok(());
    }

    let client_secret = credentials
        .google
        .client_secret
        .clone()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| color_eyre::eyre::eyre!("google client secret is required"))?;

    let code = if input.callback {
        wait_for_google_auth_code(&client_id, &input.redirect_uri, &input.state)?
    } else {
        required_setup_value(input.code, "--google-code")?
    };
    let token = exchange_google_auth_code(GoogleAuthCodeExchange {
        client_id,
        client_secret,
        redirect_uri: input.redirect_uri,
        code,
    })
    .await?;
    let refresh_token = token.refresh_token.ok_or_else(|| {
        color_eyre::eyre::eyre!(
            "Google did not return a refresh token; rerun the auth URL flow and grant consent again"
        )
    })?;
    store_google_refresh_token(&mut config, &config_path, &refresh_token)
        .wrap_err_with(|| format!("storing refresh token for {}", config_path.display()))?;

    println!("stored Google refresh token");
    println!("config: {}", config_path.display());
    Ok(())
}

fn wait_for_google_auth_code(client_id: &str, redirect_uri: &str, state: &str) -> Result<String> {
    let redirect = Url::parse(redirect_uri)
        .wrap_err_with(|| format!("parsing redirect URI {redirect_uri}"))?;
    if redirect.scheme() != "http" {
        bail!("Google callback redirect URI must use http");
    }
    let host = redirect
        .host_str()
        .ok_or_else(|| color_eyre::eyre::eyre!("redirect URI is missing a host"))?;
    if !matches!(host, "127.0.0.1" | "localhost" | "::1") {
        bail!("Google callback host must be localhost, 127.0.0.1, or ::1");
    }
    let port = redirect
        .port()
        .ok_or_else(|| color_eyre::eyre::eyre!("redirect URI must include a port"))?;
    let bind_host = if host == "localhost" {
        "127.0.0.1"
    } else {
        host
    };
    let listener = TcpListener::bind((bind_host, port))
        .wrap_err_with(|| format!("binding OAuth callback listener on {bind_host}:{port}"))?;
    let auth_url = google_auth_url(client_id, redirect_uri, state);

    println!("open this URL:");
    println!("{auth_url}");
    println!("waiting for Google OAuth callback on {redirect_uri}");

    let (mut stream, _) = listener.accept()?;
    handle_google_callback(&mut stream, &redirect, state)
}

fn handle_google_callback(
    stream: &mut TcpStream,
    redirect: &Url,
    expected_state: &str,
) -> Result<String> {
    let mut buffer = [0_u8; 8192];
    let count = stream.read(&mut buffer)?;
    let request = String::from_utf8_lossy(&buffer[..count]);
    let request_line = request
        .lines()
        .next()
        .ok_or_else(|| color_eyre::eyre::eyre!("empty OAuth callback request"))?;
    let path = request_line
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| color_eyre::eyre::eyre!("OAuth callback request is missing a path"))?;
    let callback_url = redirect
        .join(path)
        .wrap_err_with(|| format!("parsing OAuth callback path {path}"))?;

    if callback_url.path() != redirect.path() {
        write_callback_response(stream, 404, "Not Found", "Unexpected callback path")?;
        bail!("unexpected OAuth callback path: {}", callback_url.path());
    }

    let mut code = None;
    let mut state = None;
    let mut error = None;
    for (key, value) in callback_url.query_pairs() {
        match key.as_ref() {
            "code" => code = Some(value.into_owned()),
            "state" => state = Some(value.into_owned()),
            "error" => error = Some(value.into_owned()),
            _ => {}
        }
    }

    if state.as_deref() != Some(expected_state) {
        write_callback_response(stream, 400, "Bad Request", "Invalid OAuth state")?;
        bail!("invalid OAuth callback state");
    }
    if let Some(error) = error {
        write_callback_response(stream, 400, "Bad Request", "OAuth authorization failed")?;
        bail!("Google OAuth authorization failed: {error}");
    }
    let code =
        code.ok_or_else(|| color_eyre::eyre::eyre!("OAuth callback did not include a code"))?;
    write_callback_response(
        stream,
        200,
        "OK",
        "insync received the Google OAuth code. You can close this browser tab.",
    )?;

    Ok(code)
}

fn write_callback_response(
    stream: &mut TcpStream,
    status: u16,
    reason: &str,
    body: &str,
) -> Result<()> {
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes())?;
    stream.flush()?;
    Ok(())
}

#[derive(Debug, Clone)]
struct DiscoveredCalendar {
    provider: ProviderName,
    id: String,
    name: String,
    timezone: String,
    writable: bool,
}

async fn discover_calendars(providers: &LiveProviders) -> Result<Vec<DiscoveredCalendar>> {
    let mut rows = Vec::new();
    rows.extend(
        providers
            .google
            .list_calendars()
            .await?
            .into_iter()
            .map(|calendar| discovered_calendar(ProviderName::Google, calendar)),
    );
    rows.extend(
        providers
            .icloud
            .list_calendars()
            .await?
            .into_iter()
            .map(|calendar| discovered_calendar(ProviderName::Icloud, calendar)),
    );
    rows.sort_by(|left, right| {
        left.provider
            .to_string()
            .cmp(&right.provider.to_string())
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
            .then_with(|| left.id.cmp(&right.id))
    });

    Ok(rows)
}

fn discovered_calendar(provider: ProviderName, calendar: ProviderCalendar) -> DiscoveredCalendar {
    DiscoveredCalendar {
        provider,
        id: calendar.id,
        name: calendar.name,
        timezone: calendar.timezone.unwrap_or_default(),
        writable: calendar.writable,
    }
}

fn print_calendar_discovery(rows: &[DiscoveredCalendar]) {
    println!("provider\twritable\tname\ttimezone\tid");
    for row in rows {
        println!(
            "{}\t{}\t{}\t{}\t{}",
            row.provider,
            if row.writable { "yes" } else { "no" },
            row.name,
            row.timezone,
            row.id
        );
    }
}

fn write_calendar_discovery_csv(path: &PathBuf, rows: &[DiscoveredCalendar]) -> Result<()> {
    let lines = std::iter::once("provider,writable,name,timezone,id".to_string())
        .chain(rows.iter().map(|row| {
            [
                row.provider.to_string(),
                if row.writable { "yes" } else { "no" }.to_string(),
                row.name.clone(),
                row.timezone.clone(),
                row.id.clone(),
            ]
            .map(csv_escape)
            .join(",")
        }))
        .collect::<Vec<_>>();
    write_text(path, &format!("{}\n", lines.join("\n")))
}

fn runtime_snapshot_from_doctor(
    summary: &DoctorSummary,
    config: &insync_config::ServiceConfig,
) -> Result<AppRuntimeSnapshot> {
    Ok(AppRuntimeSnapshot {
        conflict_count: usize::try_from(summary.unresolved_conflict_count).unwrap_or(usize::MAX),
        last_run_at: summary.latest_run.as_ref().map(|run| {
            run.finished_at
                .clone()
                .unwrap_or_else(|| run.started_at.clone())
        }),
        last_run_status: summary.latest_run.as_ref().map(|run| run.status.clone()),
        next_run_at: next_run_at(summary, config.sync.poll_interval_seconds),
        recent_error: summary
            .latest_run
            .as_ref()
            .and_then(|run| run.error.clone()),
        pairs: pair_runtime_snapshots(&summary.db_path, config)?,
        runs: run_runtime_snapshots(&summary.db_path)?,
        report_rows: Vec::new(),
        conflict_summaries: conflict_summary_snapshots(&summary.db_path)?,
        conflict_details: conflict_detail_snapshots(&summary.db_path)?,
    })
}

fn pair_runtime_snapshots(
    db_path: &Path,
    config: &insync_config::ServiceConfig,
) -> Result<Vec<AppPairRuntimeSnapshot>> {
    let conn = open(db_path)?;
    migrate(&conn)?;
    let calendars = list_calendars(&conn)?;
    let calendars = calendars
        .into_iter()
        .map(|calendar| (calendar.id.clone(), calendar))
        .collect::<HashMap<_, _>>();

    Ok(config
        .sync
        .pairs
        .iter()
        .map(|pair| {
            let ids = configured_calendar_ids(config, pair);
            let google = calendars.get(&ids.google_calendar_id);
            let icloud = calendars.get(&ids.icloud_calendar_id);
            AppPairRuntimeSnapshot {
                pair_id: pair.id.clone(),
                google_calendar_name: calendar_name(google, &pair.google_calendar_id),
                icloud_calendar_name: calendar_name(icloud, &pair.icloud_calendar_id),
                google_account_label: google.map(|calendar| calendar.account_email.clone()),
                icloud_account_label: icloud.map(|calendar| calendar.account_email.clone()),
                google_last_sync_at: calendar_last_sync_at(google),
                icloud_last_sync_at: calendar_last_sync_at(icloud),
            }
        })
        .collect())
}

fn calendar_name(calendar: Option<&CalendarRow>, fallback_id: &str) -> Option<String> {
    calendar
        .and_then(|calendar| calendar.name.clone())
        .filter(|name| !name.trim().is_empty() && name != fallback_id)
}

fn calendar_last_sync_at(calendar: Option<&CalendarRow>) -> Option<String> {
    calendar.and_then(|calendar| {
        calendar
            .last_incremental_sync_at
            .clone()
            .or_else(|| calendar.last_full_sync_at.clone())
    })
}

fn run_runtime_snapshots(db_path: &Path) -> Result<Vec<AppRun>> {
    let conn = open(db_path)?;
    migrate(&conn)?;
    Ok(recent_sync_runs(&conn, 100)?
        .into_iter()
        .map(|run| AppRun {
            id: run.id,
            pair_id: run.sync_pair_id,
            status: sync_run_status_label(run.status).to_string(),
            started_at: run.started_at,
            finished_at: run.finished_at,
            error: run.error,
        })
        .collect())
}

fn conflict_summary_snapshots(db_path: &Path) -> Result<Vec<AppConflictSummary>> {
    let conn = open(db_path)?;
    migrate(&conn)?;
    Ok(list_unresolved_conflict_summaries(&conn)?
        .into_iter()
        .map(|row| AppConflictSummary {
            pair_id: row.sync_pair_id,
            reason: row.reason,
            count: usize::try_from(row.count).unwrap_or(usize::MAX),
            first_seen_at: row.first_seen_at,
            last_seen_at: row.last_seen_at,
        })
        .collect())
}

fn conflict_detail_snapshots(db_path: &Path) -> Result<Vec<AppConflictDetail>> {
    let conn = open(db_path)?;
    migrate(&conn)?;
    Ok(list_unresolved_conflicts(
        &conn,
        DbConflictFilter {
            limit: Some(200),
            ..DbConflictFilter::default()
        },
    )?
    .into_iter()
    .map(conflict_detail_snapshot)
    .collect())
}

fn conflict_detail_snapshot(row: UnresolvedConflictRow) -> AppConflictDetail {
    let google = row.google_snapshot.as_ref();
    let icloud = row.icloud_snapshot.as_ref();
    AppConflictDetail {
        id: row.id,
        pair_id: row.sync_pair_id,
        event_link_id: row.event_link_id,
        canonical_uid: row.canonical_uid,
        resolution_policy: conflict_resolution_policy(&row.reason),
        diff_fields: conflict_diff_fields(google, icloud),
        google_title: google.and_then(snapshot_title),
        icloud_title: icloud.and_then(snapshot_title),
        google_start: google.and_then(snapshot_start),
        icloud_start: icloud.and_then(snapshot_start),
        google_status: google.and_then(snapshot_status),
        icloud_status: icloud.and_then(snapshot_status),
        google_event_id: google.and_then(snapshot_google_event_id),
        icloud_href: icloud.and_then(snapshot_icloud_href),
        reason: row.reason,
        created_at: row.created_at,
    }
}

fn snapshot_title(value: &Value) -> Option<String> {
    non_empty_json_string(value.get("title"))
}

fn snapshot_status(value: &Value) -> Option<String> {
    non_empty_json_string(value.get("status"))
}

fn snapshot_start(value: &Value) -> Option<String> {
    snapshot_date(value.get("start")?)
}

fn snapshot_google_event_id(value: &Value) -> Option<String> {
    non_empty_json_string(value.pointer("/providerMeta/eventId"))
}

fn snapshot_icloud_href(value: &Value) -> Option<String> {
    non_empty_json_string(value.pointer("/providerMeta/href"))
}

fn snapshot_date(value: &Value) -> Option<String> {
    let kind = value.get("kind").and_then(Value::as_str);
    let raw_value = value.get("value").and_then(Value::as_str)?;
    match kind {
        Some("dateTime") => value
            .get("timezone")
            .and_then(Value::as_str)
            .filter(|timezone| !timezone.trim().is_empty())
            .map(|timezone| format!("{raw_value} [{timezone}]"))
            .or_else(|| Some(raw_value.to_string())),
        Some("date") | None => Some(raw_value.to_string()),
        Some(_) => Some(raw_value.to_string()),
    }
}

fn conflict_resolution_policy(reason: &str) -> String {
    match reason {
        "both_sides_changed" => "manual review; optional newest/google/icloud winner".to_string(),
        "delete_vs_update" => "default policy update_wins unless configured".to_string(),
        "unlinked_same_uid" => "manual review or configured UID policy".to_string(),
        "icloud_uid_exists" => "ignore known collision or choose manual".to_string(),
        _ => "manual review".to_string(),
    }
}

fn conflict_diff_fields(google: Option<&Value>, icloud: Option<&Value>) -> String {
    let Some(google) = google else {
        return "google_missing".to_string();
    };
    let Some(icloud) = icloud else {
        return "icloud_missing".to_string();
    };

    let fields = [
        ("title", snapshot_title(google), snapshot_title(icloud)),
        ("start", snapshot_start(google), snapshot_start(icloud)),
        ("status", snapshot_status(google), snapshot_status(icloud)),
        (
            "location",
            non_empty_json_string(google.get("location")),
            non_empty_json_string(icloud.get("location")),
        ),
        (
            "description",
            non_empty_json_string(google.get("description")),
            non_empty_json_string(icloud.get("description")),
        ),
    ];

    let diff = fields
        .into_iter()
        .filter_map(|(field, left, right)| (left != right).then_some(field))
        .collect::<Vec<_>>();
    if diff.is_empty() {
        "metadata".to_string()
    } else {
        diff.join("|")
    }
}

fn non_empty_json_string(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn sync_run_status_label(status: SyncRunStatus) -> &'static str {
    match status {
        SyncRunStatus::Running => "running",
        SyncRunStatus::Completed => "completed",
        SyncRunStatus::Failed => "failed",
    }
}

fn next_run_at(summary: &DoctorSummary, poll_interval_seconds: u64) -> Option<String> {
    let run = summary.latest_run.as_ref()?;
    let timestamp = run.finished_at.as_ref().unwrap_or(&run.started_at);
    let parsed = DateTime::parse_from_rfc3339(timestamp).ok()?;
    let interval = ChronoDuration::seconds(i64::try_from(poll_interval_seconds).ok()?);

    Some((parsed.with_timezone(&Utc) + interval).to_rfc3339())
}

fn print_pair_plan_summaries(rows: &[insync_engine::PairPlanSummary]) {
    if rows.is_empty() {
        return;
    }

    println!("pairs:");
    for row in rows {
        println!(
            "  {}: google_events={}, icloud_events={}, actions={}, action_counts={:?}, resolution_counts={:?}",
            row.pair_id,
            row.google_events,
            row.icloud_events,
            row.actions,
            row.action_counts,
            row.resolution_counts
        );
    }
}

fn print_conflict_summaries(rows: &[UnresolvedConflictSummary]) {
    println!("sync_pair_id\treason\tcount\tfirst_seen_at\tlast_seen_at");
    for row in rows {
        println!(
            "{}\t{}\t{}\t{}\t{}",
            row.sync_pair_id, row.reason, row.count, row.first_seen_at, row.last_seen_at
        );
    }
}

fn print_conflict_details(rows: &[UnresolvedConflictRow]) {
    println!("id\tsync_pair_id\tcanonical_uid\treason\tmanual_resolution\tcreated_at");
    for row in rows {
        println!(
            "{}\t{}\t{}\t{}\t{}\t{}",
            row.id,
            row.sync_pair_id,
            row.canonical_uid.as_deref().unwrap_or(""),
            row.reason,
            row.manual_resolution
                .map(|resolution| resolution.as_str())
                .unwrap_or(""),
            row.created_at
        );
    }
}

fn print_stale_conflict_cleanup(summary: &insync_engine::StaleConflictCleanupSummary) {
    println!("resolved stale conflicts: {}", summary.resolved_count);
    println!("db: {}", summary.db_path.display());
    if summary.pair_counts.is_empty() {
        return;
    }

    println!("pairs:");
    for (pair_id, count) in &summary.pair_counts {
        println!("  {pair_id}: {count}");
    }
}

fn write_conflict_summary_csv(path: &PathBuf, rows: &[UnresolvedConflictSummary]) -> Result<()> {
    let lines = std::iter::once("sync_pair_id,reason,count,first_seen_at,last_seen_at".to_string())
        .chain(rows.iter().map(|row| {
            [
                row.sync_pair_id.clone(),
                row.reason.clone(),
                row.count.to_string(),
                row.first_seen_at.clone(),
                row.last_seen_at.clone(),
            ]
            .map(csv_escape)
            .join(",")
        }))
        .collect::<Vec<_>>();
    write_text(path, &format!("{}\n", lines.join("\n")))
}

fn write_conflict_details_csv(path: &PathBuf, rows: &[UnresolvedConflictRow]) -> Result<()> {
    let lines = std::iter::once(
        "id,sync_pair_id,canonical_uid,reason,manual_resolution,resolution_requested_at,created_at"
            .to_string(),
    )
    .chain(rows.iter().map(|row| {
        [
            row.id.clone(),
            row.sync_pair_id.clone(),
            row.canonical_uid.clone().unwrap_or_default(),
            row.reason.clone(),
            row.manual_resolution
                .map(|resolution| resolution.as_str().to_string())
                .unwrap_or_default(),
            row.resolution_requested_at.clone().unwrap_or_default(),
            row.created_at.clone(),
        ]
        .map(csv_escape)
        .join(",")
    }))
    .collect::<Vec<_>>();
    write_text(path, &format!("{}\n", lines.join("\n")))
}

fn write_text(path: &PathBuf, body: &str) -> Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }

    fs::write(path, body)?;
    Ok(())
}

fn csv_escape(value: String) -> String {
    if value.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value
    }
}

#[derive(Debug)]
struct LiveProviders {
    google: GoogleCalendarProvider,
    icloud: IcloudCalDavProvider,
}

fn live_providers(
    mut config: insync_config::ServiceConfig,
    config_path: &PathBuf,
) -> Result<LiveProviders> {
    let credentials = resolve_credentials(&mut config, config_path)?;
    Ok(LiveProviders {
        google: GoogleCalendarProvider::new(GoogleProviderOptions {
            client_id: credentials.google.client_id,
            client_secret: credentials.google.client_secret,
            refresh_token: credentials.google.refresh_token,
            ..GoogleProviderOptions::default()
        }),
        icloud: IcloudCalDavProvider::new(IcloudProviderOptions {
            username: credentials.icloud.username,
            app_specific_password: credentials.icloud.app_specific_password,
            server_url: Some(credentials.icloud.caldav_url),
        }),
    })
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FixtureFile {
    #[serde(default)]
    google_calendar_id: Option<String>,
    #[serde(default)]
    icloud_calendar_id: Option<String>,
    #[serde(default)]
    google: Vec<GoogleEvent>,
    #[serde(default)]
    icloud: Vec<FixtureIcloudObject>,
}

#[derive(Debug, Clone, Deserialize)]
struct FixtureIcloudObject {
    url: String,
    #[serde(default)]
    etag: Option<String>,
    #[serde(default)]
    data: Option<String>,
}

#[derive(Debug, Clone)]
struct FixtureProviders {
    google: FixtureProvider,
    icloud: FixtureProvider,
}

#[derive(Debug, Clone)]
struct FixtureProvider {
    name: ProviderName,
    events: Vec<CanonicalEvent>,
}

fn fixture_providers(path: &PathBuf) -> Result<FixtureProviders> {
    let body = fs::read_to_string(path)
        .wrap_err_with(|| format!("reading fixture file {}", path.display()))?;
    let fixture: FixtureFile = serde_json::from_str(&body)
        .wrap_err_with(|| format!("parsing fixture file {}", path.display()))?;
    let google_calendar_id = fixture.google_calendar_id.as_deref().unwrap_or("primary");
    let icloud_calendar_id = fixture.icloud_calendar_id.as_deref().unwrap_or("icloud");

    let google_events = fixture
        .google
        .into_iter()
        .map(|event| google_to_canonical(google_calendar_id, event))
        .collect::<Result<Vec<_>, _>>()?;
    let mut icloud_events = Vec::new();
    for object in fixture.icloud {
        icloud_events.extend(ical_object_to_canonical(
            icloud_calendar_id,
            CalendarObject {
                url: object.url,
                etag: object.etag,
                data: object.data,
            },
        )?);
    }

    Ok(FixtureProviders {
        google: FixtureProvider {
            name: ProviderName::Google,
            events: google_events,
        },
        icloud: FixtureProvider {
            name: ProviderName::Icloud,
            events: icloud_events,
        },
    })
}

#[async_trait::async_trait]
impl CalendarProvider for FixtureProvider {
    fn name(&self) -> ProviderName {
        self.name
    }

    async fn list_calendars(&self) -> std::result::Result<Vec<ProviderCalendar>, ProviderError> {
        Ok(Vec::new())
    }

    async fn get_changes(
        &self,
        calendar_id: &str,
        _cursor: ProviderSyncCursor,
    ) -> std::result::Result<ProviderChangeSet, ProviderError> {
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
    ) -> std::result::Result<ProviderEventMeta, ProviderError> {
        Ok(fixture_meta(self.name, calendar_id, event))
    }

    async fn update_event(
        &self,
        calendar_id: &str,
        _remote_event_id: &str,
        event: &CanonicalEvent,
        _etag: Option<&str>,
    ) -> std::result::Result<ProviderEventMeta, ProviderError> {
        Ok(fixture_meta(self.name, calendar_id, event))
    }

    async fn delete_event(
        &self,
        _calendar_id: &str,
        _remote_event_id: &str,
        _etag: Option<&str>,
    ) -> std::result::Result<(), ProviderError> {
        Ok(())
    }
}

fn fixture_meta(
    provider: ProviderName,
    calendar_id: &str,
    event: &CanonicalEvent,
) -> ProviderEventMeta {
    ProviderEventMeta {
        provider,
        calendar_id: calendar_id.to_string(),
        event_id: Some(event.canonical_uid.clone()),
        href: None,
        etag: None,
        ical_uid: Some(event.canonical_uid.clone()),
        updated_at: None,
        deleted: false,
    }
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(io::stderr)
        .init();
}

fn run_tui(mut model: AppModel) -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = loop {
        terminal.draw(|frame| draw_tui(frame, &model))?;

        if event::poll(Duration::from_millis(200))?
            && let Event::Key(key) = event::read()?
        {
            if model.command_palette_open {
                match key.code {
                    KeyCode::Esc => {
                        model.update(AppEvent::CloseCommandPalette);
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        model.update(AppEvent::SelectNextCommand);
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        model.update(AppEvent::SelectPreviousCommand);
                    }
                    KeyCode::Enter => {
                        let effects = model.update(AppEvent::ExecuteSelectedCommand);
                        if apply_tui_effects(&mut model, effects) {
                            break Ok(());
                        }
                    }
                    _ => {}
                }
            } else {
                match key.code {
                    KeyCode::Char('q') => break Ok(()),
                    KeyCode::Char(':') => {
                        model.update(AppEvent::OpenCommandPalette);
                    }
                    KeyCode::Down | KeyCode::Char('j') => match model.view {
                        AppView::Dashboard | AppView::Setup => model.select_next_pair(),
                        AppView::Runs => {
                            model.update(AppEvent::SelectNextRun);
                        }
                        AppView::Reports => {
                            model.update(AppEvent::SelectNextReportRow);
                        }
                        AppView::Conflicts => {
                            model.update(AppEvent::SelectNextConflict);
                        }
                    },
                    KeyCode::Up | KeyCode::Char('k') => match model.view {
                        AppView::Dashboard | AppView::Setup => model.select_previous_pair(),
                        AppView::Runs => {
                            model.update(AppEvent::SelectPreviousRun);
                        }
                        AppView::Reports => {
                            model.update(AppEvent::SelectPreviousReportRow);
                        }
                        AppView::Conflicts => {
                            model.update(AppEvent::SelectPreviousConflict);
                        }
                    },
                    KeyCode::Esc | KeyCode::Char('p') => {
                        model.update(AppEvent::ShowDashboard);
                    }
                    KeyCode::Char('l') => {
                        model.update(AppEvent::ShowRuns);
                    }
                    KeyCode::Char('v') => {
                        model.update(AppEvent::ShowReports);
                    }
                    KeyCode::Char('c') => {
                        model.update(AppEvent::ShowConflicts);
                    }
                    KeyCode::Char('f') if model.view == AppView::Runs => {
                        model.update(AppEvent::CycleRunFilter);
                    }
                    KeyCode::Char('f') if model.view == AppView::Reports => {
                        model.update(AppEvent::CycleReportFilter);
                    }
                    KeyCode::Char('t') if model.view == AppView::Reports => {
                        model.update(AppEvent::CycleReportSort);
                    }
                    KeyCode::Char('d') => {
                        let effects = model.update(AppEvent::ExecuteCommand(AppCommand::DryRun));
                        if apply_tui_effects(&mut model, effects) {
                            break Ok(());
                        }
                    }
                    KeyCode::Char('a') => {
                        let effects = model.update(AppEvent::ExecuteCommand(AppCommand::ApplyRun));
                        if apply_tui_effects(&mut model, effects) {
                            break Ok(());
                        }
                    }
                    KeyCode::Char('r') => {
                        let effects =
                            model.update(AppEvent::ExecuteCommand(AppCommand::RefreshConflicts));
                        if apply_tui_effects(&mut model, effects) {
                            break Ok(());
                        }
                    }
                    KeyCode::Char('s') => {
                        let effects = model.update(AppEvent::ExecuteCommand(AppCommand::OpenSetup));
                        if apply_tui_effects(&mut model, effects) {
                            break Ok(());
                        }
                    }
                    KeyCode::Char('b') => {
                        let effects = model
                            .update(AppEvent::ExecuteCommand(AppCommand::ToggleBackgroundPause));
                        if apply_tui_effects(&mut model, effects) {
                            break Ok(());
                        }
                    }
                    _ => {}
                }
            }
        }

        if model.status == AppStatus::Error {
            break Ok(());
        }
    };

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

fn apply_tui_effects(model: &mut AppModel, effects: Vec<AppEffect>) -> bool {
    for effect in effects {
        match effect {
            AppEffect::RunDrySync => {
                model.update(AppEvent::EngineFinished {
                    message: "dry-run requested".to_string(),
                });
            }
            AppEffect::RunApplySync => {
                model.update(AppEvent::EngineFinished {
                    message: "apply requested".to_string(),
                });
            }
            AppEffect::LoadConflicts => {
                model.update(AppEvent::EngineFinished {
                    message: "conflict refresh requested".to_string(),
                });
            }
            AppEffect::ShowSetup => {
                model.update(AppEvent::EngineFinished {
                    message: "setup requested".to_string(),
                });
            }
            AppEffect::ExportDryRunReport => {
                model.update(AppEvent::EngineFinished {
                    message: "report export requested".to_string(),
                });
            }
            AppEffect::StartBackgroundScheduler => {
                model.update(AppEvent::EngineFinished {
                    message: "background scheduler start requested".to_string(),
                });
            }
            AppEffect::StopBackgroundScheduler => {
                model.update(AppEvent::EngineFinished {
                    message: "background scheduler stop requested".to_string(),
                });
            }
            AppEffect::Quit => return true,
        }
    }

    false
}

fn draw_tui(frame: &mut Frame<'_>, model: &AppModel) {
    let area = frame.area();
    let has_notifications = !model.shell_snapshot().notifications.is_empty();
    let constraints = if has_notifications {
        vec![
            Constraint::Length(4),
            Constraint::Length(7),
            Constraint::Min(10),
            Constraint::Length(5),
            Constraint::Length(4),
        ]
    } else {
        vec![
            Constraint::Length(4),
            Constraint::Length(7),
            Constraint::Min(12),
            Constraint::Length(4),
        ]
    };
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(area);

    render_header(frame, vertical[0], model);
    render_metrics(frame, vertical[1], model);

    match model.view {
        AppView::Dashboard => {
            if area.width < 100 {
                let body = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Length(5), Constraint::Min(7)])
                    .split(vertical[2]);
                render_pair_table(frame, body[0], model);
                render_side_panel(frame, body[1], model);
            } else {
                let body = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([Constraint::Percentage(58), Constraint::Percentage(42)])
                    .split(vertical[2]);
                render_pair_table(frame, body[0], model);
                render_side_panel(frame, body[1], model);
            }
        }
        AppView::Setup => render_setup_screen(frame, vertical[2], model),
        AppView::Runs => render_runs_screen(frame, vertical[2], model),
        AppView::Reports => render_reports_screen(frame, vertical[2], model),
        AppView::Conflicts => render_conflicts_screen(frame, vertical[2], model),
    }
    if has_notifications {
        render_notifications(frame, vertical[3], model);
        render_command_bar(frame, vertical[4], model);
    } else {
        render_command_bar(frame, vertical[3], model);
    }
    if model.command_palette_open {
        render_command_palette(frame, area, model);
    }
}

fn render_header(frame: &mut Frame<'_>, area: Rect, model: &AppModel) {
    let title = Line::from(vec![
        Span::styled(
            "insync",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  iCloud <-> Google Calendar"),
    ]);
    let subtitle = Line::from(vec![
        Span::raw("Status "),
        Span::styled(
            status_label(model.status),
            Style::default().fg(status_color(model.status)),
        ),
        Span::raw("  View "),
        Span::styled(
            view_label(model.view),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  Filter "),
        Span::styled(
            model.run_filter.label(),
            Style::default().fg(match model.view {
                AppView::Runs => color_warning(),
                AppView::Reports => color_muted(),
                _ => color_muted(),
            }),
        ),
        Span::raw("  Report "),
        Span::styled(
            format!(
                "{}/{}",
                model.report_filter.label(),
                model.report_sort.label()
            ),
            Style::default().fg(if model.view == AppView::Reports {
                color_warning()
            } else {
                color_muted()
            }),
        ),
        Span::raw("  Selected "),
        Span::styled(
            model.selected_pair_id.as_deref().unwrap_or("-"),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
    ]);

    frame.render_widget(
        Paragraph::new(vec![title, subtitle])
            .block(chrome_block("Dashboard"))
            .alignment(Alignment::Left),
        area,
    );
}

fn render_metrics(frame: &mut Frame<'_>, area: Rect, model: &AppModel) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(25),
            Constraint::Percentage(25),
            Constraint::Percentage(25),
            Constraint::Percentage(25),
        ])
        .split(area);
    let enabled_ratio = if model.pairs.is_empty() {
        0.0
    } else {
        model.enabled_pair_count() as f64 / model.pairs.len() as f64
    };

    render_metric(
        frame,
        chunks[0],
        "Sync",
        status_label(model.status),
        status_color(model.status),
    );
    frame.render_widget(
        Gauge::default()
            .block(chrome_block("Enabled Pairs"))
            .gauge_style(Style::default().fg(pair_gauge_color(model)))
            .ratio(enabled_ratio)
            .label(format!(
                "{}/{}",
                model.enabled_pair_count(),
                model.pairs.len()
            )),
        chunks[1],
    );
    render_metric(
        frame,
        chunks[2],
        "Conflicts",
        &model.conflict_count.to_string(),
        if model.conflict_count == 0 {
            color_success()
        } else {
            color_warning()
        },
    );
    render_metric(
        frame,
        chunks[3],
        "Last Run",
        last_run_label(model).as_str(),
        match model.last_run_status.as_deref() {
            Some("failed") => color_danger(),
            Some("completed") => color_success(),
            Some("running") => color_running(),
            _ => color_neutral(),
        },
    );
}

fn render_metric(frame: &mut Frame<'_>, area: Rect, title: &str, value: &str, color: Color) {
    frame.render_widget(
        Paragraph::new(value)
            .style(Style::default().fg(color).add_modifier(Modifier::BOLD))
            .block(chrome_block(title))
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true }),
        area,
    );
}

fn render_pair_table(frame: &mut Frame<'_>, area: Rect, model: &AppModel) {
    if model.pairs.is_empty() {
        render_empty_state(
            frame,
            area,
            "Calendar Pairs",
            &[
                "No calendar pairs configured.",
                "Run setup or press s to start the guided setup flow.",
            ],
            color_warning(),
        );
        return;
    }

    let compact = area.width < 95;
    let rows = model
        .pairs
        .iter()
        .map(|pair| pair_row(pair, model, compact));

    let table = if compact {
        Table::new(
            rows,
            [
                Constraint::Length(1),
                Constraint::Min(16),
                Constraint::Length(6),
                Constraint::Length(16),
            ],
        )
        .header(
            Row::new(["", "Pair", "State", "Direction"]).style(
                Style::default()
                    .fg(color_muted())
                    .add_modifier(Modifier::BOLD),
            ),
        )
        .block(chrome_block("Calendar Pairs"))
    } else {
        Table::new(
            rows,
            [
                Constraint::Length(1),
                Constraint::Length(16),
                Constraint::Length(6),
                Constraint::Length(16),
                Constraint::Percentage(26),
                Constraint::Percentage(26),
            ],
        )
        .header(
            Row::new(["", "Pair", "State", "Direction", "Google", "iCloud"]).style(
                Style::default()
                    .fg(color_muted())
                    .add_modifier(Modifier::BOLD),
            ),
        )
        .block(chrome_block("Calendar Pairs"))
    };

    frame.render_widget(table, area);
}

fn render_side_panel(frame: &mut Frame<'_>, area: Rect, model: &AppModel) {
    if area.height < 10 {
        render_selected_pair(frame, area, model);
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(54), Constraint::Percentage(46)])
        .split(area);
    render_selected_pair(frame, chunks[0], model);
    render_activity(frame, chunks[1], model);
}

fn pair_row(pair: &insync_app::AppPair, model: &AppModel, compact: bool) -> Row<'static> {
    let selected = model.selected_pair_id.as_deref() == Some(pair.id.as_str());
    let marker = if selected { ">" } else { " " };
    let enabled = if pair.enabled { "on" } else { "off" };
    let style = if selected {
        Style::default()
            .fg(color_running())
            .add_modifier(Modifier::BOLD)
    } else if pair.enabled {
        Style::default().fg(Color::White)
    } else {
        Style::default().fg(color_muted())
    };

    let cells = if compact {
        vec![
            marker.to_string(),
            pair.id.clone(),
            enabled.to_string(),
            direction_label(pair.direction).to_string(),
        ]
    } else {
        vec![
            marker.to_string(),
            pair.id.clone(),
            enabled.to_string(),
            direction_label(pair.direction).to_string(),
            compact_calendar_label(
                pair.google_calendar_name.as_deref(),
                pair.google_account_label.as_deref(),
                &pair.google_calendar_id,
            ),
            compact_calendar_label(
                pair.icloud_calendar_name.as_deref(),
                pair.icloud_account_label.as_deref(),
                &pair.icloud_calendar_id,
            ),
        ]
    };

    Row::new(cells).style(style)
}

fn render_selected_pair(frame: &mut Frame<'_>, area: Rect, model: &AppModel) {
    let lines = if let Some(pair) = model.selected_pair() {
        if area.height < 9 {
            vec![
                Line::from(vec![Span::styled(
                    &pair.id,
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                )]),
                Line::from(format!(
                    "{} / {}",
                    if pair.enabled { "enabled" } else { "disabled" },
                    direction_label(pair.direction)
                )),
                Line::from(format!(
                    "Google: {}",
                    compact_detail_value(
                        &calendar_detail_label(
                            pair.google_calendar_name.as_deref(),
                            pair.google_account_label.as_deref(),
                            &pair.google_calendar_id,
                        ),
                        area.width
                    )
                )),
                Line::from(format!(
                    "iCloud: {}",
                    compact_detail_value(
                        &calendar_detail_label(
                            pair.icloud_calendar_name.as_deref(),
                            pair.icloud_account_label.as_deref(),
                            &pair.icloud_calendar_id,
                        ),
                        area.width
                    )
                )),
                Line::from(format!(
                    "Sync: G {} / I {}",
                    pair.google_last_sync_at.as_deref().unwrap_or("never"),
                    pair.icloud_last_sync_at.as_deref().unwrap_or("never")
                )),
            ]
        } else {
            vec![
                Line::from(vec![Span::styled(
                    &pair.id,
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                )]),
                Line::from(format!(
                    "State: {}",
                    if pair.enabled { "enabled" } else { "disabled" }
                )),
                Line::from(format!("Direction: {}", direction_label(pair.direction))),
                Line::from(format!(
                    "Google: {}",
                    compact_detail_value(
                        &calendar_detail_label(
                            pair.google_calendar_name.as_deref(),
                            pair.google_account_label.as_deref(),
                            &pair.google_calendar_id,
                        ),
                        area.width
                    )
                )),
                Line::from(format!(
                    "Google ID: {}",
                    compact_detail_value(&pair.google_calendar_id, area.width)
                )),
                Line::from(format!(
                    "Google sync: {}",
                    pair.google_last_sync_at.as_deref().unwrap_or("never")
                )),
                Line::from(format!(
                    "iCloud: {}",
                    compact_detail_value(
                        &calendar_detail_label(
                            pair.icloud_calendar_name.as_deref(),
                            pair.icloud_account_label.as_deref(),
                            &pair.icloud_calendar_id,
                        ),
                        area.width
                    )
                )),
                Line::from(format!(
                    "iCloud ID: {}",
                    compact_detail_value(&pair.icloud_calendar_id, area.width)
                )),
                Line::from(format!(
                    "iCloud sync: {}",
                    pair.icloud_last_sync_at.as_deref().unwrap_or("never")
                )),
            ]
        }
    } else {
        render_empty_state(
            frame,
            area,
            "Selected Pair",
            &[
                "No calendar pair selected.",
                "Run setup or press s to add calendars.",
            ],
            color_warning(),
        );
        return;
    };

    frame.render_widget(
        Paragraph::new(lines)
            .block(chrome_block("Selected Pair"))
            .wrap(Wrap { trim: true }),
        area,
    );
}

fn render_activity(frame: &mut Frame<'_>, area: Rect, model: &AppModel) {
    let items = vec![
        activity_item(
            format!("Status: {}", status_label(model.status)),
            status_color(model.status),
        ),
        activity_item(
            format!("Unresolved conflicts: {}", model.conflict_count),
            if model.conflict_count == 0 {
                color_success()
            } else {
                color_warning()
            },
        ),
        activity_item(
            format!("Last run: {}", last_run_label(model)),
            last_run_color(model),
        ),
        activity_item(
            format!("Next run: {}", next_run_label(model)),
            if model.next_run_at.is_some() {
                color_neutral()
            } else {
                color_muted()
            },
        ),
        activity_item(
            format!(
                "Recent error: {}",
                model.recent_error.as_deref().unwrap_or("-")
            ),
            if model.recent_error.is_some() {
                color_danger()
            } else {
                color_muted()
            },
        ),
        activity_item(
            format!(
                "App message: {}",
                model.last_message.as_deref().unwrap_or("-")
            ),
            if model.last_message.is_some() {
                color_neutral()
            } else {
                color_muted()
            },
        ),
    ];

    frame.render_widget(List::new(items).block(chrome_block("Activity")), area);
}

fn render_setup_screen(frame: &mut Frame<'_>, area: Rect, model: &AppModel) {
    if area.width < 100 {
        let body = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(58), Constraint::Percentage(42)])
            .split(area);
        render_setup_steps(frame, body[0], model);
        render_setup_guidance(frame, body[1], model);
    } else {
        let body = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(58), Constraint::Percentage(42)])
            .split(area);
        render_setup_steps(frame, body[0], model);
        render_setup_guidance(frame, body[1], model);
    }
}

fn render_setup_steps(frame: &mut Frame<'_>, area: Rect, model: &AppModel) {
    let steps = model.setup_steps();
    let items = steps
        .iter()
        .map(|step| {
            let label = setup_status_label(step.status);
            let color = setup_status_color(step.status);
            ListItem::new(Line::from(vec![
                Span::styled(
                    format!("{label} "),
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    compact_string(&step.label, 20),
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("  "),
                Span::styled(compact_string(&step.detail, 82), Style::default().fg(color)),
            ]))
        })
        .collect::<Vec<_>>();

    let title = format!(
        "Setup Wizard ({}/{})",
        model.setup_ready_count(),
        steps.len()
    );
    frame.render_widget(List::new(items).block(chrome_block(&title)), area);
}

fn render_setup_guidance(frame: &mut Frame<'_>, area: Rect, model: &AppModel) {
    let next_step = model
        .setup_steps()
        .into_iter()
        .find(|step| step.status != AppSetupStepStatus::Complete);
    let pair = model.selected_pair();
    let mut lines = Vec::new();

    if let Some(step) = next_step {
        lines.extend(setup_step_lines("Next", &step, area.width));
    } else {
        lines.push(Line::from(vec![Span::styled(
            "Ready for repeated dry-runs.",
            Style::default()
                .fg(color_success())
                .add_modifier(Modifier::BOLD),
        )]));
        lines.push(Line::from(
            "Run insync sync --report .insync/reports/dry-run.csv",
        ));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(vec![Span::styled(
        "Selected Pair",
        Style::default()
            .fg(color_checking())
            .add_modifier(Modifier::BOLD),
    )]));
    if let Some(pair) = pair {
        lines.push(Line::from(format!(
            "{} / {} / {}",
            pair.id,
            if pair.enabled { "enabled" } else { "disabled" },
            direction_label(pair.direction)
        )));
        lines.push(Line::from(format!(
            "Google: {}",
            compact_detail_value(
                &calendar_detail_label(
                    pair.google_calendar_name.as_deref(),
                    pair.google_account_label.as_deref(),
                    &pair.google_calendar_id,
                ),
                area.width
            )
        )));
        lines.push(Line::from(format!(
            "iCloud: {}",
            compact_detail_value(
                &calendar_detail_label(
                    pair.icloud_calendar_name.as_deref(),
                    pair.icloud_account_label.as_deref(),
                    &pair.icloud_calendar_id,
                ),
                area.width
            )
        )));
    } else {
        lines.push(Line::from("No pair selected."));
        lines.push(Line::from("Use setup pair commands after discovery."));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(format!(
        "Config: db {} / secrets {} / poll {}s",
        compact_string(&model.setup.db_path, 26),
        model.setup.secret_store,
        model.setup.poll_interval_seconds
    )));

    frame.render_widget(
        Paragraph::new(lines)
            .block(chrome_block("Setup Guidance"))
            .wrap(Wrap { trim: true }),
        area,
    );
}

fn setup_step_lines(prefix: &str, step: &AppSetupStep, area_width: u16) -> Vec<Line<'static>> {
    vec![
        Line::from(vec![
            Span::styled(
                format!("{prefix}: "),
                Style::default()
                    .fg(setup_status_color(step.status))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                step.label.clone(),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(format!("Status: {}", setup_status_label(step.status))),
        Line::from(format!(
            "Detail: {}",
            compact_detail_value(&step.detail, area_width)
        )),
        Line::from(format!(
            "Action: {}",
            compact_detail_value(&step.next_action, area_width)
        )),
    ]
}

fn render_notifications(frame: &mut Frame<'_>, area: Rect, model: &AppModel) {
    let shell = model.shell_snapshot();
    let items = shell
        .notifications
        .iter()
        .take(3)
        .map(|notification| {
            let severity = notification.severity;
            let message_width = usize::from(area.width.saturating_sub(16)).clamp(18, 96);
            ListItem::new(Line::from(vec![
                Span::styled(
                    notification_label(severity),
                    Style::default()
                        .fg(notification_color(severity))
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" "),
                Span::styled(
                    compact_string(&notification.message, message_width),
                    Style::default().fg(Color::White),
                ),
            ]))
        })
        .collect::<Vec<_>>();

    let title = if shell.notifications.len() > 3 {
        format!("Notifications (+{})", shell.notifications.len() - 3)
    } else {
        "Notifications".to_string()
    };

    frame.render_widget(List::new(items).block(chrome_block(&title)), area);
}

fn render_runs_screen(frame: &mut Frame<'_>, area: Rect, model: &AppModel) {
    if area.width < 100 {
        let body = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(58), Constraint::Percentage(42)])
            .split(area);
        render_runs_table(frame, body[0], model);
        render_run_detail(frame, body[1], model);
    } else {
        let body = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(64), Constraint::Percentage(36)])
            .split(area);
        render_runs_table(frame, body[0], model);
        render_run_detail(frame, body[1], model);
    }
}

fn render_runs_table(frame: &mut Frame<'_>, area: Rect, model: &AppModel) {
    let visible_runs = model.visible_runs();
    if visible_runs.is_empty() {
        let lines: &[&str] = if model.runs.is_empty() {
            &[
                "No sync runs recorded yet.",
                "Run a dry-run or apply sync to populate this history.",
            ]
        } else {
            &[
                "No sync runs match this filter.",
                "Press f to cycle through failed, running, completed, and all.",
            ]
        };
        render_empty_state(frame, area, "Sync Runs", lines, color_warning());
        return;
    }

    let compact = area.width < 88;
    let rows = visible_runs.iter().map(|run| run_row(run, model, compact));
    let table = if compact {
        Table::new(
            rows,
            [
                Constraint::Length(1),
                Constraint::Length(10),
                Constraint::Min(16),
                Constraint::Length(18),
            ],
        )
        .header(
            Row::new(["", "Status", "Pair", "Started"]).style(
                Style::default()
                    .fg(color_muted())
                    .add_modifier(Modifier::BOLD),
            ),
        )
    } else {
        Table::new(
            rows,
            [
                Constraint::Length(1),
                Constraint::Length(10),
                Constraint::Length(18),
                Constraint::Percentage(26),
                Constraint::Percentage(26),
                Constraint::Percentage(20),
            ],
        )
        .header(
            Row::new(["", "Status", "Pair", "Started", "Finished", "Error"]).style(
                Style::default()
                    .fg(color_muted())
                    .add_modifier(Modifier::BOLD),
            ),
        )
    };

    let title = format!("Sync Runs ({}/{})", visible_runs.len(), model.runs.len());
    frame.render_widget(table.block(chrome_block(&title)), area);
}

fn run_row(run: &AppRun, model: &AppModel, compact: bool) -> Row<'static> {
    let selected = model.selected_run_id.as_deref() == Some(run.id.as_str());
    let marker = if selected { ">" } else { " " };
    let style = if selected {
        Style::default()
            .fg(color_running())
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(run_status_color(&run.status))
    };
    let error = run.error.as_deref().unwrap_or("-");
    let cells = if compact {
        vec![
            marker.to_string(),
            run.status.clone(),
            run.pair_id.as_deref().unwrap_or("-").to_string(),
            compact_string(&run.started_at, 18),
        ]
    } else {
        vec![
            marker.to_string(),
            run.status.clone(),
            run.pair_id.as_deref().unwrap_or("-").to_string(),
            compact_string(&run.started_at, 26),
            compact_string(run.finished_at.as_deref().unwrap_or("-"), 26),
            compact_string(error, 34),
        ]
    };

    Row::new(cells).style(style)
}

fn render_run_detail(frame: &mut Frame<'_>, area: Rect, model: &AppModel) {
    let lines = if let Some(run) = model.selected_run() {
        if area.height < 9 {
            vec![
                Line::from(vec![Span::styled(
                    &run.status,
                    Style::default()
                        .fg(run_status_color(&run.status))
                        .add_modifier(Modifier::BOLD),
                )]),
                Line::from(format!(
                    "Run: {}",
                    compact_detail_value(&run.id, area.width)
                )),
                Line::from(format!("Pair: {}", run.pair_id.as_deref().unwrap_or("-"))),
                Line::from(format!(
                    "At: {} -> {}",
                    compact_string(&run.started_at, 24),
                    compact_string(run.finished_at.as_deref().unwrap_or("-"), 24)
                )),
                Line::from(format!(
                    "Error: {}",
                    compact_detail_value(run.error.as_deref().unwrap_or("-"), area.width)
                )),
            ]
        } else {
            vec![
                Line::from(vec![Span::styled(
                    &run.status,
                    Style::default()
                        .fg(run_status_color(&run.status))
                        .add_modifier(Modifier::BOLD),
                )]),
                Line::from(format!(
                    "Run ID: {}",
                    compact_detail_value(&run.id, area.width)
                )),
                Line::from(format!("Pair: {}", run.pair_id.as_deref().unwrap_or("-"))),
                Line::from(format!("Started: {}", run.started_at)),
                Line::from(format!(
                    "Finished: {}",
                    run.finished_at.as_deref().unwrap_or("-")
                )),
                Line::from(format!(
                    "Error: {}",
                    compact_detail_value(run.error.as_deref().unwrap_or("-"), area.width)
                )),
            ]
        }
    } else {
        render_empty_state(
            frame,
            area,
            "Run Detail",
            &[
                "No run selected.",
                "Use f to change the filter or run insync sync.",
            ],
            color_warning(),
        );
        return;
    };

    frame.render_widget(
        Paragraph::new(lines)
            .block(chrome_block("Run Detail"))
            .wrap(Wrap { trim: true }),
        area,
    );
}

fn render_reports_screen(frame: &mut Frame<'_>, area: Rect, model: &AppModel) {
    if area.width < 100 {
        let body = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(58), Constraint::Percentage(42)])
            .split(area);
        render_report_table(frame, body[0], model);
        render_report_detail(frame, body[1], model);
    } else {
        let body = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(66), Constraint::Percentage(34)])
            .split(area);
        render_report_table(frame, body[0], model);
        render_report_detail(frame, body[1], model);
    }
}

fn render_report_table(frame: &mut Frame<'_>, area: Rect, model: &AppModel) {
    let visible_rows = model.visible_report_rows();
    if visible_rows.is_empty() {
        let lines: &[&str] = if model.report_rows.is_empty() {
            &[
                "No dry-run report rows loaded.",
                "Run a dry-run with reporting to populate this view.",
            ]
        } else {
            &[
                "No report rows match this filter.",
                "Press f to cycle action categories.",
            ]
        };
        render_empty_state(frame, area, "Dry-Run Report", lines, color_warning());
        return;
    }

    let compact = area.width < 96;
    let rows = visible_rows
        .iter()
        .enumerate()
        .map(|(index, row)| report_row(index, row, model, compact));
    let table = if compact {
        Table::new(
            rows,
            [
                Constraint::Length(1),
                Constraint::Length(14),
                Constraint::Length(16),
                Constraint::Min(20),
            ],
        )
        .header(
            Row::new(["", "Pair", "Action", "Title"]).style(
                Style::default()
                    .fg(color_muted())
                    .add_modifier(Modifier::BOLD),
            ),
        )
    } else {
        Table::new(
            rows,
            [
                Constraint::Length(1),
                Constraint::Length(16),
                Constraint::Length(20),
                Constraint::Percentage(28),
                Constraint::Percentage(24),
                Constraint::Percentage(20),
            ],
        )
        .header(
            Row::new(["", "Pair", "Action", "Title", "Reason", "Resolution"]).style(
                Style::default()
                    .fg(color_muted())
                    .add_modifier(Modifier::BOLD),
            ),
        )
    };

    let title = format!(
        "Dry-Run Report ({}/{}, filter {}, sort {})",
        visible_rows.len(),
        model.report_rows.len(),
        model.report_filter.label(),
        model.report_sort.label()
    );
    frame.render_widget(table.block(chrome_block(&title)), area);
}

fn report_row(index: usize, row: &AppReportRow, model: &AppModel, compact: bool) -> Row<'static> {
    let selected = model.selected_report_index == Some(index);
    let marker = if selected { ">" } else { " " };
    let style = if selected {
        Style::default()
            .fg(color_running())
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(report_action_color(&row.action))
    };

    let cells = if compact {
        vec![
            marker.to_string(),
            compact_string(&row.pair_id, 14),
            compact_string(&row.action, 16),
            compact_string(&row.title, 36),
        ]
    } else {
        vec![
            marker.to_string(),
            compact_string(&row.pair_id, 16),
            compact_string(&row.action, 20),
            compact_string(&row.title, 36),
            compact_string(&row.reason, 32),
            compact_string(&row.resolution, 28),
        ]
    };

    Row::new(cells).style(style)
}

fn render_report_detail(frame: &mut Frame<'_>, area: Rect, model: &AppModel) {
    let lines = if let Some(row) = model.selected_report_row() {
        vec![
            Line::from(vec![Span::styled(
                compact_string(&row.action, 32),
                Style::default()
                    .fg(report_action_color(&row.action))
                    .add_modifier(Modifier::BOLD),
            )]),
            Line::from(format!(
                "Pair: {}",
                compact_detail_value(&row.pair_id, area.width)
            )),
            Line::from(format!(
                "Title: {}",
                compact_detail_value(&row.title, area.width)
            )),
            Line::from(format!(
                "Reason: {}",
                compact_detail_value(&row.reason, area.width)
            )),
            Line::from(format!(
                "Resolution: {}",
                compact_detail_value(&row.resolution, area.width)
            )),
            Line::from(format!(
                "Present: Google {} / iCloud {}",
                empty_dash(&row.google_present),
                empty_dash(&row.icloud_present)
            )),
            Line::from(format!(
                "Diff: {}",
                compact_detail_value(empty_dash(&row.diff_fields), area.width)
            )),
        ]
    } else {
        render_empty_state(
            frame,
            area,
            "Report Detail",
            &[
                "No report row selected.",
                "Use f/t to change filter and sort.",
            ],
            color_warning(),
        );
        return;
    };

    frame.render_widget(
        Paragraph::new(lines)
            .block(chrome_block("Report Detail"))
            .wrap(Wrap { trim: true }),
        area,
    );
}

fn render_conflicts_screen(frame: &mut Frame<'_>, area: Rect, model: &AppModel) {
    if area.width < 100 {
        let body = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Percentage(34),
                Constraint::Percentage(33),
                Constraint::Percentage(33),
            ])
            .split(area);
        render_conflict_summary_table(frame, body[0], model);
        render_conflict_detail_table(frame, body[1], model);
        render_conflict_workbench(frame, body[2], model);
    } else {
        let body = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(38), Constraint::Percentage(62)])
            .split(area);
        let right = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
            .split(body[1]);
        render_conflict_summary_table(frame, body[0], model);
        render_conflict_detail_table(frame, right[0], model);
        render_conflict_workbench(frame, right[1], model);
    }
}

fn render_conflict_summary_table(frame: &mut Frame<'_>, area: Rect, model: &AppModel) {
    if model.conflict_summaries.is_empty() {
        render_empty_state(
            frame,
            area,
            "Conflict Groups",
            &[
                "No unresolved conflicts.",
                "Run a sync to refresh conflict state.",
            ],
            color_success(),
        );
        return;
    }

    let compact = area.width < 84;
    let rows = model
        .conflict_summaries
        .iter()
        .enumerate()
        .map(|(index, summary)| conflict_summary_row(index, summary, model, compact));
    let table = if compact {
        Table::new(
            rows,
            [
                Constraint::Length(1),
                Constraint::Min(14),
                Constraint::Length(6),
                Constraint::Percentage(44),
            ],
        )
        .header(
            Row::new(["", "Pair", "Count", "Reason"]).style(
                Style::default()
                    .fg(color_muted())
                    .add_modifier(Modifier::BOLD),
            ),
        )
    } else {
        Table::new(
            rows,
            [
                Constraint::Length(1),
                Constraint::Length(18),
                Constraint::Length(7),
                Constraint::Percentage(36),
                Constraint::Percentage(28),
            ],
        )
        .header(
            Row::new(["", "Pair", "Count", "Reason", "Last Seen"]).style(
                Style::default()
                    .fg(color_muted())
                    .add_modifier(Modifier::BOLD),
            ),
        )
    };

    frame.render_widget(table.block(chrome_block("Conflict Groups")), area);
}

fn conflict_summary_row(
    index: usize,
    summary: &AppConflictSummary,
    model: &AppModel,
    compact: bool,
) -> Row<'static> {
    let selected = model.selected_conflict_index == Some(index);
    let marker = if selected { ">" } else { " " };
    let style = if selected {
        Style::default()
            .fg(color_warning())
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::White)
    };

    let cells = if compact {
        vec![
            marker.to_string(),
            compact_string(&summary.pair_id, 18),
            summary.count.to_string(),
            compact_string(&summary.reason, 36),
        ]
    } else {
        vec![
            marker.to_string(),
            compact_string(&summary.pair_id, 18),
            summary.count.to_string(),
            compact_string(&summary.reason, 42),
            compact_string(&summary.last_seen_at, 28),
        ]
    };

    Row::new(cells).style(style)
}

fn render_conflict_detail_table(frame: &mut Frame<'_>, area: Rect, model: &AppModel) {
    let details = model.selected_conflict_details();
    if details.is_empty() {
        render_empty_state(
            frame,
            area,
            "Conflict Detail",
            &[
                "No conflict group selected.",
                "Use j/k to choose a group with detail rows.",
            ],
            color_warning(),
        );
        return;
    }

    let compact = area.width < 86;
    let rows = details
        .iter()
        .map(|detail| conflict_detail_row(detail, compact));
    let table = if compact {
        Table::new(
            rows,
            [
                Constraint::Min(18),
                Constraint::Percentage(40),
                Constraint::Length(20),
            ],
        )
        .header(
            Row::new(["UID", "Diff", "Created"]).style(
                Style::default()
                    .fg(color_muted())
                    .add_modifier(Modifier::BOLD),
            ),
        )
    } else {
        Table::new(
            rows,
            [
                Constraint::Length(1),
                Constraint::Percentage(24),
                Constraint::Percentage(24),
                Constraint::Percentage(24),
                Constraint::Length(22),
            ],
        )
        .header(
            Row::new(["", "UID", "Google", "iCloud", "Created"]).style(
                Style::default()
                    .fg(color_muted())
                    .add_modifier(Modifier::BOLD),
            ),
        )
    };

    let title = model
        .selected_conflict_summary()
        .map(|summary| format!("Conflict Events ({})", summary.count))
        .unwrap_or_else(|| "Conflict Events".to_string());
    frame.render_widget(table.block(chrome_block(&title)), area);
}

fn conflict_detail_row(detail: &AppConflictDetail, compact: bool) -> Row<'static> {
    if compact {
        Row::new(vec![
            compact_string(detail.canonical_uid.as_deref().unwrap_or("-"), 24),
            compact_string(&detail.diff_fields, 36),
            compact_string(&detail.created_at, 20),
        ])
    } else {
        Row::new(vec![
            ">".to_string(),
            compact_string(detail.canonical_uid.as_deref().unwrap_or("-"), 26),
            compact_string(detail.google_title.as_deref().unwrap_or("-"), 28),
            compact_string(detail.icloud_title.as_deref().unwrap_or("-"), 28),
            compact_string(&detail.created_at, 22),
        ])
    }
    .style(Style::default().fg(Color::White))
}

fn render_conflict_workbench(frame: &mut Frame<'_>, area: Rect, model: &AppModel) {
    let details = model.selected_conflict_details();
    let Some(detail) = details.first() else {
        render_empty_state(
            frame,
            area,
            "Conflict Comparison",
            &[
                "No conflict event selected.",
                "Use j/k to choose a conflict group.",
            ],
            color_warning(),
        );
        return;
    };

    let summary = model.selected_conflict_summary();
    let lines = vec![
        Line::from(vec![
            Span::styled(
                "Policy: ",
                Style::default()
                    .fg(color_warning())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(compact_detail_value(&detail.resolution_policy, area.width)),
        ]),
        Line::from(format!(
            "Reason: {}",
            compact_detail_value(&detail.reason, area.width)
        )),
        Line::from(format!(
            "UID: {}",
            compact_detail_value(detail.canonical_uid.as_deref().unwrap_or("-"), area.width)
        )),
        Line::from(format!(
            "Diff: {}",
            compact_detail_value(&detail.diff_fields, area.width)
        )),
        Line::from(format!(
            "Google: {} | {} | {}",
            compact_string(detail.google_title.as_deref().unwrap_or("-"), 24),
            compact_string(detail.google_start.as_deref().unwrap_or("-"), 28),
            detail.google_status.as_deref().unwrap_or("-")
        )),
        Line::from(format!(
            "iCloud: {} | {} | {}",
            compact_string(detail.icloud_title.as_deref().unwrap_or("-"), 24),
            compact_string(detail.icloud_start.as_deref().unwrap_or("-"), 28),
            detail.icloud_status.as_deref().unwrap_or("-")
        )),
        Line::from(format!(
            "Audit: unresolved since {}; group first {} last {}; link {}",
            detail.created_at,
            summary
                .map(|summary| summary.first_seen_at.as_str())
                .unwrap_or("-"),
            summary
                .map(|summary| summary.last_seen_at.as_str())
                .unwrap_or("-"),
            detail.event_link_id.as_deref().unwrap_or("-")
        )),
        Line::from(format!(
            "Provider IDs: G {} / I {}",
            compact_string(detail.google_event_id.as_deref().unwrap_or("-"), 28),
            compact_string(detail.icloud_href.as_deref().unwrap_or("-"), 28)
        )),
    ];

    frame.render_widget(
        Paragraph::new(lines)
            .block(chrome_block("Conflict Comparison"))
            .wrap(Wrap { trim: true }),
        area,
    );
}

fn render_command_bar(frame: &mut Frame<'_>, area: Rect, model: &AppModel) {
    let mut spans = vec![
        Span::styled(
            "d",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" dry   "),
        Span::styled(
            "a",
            Style::default()
                .fg(color_danger())
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" apply   "),
        Span::styled(
            "r",
            Style::default()
                .fg(color_warning())
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" ref   "),
        Span::styled(
            "s",
            Style::default()
                .fg(color_neutral())
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" set   "),
        Span::styled(
            "c",
            Style::default()
                .fg(color_warning())
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" conf   "),
        Span::styled(
            "l",
            Style::default()
                .fg(color_success())
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" runs   "),
        Span::styled(
            "v",
            Style::default()
                .fg(color_checking())
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" report   "),
        Span::styled(
            "p",
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" pairs   "),
        Span::styled(
            "b",
            Style::default()
                .fg(color_warning())
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(if model.background_paused {
            " resume   "
        } else {
            " pause   "
        }),
        Span::styled(
            ":",
            Style::default()
                .fg(color_running())
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" pal   "),
    ];

    if model.view == AppView::Runs || model.view == AppView::Reports {
        spans.extend([
            Span::styled(
                "f",
                Style::default()
                    .fg(color_warning())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" filter   "),
        ]);
    }
    if model.view == AppView::Reports {
        spans.extend([
            Span::styled(
                "t",
                Style::default()
                    .fg(color_checking())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" sort   "),
        ]);
    }

    spans.extend([
        Span::styled(
            "j/k",
            Style::default()
                .fg(Color::Blue)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" move   "),
        Span::styled(
            "q",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ),
        Span::raw(" quit"),
    ]);
    let commands = Line::from(spans);

    frame.render_widget(
        Paragraph::new(commands)
            .block(chrome_block("Commands"))
            .alignment(Alignment::Center),
        area,
    );
}

fn render_command_palette(frame: &mut Frame<'_>, area: Rect, model: &AppModel) {
    let area = centered_rect(72, 58, area);
    frame.render_widget(Clear, area);
    let shell = model.shell_snapshot();

    let commands = shell
        .actions
        .iter()
        .enumerate()
        .map(|(index, action)| command_palette_item(index, action, model))
        .collect::<Vec<_>>();

    let help = Line::from(vec![
        Span::styled(
            "enter",
            Style::default()
                .fg(color_running())
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" run   "),
        Span::styled(
            "j/k",
            Style::default()
                .fg(color_checking())
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" move   "),
        Span::styled(
            "esc",
            Style::default()
                .fg(color_warning())
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" close"),
    ]);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(8), Constraint::Length(3)])
        .split(area);
    frame.render_widget(
        List::new(commands).block(chrome_block("Command Palette")),
        chunks[0],
    );
    frame.render_widget(
        Paragraph::new(help)
            .block(chrome_block("Action"))
            .alignment(Alignment::Center),
        chunks[1],
    );
}

fn command_palette_item(
    index: usize,
    action: &AppShellAction,
    model: &AppModel,
) -> ListItem<'static> {
    let selected = index == model.selected_command_index;
    let marker = if selected { ">" } else { " " };
    let style = if selected {
        Style::default()
            .fg(command_color(action.command))
            .add_modifier(Modifier::BOLD)
    } else if !action.enabled {
        Style::default().fg(color_muted())
    } else {
        Style::default().fg(Color::White)
    };

    ListItem::new(Line::from(vec![
        Span::styled(marker.to_string(), style),
        Span::raw(" "),
        Span::styled(compact_string(&action.label, 20), style),
        Span::raw("  "),
        Span::styled(
            compact_string(&action.description, 56),
            Style::default().fg(color_muted()),
        ),
    ]))
}

fn render_empty_state(
    frame: &mut Frame<'_>,
    area: Rect,
    title: &str,
    lines: &[&str],
    color: Color,
) {
    let lines = lines
        .iter()
        .enumerate()
        .map(|(index, line)| {
            if index == 0 {
                Line::from(Span::styled(
                    *line,
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                ))
            } else {
                Line::from(Span::styled(*line, Style::default().fg(color_muted())))
            }
        })
        .collect::<Vec<_>>();

    frame.render_widget(
        Paragraph::new(lines)
            .block(chrome_block(title))
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true }),
        area,
    );
}

fn activity_item(value: String, color: Color) -> ListItem<'static> {
    ListItem::new(value).style(Style::default().fg(color))
}

fn chrome_block<'a>(title: &'a str) -> Block<'a> {
    Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(color_muted()))
}

fn status_label(status: AppStatus) -> &'static str {
    match status {
        AppStatus::Idle => "idle",
        AppStatus::Checking => "checking",
        AppStatus::Syncing => "syncing",
        AppStatus::Error => "error",
    }
}

fn status_color(status: AppStatus) -> Color {
    match status {
        AppStatus::Idle => color_neutral(),
        AppStatus::Checking => color_checking(),
        AppStatus::Syncing => color_running(),
        AppStatus::Error => color_danger(),
    }
}

fn view_label(view: AppView) -> &'static str {
    match view {
        AppView::Dashboard => "pairs",
        AppView::Setup => "setup",
        AppView::Runs => "runs",
        AppView::Reports => "reports",
        AppView::Conflicts => "conflicts",
    }
}

fn run_status_color(status: &str) -> Color {
    match status {
        "failed" => color_danger(),
        "completed" => color_success(),
        "running" => color_running(),
        _ => color_neutral(),
    }
}

fn pair_gauge_color(model: &AppModel) -> Color {
    if model.pairs.is_empty() {
        color_muted()
    } else if model.enabled_pair_count() == model.pairs.len() {
        color_success()
    } else {
        color_warning()
    }
}

fn last_run_color(model: &AppModel) -> Color {
    match model.last_run_status.as_deref() {
        Some(status) => run_status_color(status),
        None => color_muted(),
    }
}

fn command_color(command: AppCommand) -> Color {
    match command {
        AppCommand::DryRun => color_running(),
        AppCommand::ApplyRun => color_danger(),
        AppCommand::RefreshConflicts => color_warning(),
        AppCommand::ShowConflicts => color_warning(),
        AppCommand::OpenSetup => color_neutral(),
        AppCommand::ShowPairs => Color::White,
        AppCommand::ShowRuns => color_success(),
        AppCommand::ShowReports => color_checking(),
        AppCommand::ToggleBackgroundPause => color_warning(),
        AppCommand::ExportReport => color_checking(),
        AppCommand::Quit => color_danger(),
    }
}

fn setup_status_label(status: AppSetupStepStatus) -> &'static str {
    match status {
        AppSetupStepStatus::Complete => "Ready",
        AppSetupStepStatus::Attention => "Check",
        AppSetupStepStatus::Missing => "Todo",
    }
}

fn setup_status_color(status: AppSetupStepStatus) -> Color {
    match status {
        AppSetupStepStatus::Complete => color_success(),
        AppSetupStepStatus::Attention => color_warning(),
        AppSetupStepStatus::Missing => color_danger(),
    }
}

fn report_action_color(action: &str) -> Color {
    if action.contains("delete") {
        color_danger()
    } else if action.contains("conflict") || action.contains("manual") {
        color_warning()
    } else if action.contains("create") {
        color_success()
    } else if action.contains("update") {
        color_running()
    } else {
        Color::White
    }
}

fn notification_label(severity: AppNotificationSeverity) -> &'static str {
    match severity {
        AppNotificationSeverity::Info => "Info",
        AppNotificationSeverity::Warning => "Warning",
        AppNotificationSeverity::Error => "Error",
    }
}

fn notification_color(severity: AppNotificationSeverity) -> Color {
    match severity {
        AppNotificationSeverity::Info => color_checking(),
        AppNotificationSeverity::Warning => color_warning(),
        AppNotificationSeverity::Error => color_danger(),
    }
}

fn color_neutral() -> Color {
    Color::Gray
}

fn color_muted() -> Color {
    Color::DarkGray
}

fn color_checking() -> Color {
    Color::Blue
}

fn color_running() -> Color {
    Color::Cyan
}

fn color_success() -> Color {
    Color::Green
}

fn color_warning() -> Color {
    Color::Yellow
}

fn color_danger() -> Color {
    Color::Red
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1])[1]
}

fn last_run_label(model: &AppModel) -> String {
    match (
        model.last_run_status.as_deref(),
        model.last_run_at.as_deref(),
    ) {
        (Some(status), Some(at)) => format!("{status} at {at}"),
        (Some(status), None) => status.to_string(),
        _ => "No runs yet".to_string(),
    }
}

fn next_run_label(model: &AppModel) -> String {
    model
        .next_run_at
        .clone()
        .unwrap_or_else(|| "not scheduled".to_string())
}

fn direction_label(direction: insync_core::SyncDirection) -> &'static str {
    match direction {
        insync_core::SyncDirection::TwoWay => "two-way",
        insync_core::SyncDirection::LeftToRight => "google -> icloud",
        insync_core::SyncDirection::RightToLeft => "icloud -> google",
    }
}

fn compact_calendar_label(name: Option<&str>, account: Option<&str>, fallback_id: &str) -> String {
    compact_string(&calendar_detail_label(name, account, fallback_id), 28)
}

fn calendar_detail_label(name: Option<&str>, account: Option<&str>, fallback_id: &str) -> String {
    let label = name
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(fallback_id);
    match account.filter(|value| !value.trim().is_empty()) {
        Some(account) => format!("{label} [{account}]"),
        None => label.to_string(),
    }
}

fn compact_detail_value(value: &str, area_width: u16) -> String {
    let max = usize::from(area_width.saturating_sub(14)).clamp(16, 64);
    compact_string(value, max)
}

fn empty_dash(value: &str) -> &str {
    if value.is_empty() { "-" } else { value }
}

fn compact_string(value: &str, max: usize) -> String {
    let char_count = value.chars().count();
    if char_count <= max {
        value.to_string()
    } else {
        let suffix = value
            .chars()
            .skip(char_count.saturating_sub(max - 3))
            .collect::<String>();
        format!("...{suffix}")
    }
}
