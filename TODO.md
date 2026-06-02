# TODO

This is the working backlog for moving insync from the current Bun/TypeScript
service to a Rust application with a strong terminal UI, background sync, and a
future interchangeable taskbar/menu-bar interface.

## Guiding Shape

- Keep the TypeScript app working until the Rust version reaches dry-run and
  apply parity.
- Keep sync decisions in pure, heavily tested Rust code.
- Keep database access explicit: SQLite, raw SQL migrations, typed repositories.
- Keep UI logic shell-independent: TUI now, tray/taskbar later.
- Treat the TUI as a primary product surface, not a debug console.

## 1. Rust Workspace

- [x] Commit the initial `rust/` workspace scaffold.
- [x] Add CI for `cd rust && cargo fmt --check && cargo test`.
- [x] Decide whether the root repo keeps both Bun and Rust checks during the
      migration or adds a dedicated Rust workflow.
- [x] Add release/profile defaults once the CLI starts doing real work.

## 2. Core Sync Logic

- [x] Port canonical event hashing from `src/sync/event-hash.ts`.
- [x] Add the first Rust two-way planner implementation with event links,
      hashes, directions, conflict policies, and auto-resolution metadata.
- [x] Port the existing hash tests and the current automatic conflict policy
      tests from `src/sync/planner.test.ts`.
- [x] Add focused planner coverage for linked updates, snapshots, manual
      conflicts, provider-winner policies, delete-wins, one-way direction
      guards, and known iCloud UID collisions.
- [x] Port the full TypeScript planner into `insync-core`.
- [x] Port all planner tests from `src/sync/planner.test.ts`.
- [x] Cover initial sync, linked sync, one-way directions, deletes, update
      detection, and no-op snapshots.
- [x] Cover automatic conflict policies:
      `google_wins`, `icloud_wins`, `newest_updated_wins`, `update_wins`,
      `delete_wins`, and manual fallback.
- [x] Cover iCloud UID collision handling and known-collision suppression.
- [x] Add fixture-based tests that compare TypeScript and Rust decisions for
      representative dry-run reports.

## 3. SQLite Layer

- [x] Port the full SQLite schema into `insync-db`.
- [x] Add typed repository shape for configured calendars/pairs, event links,
      sync state, and conflicts.
- [x] Add repository modules:
      `calendars` and any remaining support tables.
- [x] Add typed repository modules for event links, sync state, sync runs, and
      conflicts.
- [x] Add in-memory SQLite tests for the current repositories.
- [x] Add migration tests from empty DB to latest schema.
- [x] Add backup/export/import helpers for user support and debugging.
- [x] Keep raw SQL visible and boring; avoid Diesel unless repository code
      becomes a real maintenance problem.

## 4. Config And Secrets

- [x] Finish JSON read/write support in `insync-config`.
- [x] Support local config and OS app-data config paths.
- [x] Support explicit `--config` and `INSYNC_CONFIG` everywhere.
- [x] Port `secretStore: "none" | "os"` behavior.
- [x] Integrate OS keychain/credential storage for Google and iCloud secrets.
- [x] Move inline secrets into the OS secret store when configured.
- [x] Add non-secret config validation with clear diagnostics.
- [x] Add config migration tests for future schema versions.

## 5. Provider Mapping

- [x] Port Google event to canonical event mapping.
- [x] Port canonical event to Google API payload mapping.
- [x] Port Google calendar-list entry to provider calendar mapping.
- [x] Port iCalendar/CalDAV event to canonical event mapping.
- [x] Port canonical event to iCalendar payload mapping.
- [x] Port iCloud calendar metadata to provider calendar mapping.
- [x] Add fixture tests for all-day events, timed events, time zones,
      recurrence, attendees, reminders, cancelled events, privacy, and status.
- [x] Add round-trip tests where provider limitations allow it.
- [x] Document provider fields that cannot safely round-trip.

## 6. Provider Clients

- [x] Implement Google OAuth auth URL, auth-code exchange helper, and refresh
      flow.
- [x] Implement Google calendar list, event list, create, update, delete.
- [x] Implement iCloud CalDAV calendar discovery.
- [x] Implement iCloud event list, create, update, delete.
- [x] Port iCloud cross-calendar UID collision probing and metadata reuse from
      the TypeScript provider.
- [x] Add typed provider errors for auth, rate limits, precondition failures,
      UID collisions, network failures, and mapping failures.
- [x] Emit UID collision errors from iCloud cross-calendar collision probing.
- [x] Add retry/backoff policy for transient provider errors.
- [x] Add provider integration tests that can run against fixtures/mocks by
      default and real accounts only when explicitly enabled.

## 7. Sync Engine

- [x] Wire the Rust engine to open SQLite, run migrations, seed configured
      pairs, and return DB-backed doctor/sync scaffold summaries.
- [x] Resolve relative `dbPath` values relative to the config file directory.
- [x] Add injected-provider dry-run planning orchestration that fetches both
      providers, loads event links and known conflicts from SQLite, runs the
      core planner, and returns per-pair action/resolution counts.
- [x] Wire the default Rust `sync` dry-run command to real Google and iCloud
      provider clients.
- [x] Replace the scaffolded `insync-engine` runner with real orchestration.
- [x] Load config, resolve secrets, open SQLite, run migrations, and seed pairs.
- [x] Fetch provider changes for each enabled pair in dry-run mode.
- [x] Plan actions through `insync-core` in dry-run mode.
- [x] Load and pass stored provider sync tokens when fetching provider changes.
- [x] Clear expired provider sync tokens and retry as full sync.
- [x] Apply provider writes through injected providers only in apply mode.
- [x] Record event links, sync state, sync runs, and conflicts from provider
      apply orchestration.
- [x] Enable explicit live CLI `--apply`.
- [ ] Run and record at least one Rust `--apply` test against throwaway live
      calendars before recommending it for primary calendars.
- [x] Record scaffold sync runs and expose latest-run state for doctor/TUI.
- [x] Resolve stale conflicts after successful apply runs.
- [x] Add single-instance locking so two syncs cannot mutate the same DB.
- [x] Add structured sync summaries for CLI, TUI, reports, and future tray UI.

## 8. Dry-Run Reports

- [x] Recreate CSV dry-run reports in Rust.
- [x] Include action, reason, resolution, conflict policy, pair ID, provider IDs,
      timestamps, and hashes where useful.
- [x] Keep report rows on provider-backed dry-run plans and write CSV with the
      Bun-compatible column names.
- [x] Wire fixture-backed CLI dry-run planning and report writing.
- [x] Support `--report-all` for full snapshot/debug reports.
- [x] Add summary counts by pair, action, reason, and resolution.
- [x] Add report fixtures so output stays stable across refactors.

## 9. Conflict Tools

- [x] Port unresolved conflict summary command.
- [x] Port detailed conflict listing with filters.
- [x] Port conflict CSV export.
- [x] Port duplicate unresolved conflict cleanup.
- [x] Port stale conflict cleanup.
- [x] Add conflict inspection helpers for the TUI.
- [ ] Add future manual-resolution commands once we know the desired workflow.

## 10. Setup Flow

- [x] Implement `insync setup` in Rust.
- [x] Add initial `insync setup` config creation command.
- [x] Create config in local path or OS app-data path.
- [x] Add Google OAuth URL and auth-code exchange helpers.
- [x] Add noninteractive Google OAuth credential storage.
- [x] Guide through Google OAuth credentials interactively.
- [x] Run local callback server for Google OAuth.
- [x] Store refresh token in config or OS secret store based on config.
- [x] Add noninteractive iCloud username and app-specific password storage.
- [x] Guide through iCloud username and app-specific password interactively.
- [x] Discover calendars from both providers.
- [x] Add noninteractive calendar pair writing from discovered calendar IDs.
- [x] Let the user select and name calendar pairs interactively.
- [x] Run doctor checks at the end and show next safe command.

## 11. TUI

The TUI should be powerful, visually appealing, and genuinely useful. It should
feel like the main app for people who live in a terminal, not like logs with a
border.

- [x] Build the first polished dashboard shell with sync status, active pair
      count, unresolved conflicts, selected pair, and recent app message.
- [x] Connect dashboard next-run data once daemon scheduling is wired.
- [x] Connect dashboard last-run and recent-error data from sync runs.
- [x] Add the first calendar-pair screen with calendar IDs, direction, enabled
      state, and selection.
- [x] Add provider display names, calendar names, and last sync state once
      discovery and sync-state repositories are wired into the TUI.
- [x] Add a dry-run report viewer with sortable/filterable action rows.
- [x] Add a conflict workbench with grouped reasons, event detail comparison,
      selected resolution policy, and audit history.
- [ ] Add a setup wizard screen that mirrors CLI setup but feels guided.
- [x] Add a logs/sync-runs screen with severity filtering and run detail.
- [x] Add keyboard-first navigation with visible but compact command hints.
- [x] Add command palette style actions for sync, dry-run, apply, setup,
      conflicts, export report, and quit.
- [x] Add careful color semantics:
      neutral for idle, blue/cyan for running, green for clean, amber for
      warnings/conflicts, red for errors/destructive actions.
- [x] Add graceful empty, loading, success, warning, and error states.
- [x] Add responsive layouts for narrow terminals and wide terminals.
- [x] Add snapshot tests or golden rendering tests for key TUI screens where
      practical.
- [x] Keep the TUI connected to `insync-app` events/effects so a future
      taskbar/menu-bar shell can reuse the same control model.

## 12. Background Running

- [x] Implement daemon mode with interval scheduling.
- [x] Add graceful shutdown on Ctrl-C/signals.
- [x] Add retry/backoff and jitter for transient failures.
- [x] Add sync lock acquisition and lock expiry handling.
- [x] Add `launchd` support for macOS user agents.
- [x] Add `systemd --user` support for Linux.
- [x] Investigate Windows scheduled task/service support.
- [x] Add install/uninstall/status commands for background runners.
- [x] Add clear logs and health status for background mode.

## 13. Future Taskbar/Menu-Bar UI

- [x] Keep all app actions expressed through `insync-app` events/effects.
- [x] Define a small UI shell contract for status, actions, notifications, and
      setup entrypoints.
- [ ] Investigate macOS menu-bar implementation options.
- [ ] Investigate Windows tray implementation options.
- [ ] Investigate Linux tray/AppIndicator practicality.
- [x] Support notifications for failed syncs and unresolved conflicts.
- [x] Support safe quick actions: dry-run now, apply now, open conflicts, pause.

## 14. Packaging And Release

- [x] Add `cargo install --path rust/crates/insync-cli` instructions.
- [x] Add release builds for macOS, Linux, and Windows.
- [x] Decide binary name and package naming.
- [x] Add bundled/default config search order documentation.
- [x] Add migration docs from Bun to Rust.
- [x] Add GitHub Actions artifact builds.
- [ ] Add signed/notarized macOS builds if we ship a menu-bar app.

## 15. Cutover

- [ ] Run TypeScript and Rust dry-runs against the same calendars.
- [ ] Compare action counts, conflict counts, and CSV report rows.
- [ ] Run Rust apply against test calendars.
- [ ] Run Rust apply against real calendars after repeated clean dry-runs.
- [ ] Keep TypeScript fallback until Rust apply has proven stable.
- [ ] Archive or remove the TypeScript implementation after parity.

## Immediate Next Step

Port the planner and event hashing into `insync-core`, then make the Rust tests
match the TypeScript planner tests. That gives the migration a trustworthy spine
before provider writes or background scheduling enter the picture.
