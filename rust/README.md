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
run detail. It also has a conflict inspection view (`c`) for unresolved conflict
groups and selected conflict rows. Calendar names are populated after setup discovery or
`insync db calendars` has cached provider metadata. Color semantics are kept
consistent: neutral idle states, blue/cyan active states, green clean states,
yellow warnings/conflicts, and red failures or destructive actions. Press `:`
inside the TUI to open the command palette for dry-run, apply, conflict refresh,
setup, view switching, report export, and quit actions. The key dashboard, run
history, and command-palette screens have buffer-level render tests to catch
layout regressions. The TUI drives commands through `insync-app` events/effects,
and `insync-app` exposes a shell snapshot with status, actions, and
notifications for future tray or menu-bar shells.

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
