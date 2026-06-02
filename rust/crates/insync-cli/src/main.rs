use chrono::{DateTime, Duration as ChronoDuration, Utc};
use clap::{Parser, Subcommand, ValueEnum};
use color_eyre::eyre::{Context, Result, bail};
use crossterm::{
    event::{self, Event, KeyCode},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use insync_app::{AppEvent, AppModel, AppPairRuntimeSnapshot, AppRuntimeSnapshot, AppStatus};
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
    },
};
use insync_engine::{
    ConflictFilter, DoctorSummary, ReportMode, RunMode, SyncEngine, SyncProviders,
    UnresolvedConflictRow, UnresolvedConflictSummary,
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
    widgets::{Block, BorderType, Borders, Gauge, List, ListItem, Paragraph, Row, Table, Wrap},
};
use serde::Deserialize;
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
    println!("id\tsync_pair_id\tcanonical_uid\treason\tcreated_at");
    for row in rows {
        println!(
            "{}\t{}\t{}\t{}\t{}",
            row.id,
            row.sync_pair_id,
            row.canonical_uid.as_deref().unwrap_or(""),
            row.reason,
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
    let lines = std::iter::once("id,sync_pair_id,canonical_uid,reason,created_at".to_string())
        .chain(rows.iter().map(|row| {
            [
                row.id.clone(),
                row.sync_pair_id.clone(),
                row.canonical_uid.clone().unwrap_or_default(),
                row.reason.clone(),
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
            match key.code {
                KeyCode::Char('q') => break Ok(()),
                KeyCode::Down | KeyCode::Char('j') => model.select_next_pair(),
                KeyCode::Up | KeyCode::Char('k') => model.select_previous_pair(),
                KeyCode::Char('d') => {
                    model.update(AppEvent::StartDryRun);
                    model.update(AppEvent::EngineFinished {
                        message: "dry-run requested".to_string(),
                    });
                }
                KeyCode::Char('a') => {
                    model.update(AppEvent::StartApplyRun);
                    model.update(AppEvent::EngineFinished {
                        message: "apply requested".to_string(),
                    });
                }
                KeyCode::Char('r') => {
                    model.update(AppEvent::RefreshConflicts);
                    model.update(AppEvent::EngineFinished {
                        message: "conflict refresh requested".to_string(),
                    });
                }
                KeyCode::Char('s') => {
                    model.update(AppEvent::OpenSetup);
                    model.update(AppEvent::EngineFinished {
                        message: "setup requested".to_string(),
                    });
                }
                _ => {}
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

fn draw_tui(frame: &mut Frame<'_>, model: &AppModel) {
    let area = frame.area();
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4),
            Constraint::Length(7),
            Constraint::Min(12),
            Constraint::Length(4),
        ])
        .split(area);

    render_header(frame, vertical[0], model);
    render_metrics(frame, vertical[1], model);

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
    render_command_bar(frame, vertical[3]);
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
            .gauge_style(Style::default().fg(Color::Green))
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
            Color::Green
        } else {
            Color::Yellow
        },
    );
    render_metric(
        frame,
        chunks[3],
        "Last Run",
        last_run_label(model).as_str(),
        match model.last_run_status.as_deref() {
            Some("failed") => Color::Red,
            Some("completed") => Color::Green,
            Some("running") => Color::Cyan,
            _ => Color::Gray,
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
                    .fg(Color::Gray)
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
                    .fg(Color::Gray)
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
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else if pair.enabled {
        Style::default().fg(Color::White)
    } else {
        Style::default().fg(Color::DarkGray)
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
        vec![
            Line::from("No calendar pairs configured."),
            Line::from("Run insync setup --interactive to add calendars."),
        ]
    };

    frame.render_widget(
        Paragraph::new(lines)
            .block(chrome_block("Selected Pair"))
            .wrap(Wrap { trim: true }),
        area,
    );
}

fn render_activity(frame: &mut Frame<'_>, area: Rect, model: &AppModel) {
    let items = [
        format!("Status: {}", status_label(model.status)),
        format!("Unresolved conflicts: {}", model.conflict_count),
        format!("Last run: {}", last_run_label(model)),
        format!("Next run: {}", next_run_label(model)),
        format!(
            "Recent error: {}",
            model.recent_error.as_deref().unwrap_or("-")
        ),
        format!(
            "App message: {}",
            model.last_message.as_deref().unwrap_or("-")
        ),
    ];
    let items = items
        .into_iter()
        .map(|item| ListItem::new(item).style(Style::default().fg(Color::White)))
        .collect::<Vec<_>>();

    frame.render_widget(List::new(items).block(chrome_block("Activity")), area);
}

fn render_command_bar(frame: &mut Frame<'_>, area: Rect) {
    let commands = Line::from(vec![
        Span::styled(
            "d",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" dry-run   "),
        Span::styled(
            "a",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" apply   "),
        Span::styled(
            "r",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" conflicts   "),
        Span::styled(
            "s",
            Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" setup   "),
        Span::styled(
            "j/k",
            Style::default()
                .fg(Color::Blue)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" select   "),
        Span::styled(
            "q",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ),
        Span::raw(" quit"),
    ]);

    frame.render_widget(
        Paragraph::new(commands)
            .block(chrome_block("Commands"))
            .alignment(Alignment::Center),
        area,
    );
}

fn chrome_block<'a>(title: &'a str) -> Block<'a> {
    Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::DarkGray))
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
        AppStatus::Idle => Color::Gray,
        AppStatus::Checking => Color::Blue,
        AppStatus::Syncing => Color::Cyan,
        AppStatus::Error => Color::Red,
    }
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
