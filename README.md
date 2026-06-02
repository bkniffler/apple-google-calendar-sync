# insync

Bun/TypeScript service for syncing selected iCloud calendars with selected Google calendars.

## Setup

```bash
bun install
bun run setup
```

`setup` creates `insync.local.json` from `insync.example.json` when it does not
exist. In an interactive terminal it can also guide you through credentials,
OAuth, calendar discovery, and pair selection. In noninteractive environments it
runs a config/database check.

Edit `insync.local.json` with your provider settings and calendar pair:

```json
{
  "secretStore": "none",
  "dbPath": ".insync/insync.db",
  "logLevel": "info",
  "google": {
    "accountLabel": "personal",
    "clientId": "",
    "clientSecret": "",
    "refreshToken": ""
  },
  "icloud": {
    "accountLabel": "personal",
    "username": "",
    "appSpecificPassword": "",
    "caldavUrl": "https://caldav.icloud.com"
  },
  "sync": {
    "pollIntervalSeconds": 300,
    "conflictPolicy": "manual",
    "conflicts": {
      "default": "manual",
      "bothSidesChanged": "manual",
      "unlinkedSameUid": "manual",
      "deleteVsUpdate": "update_wins",
      "icloudUidCollision": "ignore_known"
    },
    "pairs": [
      {
        "id": "personal",
        "enabled": true,
        "direction": "two_way",
        "googleCalendarId": "primary",
        "icloudCalendarId": "https://caldav.icloud.com/..."
      }
    ]
  }
}
```

`insync.local.json` is ignored by git. SQLite state, reports, event links, sync
cursors, and conflicts live under `.insync/` by default.

## Secret Store

Set `"secretStore": "none"` to keep credentials in `insync.local.json`.

Set `"secretStore": "os"` to use the OS secret store via `@napi-rs/keyring`.
If `clientSecret`, `refreshToken`, or `appSpecificPassword` are present in
config, insync moves them into the OS secret store and removes them from the
JSON file the next time you run a command.

The current OS backend uses the native platform store exposed by
`@napi-rs/keyring`, such as macOS Keychain, Linux Secret Service, and Windows
Credential Manager. Platform-specific native binaries are delivered through
the package's optional dependencies during install.

## Google Auth

1. Create a Google OAuth desktop/web client.
2. Add `http://127.0.0.1:53682/oauth2/callback` as a redirect URI.
3. Put `google.clientId` and `google.clientSecret` in `insync.local.json`.
4. Run:

```bash
bun run auth:google
```

The refresh token is written back to config or stored in the OS secret store,
depending on `secretStore`.

## Calendar Discovery

```bash
bun run calendars:google
bun run calendars:icloud
```

These commands print calendars and cache discovered calendar metadata in SQLite.
Copy the selected IDs into `sync.pairs`.

## Sync

Dry run:

```bash
bun run sync:dry
```

Apply writes:

```bash
bun run sync:apply
```

Watch mode:

```bash
bun run sync:watch
```

Dry runs write a CSV report to `.insync/reports/` by default. Reports include
actionable rows by default. Add `--report-all` to include snapshots:

```bash
bun src/index.ts sync --once --report-all
```

During the Rust migration, compare Bun and Rust dry-runs with:

```bash
bun run parity:dry-run -- --config ./insync.local.json
```

The comparison writes reports and `.insync/parity/comparison.json`, then exits
non-zero if action counts, reason counts, resolution counts, or normalized CSV
rows differ.

## Conflict Policies

`sync.conflicts` controls automatic conflict handling.

```json
{
  "default": "manual",
  "bothSidesChanged": "newest_updated_wins",
  "unlinkedSameUid": "manual",
  "deleteVsUpdate": "update_wins",
  "icloudUidCollision": "ignore_known"
}
```

Provider-winner policies:

- `manual`: report and record a conflict.
- `google_wins`: write the Google copy to iCloud.
- `icloud_wins`: write the iCloud copy to Google.
- `newest_updated_wins`: use provider update timestamps when both are known.

Delete-versus-update policies:

- `manual`: report and record a conflict.
- `update_wins`: recreate the deleted side from the changed side.
- `delete_wins`: delete the changed side too.

iCloud UID collision policies:

- `manual`: keep reporting the collision.
- `ignore_known`: record once, then suppress repeated write attempts.

CSV reports include `reason`, `resolution`, and `conflict_policy` columns so
auto-resolved and ignored conflicts remain auditable.

## Commands

```bash
bun run setup
bun run auth:google
bun run calendars:google
bun run calendars:icloud
bun run conflicts
bun run doctor
bun run migrate
bun run sync:dry
bun run sync:apply
bun run sync:watch
```

## Conflicts

Show unresolved conflict counts:

```bash
bun run conflicts
```

Show conflict details:

```bash
bun src/index.ts conflicts --details
bun src/index.ts conflicts --details --reason both_sides_changed
bun src/index.ts conflicts --details --pair personal --limit 50
```

Resolve duplicate unresolved records while keeping the first occurrence:

```bash
bun src/index.ts conflicts --dedupe
```

Write CSV output:

```bash
bun src/index.ts conflicts --csv .insync/reports/conflicts.csv
bun src/index.ts conflicts --details --csv .insync/reports/conflict-details.csv
```

## Current Sync Behavior

- Full-scan sync on each run for reliable delete detection.
- Two-way, Google-to-iCloud, and iCloud-to-Google directions.
- Dry-run by default; writes only happen with `--apply`.
- Conflict policies: manual, provider-winner, newest-updated-wins,
  delete/update-wins, and known iCloud UID collision ignore.
- Syncs normal events, all-day events, title, description, location, status,
  visibility, reminders, attendees, and basic recurrence lines.

## Caveats

- Attendee/invitation semantics are dangerous across providers; writes use
  `sendUpdates=none` on Google, but organizer-owned meetings can still have
  provider-specific constraints.
- Recurring exceptions are represented differently by Google and iCalendar.
  Basic recurrence is supported; complex edited instances need careful testing.
- iCloud can reject creates when the same UID already exists in another iCloud
  calendar. Insync records those as conflicts instead of repeatedly retrying.
