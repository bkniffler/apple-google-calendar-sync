use clap::{Parser, Subcommand};
use color_eyre::eyre::{Context, Result, bail};
use crossterm::{
    event::{self, Event, KeyCode},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use insync_app::{AppEvent, AppModel, AppRuntimeSnapshot, AppStatus};
use insync_config::{credentials::resolve_credentials, default_config_path, load_config};
use insync_core::{CanonicalEvent, ProviderEventMeta, ProviderName};
use insync_engine::{
    ConflictFilter, DoctorSummary, ReportMode, RunMode, SyncEngine, SyncProviders,
    UnresolvedConflictRow, UnresolvedConflictSummary,
};
use insync_providers::{
    CalendarProvider, ProviderCalendar, ProviderChangeSet, ProviderError, ProviderSyncCursor,
    google::{GoogleCalendarProvider, GoogleEvent, GoogleProviderOptions, google_to_canonical},
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
use std::{fs, io, path::PathBuf, time::Duration};
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
#[command(name = "insync", version, about = "iCloud <-> Google Calendar sync")]
struct Cli {
    #[arg(long, env = "INSYNC_CONFIG")]
    config: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Doctor,
    Conflicts {
        #[arg(long)]
        details: bool,
        #[arg(long)]
        reason: Option<String>,
        #[arg(long)]
        pair: Option<String>,
        #[arg(long, default_value_t = 100)]
        limit: u32,
        #[arg(long)]
        csv: Option<PathBuf>,
        #[arg(long)]
        dedupe: bool,
    },
    Sync {
        #[arg(long)]
        apply: bool,
        #[arg(long)]
        fixtures: Option<PathBuf>,
        #[arg(long)]
        report: Option<PathBuf>,
        #[arg(long)]
        report_all: bool,
    },
    Daemon {
        #[arg(long)]
        apply: bool,
    },
    Tui,
}

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;
    init_tracing();

    let cli = Cli::parse();
    let config_path = match cli.config {
        Some(path) => path,
        None => default_config_path()?,
    };
    let config = load_config(&config_path)
        .wrap_err_with(|| format!("loading config {}", config_path.display()))?;

    match cli.command {
        Command::Doctor => {
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
        Command::Conflicts {
            details,
            reason,
            pair,
            limit,
            csv,
            dedupe,
        } => {
            let engine = SyncEngine::with_config_path(config.clone(), &config_path);

            if dedupe {
                let resolved = engine.dedupe_conflicts()?;
                println!("deduped unresolved conflicts: {resolved}");
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
        } => {
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
                if apply {
                    bail!(
                        "Rust apply mode is not wired to provider writes yet; run without --apply for live dry-run planning"
                    );
                }

                let providers = live_providers(config.clone(), &config_path)?;
                let summary = engine
                    .plan_once_with_providers_and_report_mode(
                        RunMode::DryRun,
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
                println!(
                    "planned live Rust dry-run: db={}, configured_pairs={}, enabled_pairs={}, action_counts={:?}, resolution_counts={:?}, mode={:?}",
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
            let engine = SyncEngine::with_config_path(config.clone(), &config_path);
            println!("starting daemon scaffold; press Ctrl-C to stop");
            engine
                .run_forever(
                    if apply {
                        RunMode::Apply
                    } else {
                        RunMode::DryRun
                    },
                    async {
                        let _ = tokio::signal::ctrl_c().await;
                    },
                )
                .await?;
        }
        Command::Tui => {
            let engine = SyncEngine::with_config_path(config.clone(), &config_path);
            let doctor = engine.doctor()?;
            let mut model = AppModel::from_config(&config);
            model.apply_runtime_snapshot(runtime_snapshot_from_doctor(&doctor));
            run_tui(model)?;
        }
    }

    Ok(())
}

fn runtime_snapshot_from_doctor(summary: &DoctorSummary) -> AppRuntimeSnapshot {
    AppRuntimeSnapshot {
        conflict_count: usize::try_from(summary.unresolved_conflict_count).unwrap_or(usize::MAX),
        last_run_at: summary.latest_run.as_ref().map(|run| {
            run.finished_at
                .clone()
                .unwrap_or_else(|| run.started_at.clone())
        }),
        last_run_status: summary.latest_run.as_ref().map(|run| run.status.clone()),
        recent_error: summary
            .latest_run
            .as_ref()
            .and_then(|run| run.error.clone()),
    }
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
            compact_calendar_id(&pair.google_calendar_id),
            compact_calendar_id(&pair.icloud_calendar_id),
        ]
    };

    Row::new(cells).style(style)
}

fn render_selected_pair(frame: &mut Frame<'_>, area: Rect, model: &AppModel) {
    let lines = if let Some(pair) = model.selected_pair() {
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
                compact_detail_value(&pair.google_calendar_id, area.width)
            )),
            Line::from(format!(
                "iCloud: {}",
                compact_detail_value(&pair.icloud_calendar_id, area.width)
            )),
        ]
    } else {
        vec![
            Line::from("No calendar pairs configured."),
            Line::from("Run setup once provider clients are wired."),
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

fn direction_label(direction: insync_core::SyncDirection) -> &'static str {
    match direction {
        insync_core::SyncDirection::TwoWay => "two-way",
        insync_core::SyncDirection::LeftToRight => "google -> icloud",
        insync_core::SyncDirection::RightToLeft => "icloud -> google",
    }
}

fn compact_calendar_id(value: &str) -> String {
    const MAX: usize = 28;
    compact_string(value, MAX)
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
