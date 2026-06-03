//! Cross-platform menu-bar / system-tray app for insync.
//!
//! This is intentionally a thin *control surface* over the existing `insync`
//! CLI: it supervises a background `daemon --apply` child, can trigger one-shot
//! syncs, launches the TUI dashboard and setup wizard in a terminal, and shows
//! live status through the tray icon colour and a native context menu.
//!
//! All credential / provider / engine logic stays in the CLI so the tray never
//! has to touch the OS secret store on its own; status is read cheaply straight
//! from the SQLite cache on a refresh timer.

mod icon;
mod status;
mod supervisor;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use tao::event::{Event, StartCause};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{TrayIcon, TrayIconBuilder};

use status::{StatusSnapshot, SyncState, resolve_db_path};
use supervisor::{Supervisor, locate_insync_binary};

/// How often the tray re-reads status from the database.
const REFRESH_INTERVAL: Duration = Duration::from_secs(2);

/// Custom event type pumped into the tao event loop. Menu clicks arrive on a
/// separate muda callback thread and are forwarded here via the event-loop
/// proxy so all handling happens on the main loop.
enum UserEvent {
    Menu(MenuEvent),
}

fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let config_path = resolve_config_path();
    let db_path = resolve_db_path(&config_path);
    let binary = locate_insync_binary();
    let log_dir = log_dir(db_path.as_deref());

    tracing::info!(
        config = %config_path.display(),
        binary = %binary.display(),
        configured = db_path.is_some(),
        "starting insync tray"
    );

    let mut supervisor = Supervisor::new(binary, config_path.clone(), log_dir);

    // The user opted for "background applies automatically": start the daemon
    // immediately when the app is configured. When unconfigured we stay idle
    // and surface the setup wizard instead.
    let mut background_enabled = false;
    if db_path.is_some() {
        match supervisor.start_background() {
            Ok(()) => background_enabled = true,
            Err(err) => tracing::warn!(%err, "failed to start background daemon"),
        }
    }

    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();

    // Forward muda menu events into our event loop.
    let proxy = event_loop.create_proxy();
    MenuEvent::set_event_handler(Some(move |event| {
        let _ = proxy.send_event(UserEvent::Menu(event));
    }));

    let menu = TrayMenu::new();
    // The tray icon must be constructed after the event loop is running (and on
    // macOS, after the NSApplication is initialised), so it is created lazily in
    // `StartCause::Init` below.
    let mut tray: Option<TrayIcon> = None;

    let initial = snapshot(db_path.as_deref(), &mut supervisor);
    menu.apply(&initial);

    event_loop.run(move |event, _target, control_flow| {
        *control_flow = ControlFlow::WaitUntil(Instant::now() + REFRESH_INTERVAL);

        match event {
            Event::NewEvents(StartCause::Init) => {
                let snap = snapshot(db_path.as_deref(), &mut supervisor);
                menu.apply(&snap);
                match TrayIconBuilder::new()
                    .with_menu(Box::new(menu.menu.clone()))
                    .with_tooltip(snap.summary_line())
                    .with_icon(icon::icon_for(snap.state))
                    .build()
                {
                    Ok(t) => tray = Some(t),
                    Err(err) => tracing::error!(%err, "failed to build tray icon"),
                }
            }
            Event::NewEvents(StartCause::ResumeTimeReached { .. }) => {
                let snap = snapshot(db_path.as_deref(), &mut supervisor);
                menu.apply(&snap);
                update_tray(tray.as_ref(), &snap);
            }
            Event::UserEvent(UserEvent::Menu(menu_event))
                if menu.handle(
                    &menu_event,
                    &mut supervisor,
                    &mut background_enabled,
                    control_flow,
                ) =>
            {
                let snap = snapshot(db_path.as_deref(), &mut supervisor);
                menu.apply(&snap);
                update_tray(tray.as_ref(), &snap);
            }
            _ => {}
        }
    });
}

/// Read a fresh status snapshot, folding in whether the daemon child is alive.
fn snapshot(db_path: Option<&Path>, supervisor: &mut Supervisor) -> StatusSnapshot {
    let background_running = supervisor.background_running();
    match db_path {
        Some(path) => StatusSnapshot::read(path, background_running),
        None => StatusSnapshot::unconfigured(),
    }
}

/// Push the latest snapshot onto the live tray icon (colour + tooltip).
fn update_tray(tray: Option<&TrayIcon>, snap: &StatusSnapshot) {
    if let Some(tray) = tray {
        let _ = tray.set_icon(Some(icon::icon_for(snap.state)));
        let _ = tray.set_tooltip(Some(snap.summary_line()));
    }
}

/// The native context menu plus stable handles to its dynamic items.
struct TrayMenu {
    menu: Menu,
    status_item: MenuItem,
    detail_item: MenuItem,
    sync_now_item: MenuItem,
    toggle_bg_item: MenuItem,
    dashboard_item: MenuItem,
    setup_item: MenuItem,
    quit_item: MenuItem,
}

impl TrayMenu {
    fn new() -> Self {
        let menu = Menu::new();
        let status_item = MenuItem::new("insync", false, None);
        let detail_item = MenuItem::new("…", false, None);
        let sync_now_item = MenuItem::new("Sync now", false, None);
        let toggle_bg_item = MenuItem::new("Pause background sync", true, None);
        let dashboard_item = MenuItem::new("Open dashboard", true, None);
        let setup_item = MenuItem::new("Setup…", true, None);
        let quit_item = MenuItem::new("Quit insync", true, None);

        // Build order top-to-bottom; ignore append errors (only happen on bad
        // platform state, in which case the menu is simply incomplete).
        let _ = menu.append_items(&[
            &status_item,
            &detail_item,
            &PredefinedMenuItem::separator(),
            &sync_now_item,
            &toggle_bg_item,
            &PredefinedMenuItem::separator(),
            &dashboard_item,
            &setup_item,
            &PredefinedMenuItem::separator(),
            &quit_item,
        ]);

        Self {
            menu,
            status_item,
            detail_item,
            sync_now_item,
            toggle_bg_item,
            dashboard_item,
            setup_item,
            quit_item,
        }
    }

    /// Sync the dynamic menu labels / enabled state to a status snapshot.
    fn apply(&self, snap: &StatusSnapshot) {
        self.status_item.set_text(snap.summary_line());
        self.detail_item.set_text(snap.detail_line());

        let configured = snap.state != SyncState::Unconfigured;
        let syncing = snap.state == SyncState::Syncing;

        // A manual one-shot sync is only safe while the background daemon is
        // paused (single writer) and nothing is already running.
        self.sync_now_item
            .set_enabled(configured && !snap.background_running && !syncing);

        self.toggle_bg_item.set_enabled(configured);
        self.toggle_bg_item.set_text(if snap.background_running {
            "Pause background sync"
        } else {
            "Start background sync"
        });

        self.dashboard_item.set_enabled(configured);
    }

    /// Handle a menu click. Returns `true` if the caller should refresh status.
    fn handle(
        &self,
        event: &MenuEvent,
        supervisor: &mut Supervisor,
        background_enabled: &mut bool,
        control_flow: &mut ControlFlow,
    ) -> bool {
        let id = &event.id;

        if id == self.quit_item.id() {
            supervisor.stop_background();
            *control_flow = ControlFlow::Exit;
            return false;
        }

        if id == self.sync_now_item.id() {
            if let Err(err) = supervisor.run_sync_once() {
                tracing::warn!(%err, "failed to start one-shot sync");
            }
            return true;
        }

        if id == self.toggle_bg_item.id() {
            if supervisor.background_running() {
                supervisor.stop_background();
                *background_enabled = false;
            } else if let Err(err) = supervisor.start_background() {
                tracing::warn!(%err, "failed to start background daemon");
            } else {
                *background_enabled = true;
            }
            return true;
        }

        if id == self.dashboard_item.id() {
            if let Err(err) = supervisor.open_dashboard() {
                tracing::warn!(%err, "failed to open dashboard");
            }
            return false;
        }

        if id == self.setup_item.id() {
            if let Err(err) = supervisor.open_setup() {
                tracing::warn!(%err, "failed to open setup");
            }
            return false;
        }

        false
    }
}

/// Resolve the config path, honouring a `--config <path>` argument and otherwise
/// delegating to the shared config resolution (env + local file + OS app dir).
fn resolve_config_path() -> PathBuf {
    let mut args = std::env::args_os().skip(1);
    let mut explicit: Option<PathBuf> = None;
    while let Some(arg) = args.next() {
        if arg == "--config" {
            explicit = args.next().map(PathBuf::from);
            break;
        }
    }
    insync_config::resolve_config_path(explicit).unwrap_or_else(|err| {
        tracing::warn!(%err, "could not resolve config path; falling back to ./insync.json");
        PathBuf::from("insync.json")
    })
}

/// Pick a directory for child-process log files. Prefer beside the database so
/// logs live with the rest of the app data; fall back to the temp dir.
fn log_dir(db_path: Option<&Path>) -> PathBuf {
    db_path
        .and_then(|p| p.parent())
        .map(|dir| dir.join("logs"))
        .unwrap_or_else(std::env::temp_dir)
}
