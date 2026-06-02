# insync Rust workspace

This folder is the Rust rewrite path for insync. The TypeScript/Bun service stays
usable while the Rust implementation is ported behind stable crate boundaries.

## Shape

```text
rust/
  crates/
    insync-core/       Domain types, planner, conflict policy, pure tests.
    insync-config/     JSON config, app-data paths, secret-store selection.
    insync-db/         SQLite migrations and repositories.
    insync-providers/  Provider traits plus Google/CalDAV adapter boundaries.
    insync-engine/     Sync orchestration, scheduler, daemon loop.
    insync-app/        UI/control-plane state machine, Crux-compatible boundary.
    insync-cli/        CLI, TUI, daemon commands, local shell effects.
```

## Crux fit

Crux makes sense for the app/control plane, not for the sync engine itself.
The engine should remain ordinary Rust so it can run from a daemon, CLI command,
test harness, launch agent, or future tray/taskbar shell. The `insync-app` crate
keeps an Elm/Crux-shaped model-update-effect boundary so a TUI shell and a tray
shell can drive the same app behavior.

The first Rust target is therefore:

1. Port pure planner and canonical event model to `insync-core`.
2. Port JSON config and secret resolution to `insync-config`.
3. Port SQLite schema/repositories to `insync-db`.
4. Port providers one at a time behind `insync-providers`.
5. Replace the TS runner with `insync-engine`.
6. Build richer shells on top: `insync tui`, `insync daemon`, then tray/taskbar.

Provider field preservation and known lossy mappings are documented in
[`PROVIDER_MAPPING.md`](PROVIDER_MAPPING.md).

## Commands

```bash
cd rust
cargo test
cargo run -p insync-cli -- --help
cargo run -p insync-cli -- doctor
cargo run -p insync-cli -- tui
```

## Local Install

From this repository:

```bash
cd rust
cargo install --path crates/insync-cli
insync --help
```

During the migration, the Rust workflow is intentionally separate from the
Bun/TypeScript app. CI runs `cargo fmt --all -- --check` and
`cargo test --workspace --locked` for changes under `rust/`.

The release-artifact workflow builds the `insync` CLI binary for Linux, macOS,
and Windows on manual dispatch or `v*` tags. These are unsigned development
artifacts until the final packaging path is decided.

Future tray/menu-bar notes live in [`TRAY_UI.md`](TRAY_UI.md). The current
recommendation is to add a separate `insync-tray` shell after Rust live parity,
using Tauri v2 for a richer setup/conflict/report window or `tray-icon`
directly only for a menu-only shell.

## Naming

The user-facing binary name is `insync`. The Rust crate that owns that binary is
`insync-cli`, so the workspace can keep separate library crates for the app
model, engine, providers, database, config, and core planner without changing
the command people run.

Release archives and CI artifacts should use the binary name plus target label,
for example `insync-macos-aarch64`, `insync-linux-x86_64`, and
`insync-windows-x86_64.exe`. Future tray or menu-bar shells should use a
separate package/app name, but should keep driving the same `insync-app`
contract and `insync-engine` crates.

## Setup

Create a starter config:

```bash
insync setup
insync setup --interactive
insync setup --location app
insync --config /path/to/insync.json setup
```

By default setup writes `insync.local.json` and uses `"secretStore": "os"` so
future credentials go to the OS secret store.

Config loading normalizes legacy/missing versions to the current schema version
and rejects future schema versions before validation so old binaries fail
clearly.

`--interactive` prompts for Google and iCloud credentials, can run the Google
OAuth callback, can discover calendars, can write a pair, and finishes with a
doctor summary plus the next safe dry-run command.

Store provider account metadata and secrets:

```bash
insync setup \
  --google-client-id ... \
  --google-client-secret ...

insync setup \
  --icloud-username you@example.com \
  --icloud-app-password xxxx-xxxx-xxxx-xxxx
```

`--google-client-secret` and `--icloud-app-password` are stored according to
`secretStore`; with `"os"` they are written to the OS secret store and removed
from the JSON config.

Generate a Google OAuth URL and exchange the returned code:

```bash
insync setup --google-callback
insync setup --google-auth-url
insync setup --google-code 4/returned-code
```

The default redirect URI is `http://127.0.0.1:8787/oauth2/callback`; pass
`--redirect-uri` if your Google OAuth client uses a different one. The exchanged
refresh token is stored according to `secretStore`. `--google-callback` starts a
single-use loopback callback server, prints the consent URL, waits for Google to
redirect back, and then stores the returned refresh token.

After credentials are configured, discover calendars for pairing:

```bash
insync setup --discover
insync setup --discover --csv .insync/reports/calendars.csv
```

Discovery prints provider, writable state, display name, timezone, and calendar
ID. It does not modify config yet.

Add or replace a calendar pair:

```bash
insync setup \
  --pair-id personal \
  --google-calendar-id primary \
  --icloud-calendar-id https://caldav.icloud.com/.../

insync setup \
  --pair-id work \
  --google-calendar-id work@example.com \
  --icloud-calendar-id https://caldav.icloud.com/.../ \
  --direction google-to-icloud
```

Use `--force` to replace an existing pair with the same ID. Directions are
`two-way`, `google-to-icloud`, and `icloud-to-google`.

## Terminal Dashboard

```bash
insync tui
```

The TUI shows sync health, unresolved conflicts, recent run status, configured
pairs, cached provider calendar names/accounts, raw calendar IDs, per-side last
sync timestamps from SQLite, and a filterable recent-run history with selected
run detail. It also has a setup wizard (`s`) with readiness checks and next
actions, a conflict workbench (`c`) for unresolved conflict groups, stored
Google/iCloud snapshot comparison, selected policy context, and unresolved audit
timestamps; a dry-run report viewer (`v`) with action filters (`f`) and sort
modes (`t`); plus a notification band for failed syncs, unresolved conflicts,
and setup nudges. Calendar names are populated after setup discovery or
`insync db calendars` has cached provider metadata. Color semantics are kept
consistent: neutral idle states, blue/cyan active states, green clean states,
yellow warnings/conflicts, and red failures or destructive actions.
Press `:` inside the TUI to open the command palette for dry-run, apply,
conflict refresh, setup, view switching, report export, and quit actions. The
same shell actions are exposed as safe quick actions, including immediate
dry-run/apply, conflict inspection, and background pause/resume (`b`). The key
dashboard, run history, report, conflict, notification, and command-palette
screens have buffer-level render tests to catch layout regressions. The TUI
drives commands through `insync-app` events/effects, and `insync-app` exposes a
shell snapshot with status, actions, and notifications for future tray or
menu-bar shells.

## Sync

```bash
insync sync
insync sync --report .insync/reports/dry-run.csv
insync sync --summary-json .insync/reports/sync-summary.json
insync sync --report-all --report .insync/reports/full-dry-run.csv
insync sync --apply
```

`insync sync` is a live dry-run by default: it fetches both providers, plans the
work, updates no remote events, and can write CSV and JSON reports. `--apply`
executes the planned provider writes, records event links and sync state in
SQLite, and resolves stale manual conflicts after a successful run.

Manual conflicts can be inspected and queued for explicit resolution:

```bash
insync conflicts --details
insync conflicts --resolve <conflict-id> --resolution google-wins
insync conflicts --resolve <conflict-id> --resolution icloud-wins
insync conflicts --resolve <conflict-id> --resolution delete-wins
insync conflicts --resolve <conflict-id> --resolution update-wins
insync sync --apply
```

Queued resolutions are stored in SQLite and consumed by the next apply run.

For repeated background runs:

```bash
insync daemon
insync daemon --apply
```

Daemon mode uses `sync.pollIntervalSeconds`, stops cleanly on Ctrl-C, and keeps
running after failed ticks with capped retry/backoff plus jitter.

To install it as a user background runner:

```bash
insync background install --apply
insync background status
insync background uninstall
```

On macOS, `background install` writes a launchd user agent to
`~/Library/LaunchAgents/dev.bkniffler.insync.plist` and logs to
`~/Library/Logs/insync`. On Linux, it writes a `systemd --user` unit to
`$XDG_CONFIG_HOME/systemd/user/insync.service` or
`~/.config/systemd/user/insync.service`; logs are available through
`journalctl --user -u insync.service`. The installer pins the validated config
path with `--config` so the service does not depend on a working directory or
shell environment. Use `--force` to replace an existing service definition.

You can inspect the generated files without installing:

```bash
insync background print --template launchd --apply
insync background print --template systemd --apply
```

Windows background service support is not implemented yet. For now, use Windows
Task Scheduler to run `insync daemon --apply` after installing the binary.

## Database Maintenance

```bash
insync db calendars
insync db backup .insync/backups/insync.db
insync db export .insync/backups/insync-support.json
insync db import .insync/backups/insync-support.json --to .insync/restored.db
```

`db backup` creates a compact SQLite copy using `VACUUM INTO`. `db export`
writes a JSON dump of the known support tables for inspection or support
handoff, and `db import` rebuilds a migrated SQLite database from that dump.
Commands refuse to overwrite existing files unless `--force` is passed.

## Conflict Maintenance

```bash
insync conflicts
insync conflicts --details --reason both_sides_changed
insync conflicts --dedupe
insync conflicts --resolve-stale
```

`--resolve-stale` fetches the current provider state, plans current manual
conflicts, and only marks SQLite conflict rows resolved when those conflicts are
no longer active. It does not write calendar events.

## Config Search Order

Every Rust command resolves config in the same order:

1. `--config /path/to/insync.json`
2. `INSYNC_CONFIG=/path/to/insync.json`
3. `insync.local.json` in the current working directory, when present
4. the OS app config path:
   - macOS: `~/Library/Application Support/dev.bkniffler.insync/insync.json`
   - Linux: `$XDG_CONFIG_HOME/insync/insync.json` or `~/.config/insync/insync.json`
   - Windows: `%APPDATA%\bkniffler\insync\config\insync.json`

Relative `dbPath` values are resolved relative to the config file directory, so
a portable config can keep its SQLite cache beside the config file:

```json
{
  "dbPath": ".insync/insync.db",
  "secretStore": "os",
  "google": { "accountLabel": "personal" },
  "icloud": { "accountLabel": "personal" },
  "sync": { "pairs": [] }
}
```

The CLI validates non-secret config shape before starting: version, database
path, log level, account labels, CalDAV URL shape, poll interval, unique pair
IDs, and non-empty calendar IDs. Credentials can still live inline or in the OS
secret store.

## Migrating From Bun

The Rust config intentionally keeps the same JSON shape as the Bun service:
`secretStore`, `dbPath`, provider account labels, credentials, conflict policy,
and `sync.pairs`. In most cases you can start from the same
`insync.local.json`.

Recommended migration:

1. Keep the Bun app installed and working until Rust dry-runs and reports match
   your expectations.
2. Back up your current SQLite state before the Rust binary touches it:

   ```bash
   cp .insync/insync.db .insync/insync.before-rust.db
   ```

3. Install the Rust binary:

   ```bash
   cd rust
   cargo install --path crates/insync-cli
   ```

4. Run Rust doctor against the same config:

   ```bash
   insync --config ../insync.local.json doctor
   ```

5. Run the TypeScript/Rust dry-run parity harness:

   ```bash
   cd ..
   bun run parity:dry-run -- --config ./insync.local.json
   ```

   The harness writes TypeScript and Rust reports plus a comparison summary to
   `.insync/parity/`. It exits non-zero when row counts, action counts, reason
   counts, resolution counts, or normalized common CSV rows differ.

   To compare existing CSVs without fetching providers again:

   ```bash
   bun scripts/compare-dry-runs.ts \
     --skip-run \
     --ts-report .insync/reports/typescript-dry-run.csv \
     --rust-report .insync/reports/rust-dry-run.csv
   ```

6. You can also run a Rust dry-run directly and write reports:

   ```bash
   cd rust
   insync --config ../insync.local.json sync \
     --report ../.insync/reports/rust-dry-run.csv \
     --summary-json ../.insync/reports/rust-summary.json
   ```

7. Compare the Rust report/counts with the Bun dry-run report before enabling
   `--apply`. Expect harmless ordering differences, but pair counts, conflict
   counts, and actionable rows should make sense.

8. Only after repeated clean dry-runs, test Rust apply on throwaway calendars
   before pointing it at primary calendars:

   ```bash
   insync --config ../insync.local.json sync --apply
   ```

Notes:

- `secretStore: "os"` is supported in both implementations, but the Rust binary
  uses the Rust `keyring` crate. If secrets are still inline, Rust can move
  them into the OS store on setup/credential writes.
- Relative `dbPath` values are resolved relative to the config file location in
  Rust. Keep this in mind if you pass a config outside the repository.
- Do not run Bun watch mode and Rust daemon/apply mode against the same
  calendars at the same time. Use one writer at a time.
