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

## Commands

```bash
cd rust
cargo test
cargo run -p insync-cli -- --help
cargo run -p insync-cli -- doctor
cargo run -p insync-cli -- tui
```

The Rust code defaults to `insync.local.json` in the repo when present, and can
later move to the OS app config folder without changing the core sync crates.
