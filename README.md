# insync

Bun/TypeScript service for syncing selected iCloud calendars with selected Google calendars.

## Current shape

This is a structured service scaffold with:

- Bun + TypeScript
- SQLite via `bun:sqlite`
- typed config loading
- migration runner
- provider boundaries for Google Calendar and iCloud CalDAV
- sync planner/runner skeleton
- CLI commands for doctor, migrate, one-shot sync, and watch mode

## Setup

```bash
bun install
cp .env.example .env
bun run migrate
bun run doctor
```

Edit `insync.config.ts` to define calendar pairs.

For Google:

1. Create an OAuth desktop/web client in Google Cloud.
2. Add `http://127.0.0.1:53682/oauth2/callback` as a redirect URI.
3. Set `GOOGLE_CLIENT_ID` and `GOOGLE_CLIENT_SECRET` in `.env`.
4. Run `bun run auth:google`.
5. Put the printed `GOOGLE_REFRESH_TOKEN` into `.env`.

For iCloud:

1. Create an Apple app-specific password.
2. Set `ICLOUD_USERNAME` and `ICLOUD_APP_SPECIFIC_PASSWORD` in `.env`.
3. Run `bun run calendars:icloud` and copy the calendar URL/path into `insync.config.ts`.

## Commands

```bash
bun run auth:google
bun run calendars:google
bun run calendars:icloud
bun run doctor
bun run migrate
bun run sync
bun run sync:watch
```

`sync` runs as a dry-run by default. To write changes, set `dryRun: false`
in `insync.config.ts`, then run:

```bash
bun src/index.ts sync --once --apply
```

## Current Sync Behavior

- Full-scan sync on each run for reliable delete detection.
- Two-way, Google-to-iCloud, and iCloud-to-Google directions.
- Dry-run by default.
- Conflict policies: manual, google_wins, icloud_wins, newest_updated_wins.
- Syncs normal events, all-day events, title, description, location, status,
  visibility, reminders, attendees, and basic recurrence lines.

## Caveats

- Attendee/invitation semantics are dangerous across providers; writes use
  `sendUpdates=none` on Google, but organizer-owned meetings can still have
  provider-specific constraints.
- Recurring exceptions are represented differently by Google and iCalendar.
  Basic recurrence is supported; complex edited instances need careful testing.
- iCloud is scanned via CalDAV objects. Google incremental tokens are captured
  but not used yet because the first working engine favors full-state safety.

## Notes

The first implementation keeps provider calls behind adapters. That is deliberate: the sync ledger, conflict handling, and event identity model need to be stable before wiring destructive writes to live calendars.
