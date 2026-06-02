# Tray And Menu-Bar UI Notes

This note records the current direction for a future taskbar/menu-bar shell.
The terminal UI remains the primary Rust surface for now; tray UI should reuse
`insync-app` actions, notifications, setup state, and `insync-engine` instead of
forking sync behavior.

## Recommendation

Build a small tray shell after Rust live parity is proven. Start with Tauri v2
if we want a polished popover/window for setup, conflicts, reports, and status.
Use `tray-icon` directly only if the shell stays menu-only and does not need a
rich window.

Why Tauri first:

- Tauri v2 has first-class tray APIs in Rust and JavaScript, including tray
  icons, menus, click/menu event handlers, and menu-on-left-click control.
- Tauri native menus can be attached to windows or system trays, and macOS menu
  behavior is documented separately from Windows/Linux behavior.
- A Tauri shell lets us keep a lightweight Rust core while using a small web
  view for conflict/report/setup screens that are awkward in a native tray menu.

Why not implement tray now:

- The sync engine still needs live apply proof on throwaway calendars.
- Linux tray behavior depends on desktop environment support and system
  libraries, so it needs packaging validation.
- macOS signing/notarization should be designed together with the app bundle,
  icon, launch-at-login behavior, and update channel.

Primary references:

- Tauri v2 system tray guide:
  <https://v2.tauri.app/learn/system-tray/>
- Tauri v2 native menu guide:
  <https://v2.tauri.app/learn/window-menu/>
- `tray-icon` crate repository:
  <https://github.com/tauri-apps/tray-icon>

## Platform Notes

### macOS

The tray/menu-bar shell should behave like a menu-bar app:

- Use a template-style monochrome status icon.
- Keep the app menu minimal: open dashboard, dry-run now, apply now, open
  conflicts, pause/resume background sync, quit.
- If shipped as a proper app bundle, plan for codesigning and notarization.
- Background scheduling can keep using launchd, but the UI should show whether
  the launchd agent is installed/running rather than owning scheduling itself.

`tray-icon` requires the event loop and tray creation on the main thread on
macOS. Tauri handles most of this app-shell plumbing if we go that route.

### Windows

Use a normal notification-area tray icon with a context menu and optional small
dashboard window. The existing background story is still weaker here than on
macOS/Linux, so Windows should get explicit scheduled-task/service packaging
before we promise background reliability.

For a Tauri shell, use the same action IDs as `insync-app`: dry-run, apply,
refresh conflicts, setup, show reports, pause/resume, quit.

### Linux

Linux is the highest-variance platform. `tray-icon` documents GTK event-loop
requirements and system dependencies such as GTK, `libxdo`, and AppIndicator or
Ayatana AppIndicator libraries. Some desktops show tray icons by default; GNOME
commonly requires an extension. Treat Linux tray support as best effort until
tested on at least GNOME, KDE Plasma, and one lightweight desktop.

Packaging should document/install these dependencies per distro if we ship a
tray binary.

## Proposed Crate Shape

Keep the current core crates as-is and add a separate shell crate later:

```text
rust/crates/
  insync-app/         shared model, actions, setup state, notifications
  insync-engine/      sync orchestration
  insync-cli/         CLI, TUI, daemon, background install
  insync-tray/        future Tauri/tray shell
```

The tray shell should:

- Load config through `insync-config`.
- Build an `AppModel` from config plus runtime snapshots.
- Render status/actions from `AppModel::shell_snapshot()`.
- Dispatch menu/window actions back through `AppEvent`/`AppEffect`.
- Use `insync-engine` for dry-run/apply/doctor/conflict refresh.
- Avoid storing credentials or calendar data outside config, OS secret store,
  and SQLite.

## First Prototype Scope

Do this only after dry-run/apply parity work:

1. Add `insync-tray` as a separate crate.
2. Show a tray icon and menu with status, dry-run, apply, conflicts, setup,
   pause/resume, quit.
3. Reuse `AppShellSnapshot` for menu labels/enabled/destructive state.
4. Show a small window for the setup wizard and conflict/report views.
5. Package macOS as an app bundle before considering notarization.
6. Validate Linux tray dependencies on target desktops.
