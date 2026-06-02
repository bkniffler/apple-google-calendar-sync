import { loadEnv } from "./config/env";
import { resolveCredentials, storeGoogleRefreshToken, type ResolvedCredentials } from "./config/credentials";
import { loadServiceConfig } from "./config/service-config";
import { openDatabase } from "./db/database";
import { migrate } from "./db/migrations";
import {
  cacheProviderCalendars,
  dedupeUnresolvedConflicts,
  listUnresolvedConflicts,
  listUnresolvedConflictSummaries,
  seedConfiguredPairs
} from "./db/repositories";
import { createLogger } from "./logger";
import { GoogleCalendarProvider, ICloudCalDavProvider } from "./providers";
import { exchangeGoogleAuthCode, googleAuthUrl } from "./providers/google/oauth";
import { SyncRunner } from "./sync/runner";
import type { Logger } from "pino";
import type { ResolvedServiceConfig } from "./config/service-config";
import { mkdirSync, writeFileSync } from "node:fs";
import { dirname } from "node:path";
import { createInterface } from "node:readline/promises";

const env = loadEnv();
let logger = createLogger(env.INSYNC_LOG_LEVEL ?? "info");

type Command = "auth" | "calendars" | "conflicts" | "doctor" | "migrate" | "setup" | "sync";

type Runtime = {
  config: ResolvedServiceConfig;
  credentials: ResolvedCredentials;
  logger: Logger;
};

async function main(): Promise<void> {
  const [command = "doctor", ...args] = Bun.argv.slice(2);

  if (!isCommand(command)) {
    printUsage();
    process.exitCode = 1;
    return;
  }

  if (command === "auth") {
    await auth(args);
    return;
  }

  if (command === "calendars") {
    await calendars(args);
    return;
  }

  if (command === "doctor") {
    await doctor();
    return;
  }

  if (command === "conflicts") {
    await conflicts(args);
    return;
  }

  if (command === "migrate") {
    await runMigrations();
    return;
  }

  if (command === "setup") {
    await setup(args);
    return;
  }

  await sync(args);
}

async function doctor(): Promise<void> {
  const { config, credentials, logger } = await loadRuntime();
  const db = openDatabase(config.dbPath);
  migrate(db);
  seedConfiguredPairs(db, config);
  db.close();

  logger.info(
    {
      dbPath: config.dbPath,
      configPath: env.INSYNC_CONFIG,
      secretStore: config.secretStore,
      pairCount: config.sync.pairs.length,
      pollIntervalSeconds: config.sync.pollIntervalSeconds
    },
    "insync doctor passed"
  );

  if (!credentials.google.clientId || !credentials.google.clientSecret || !credentials.google.refreshToken) {
    logger.warn("google credentials are not configured yet");
  }

  if (!credentials.icloud.username || !credentials.icloud.appSpecificPassword) {
    logger.warn("icloud credentials are not configured yet");
  }
}

async function runMigrations(): Promise<void> {
  const { config, logger } = await loadRuntime();
  const db = openDatabase(config.dbPath);
  migrate(db);
  seedConfiguredPairs(db, config);
  db.close();
  logger.info({ dbPath: config.dbPath }, "database migrated");
}

async function sync(args: string[]): Promise<void> {
  const watch = args.includes("--watch");
  const once = args.includes("--once") || !watch;
  const apply = args.includes("--apply");
  const reportSnapshots = args.includes("--report-all");
  const { config, credentials, logger } = await loadRuntime();
  const dryRun = !apply;
  const reportPath = readFlagValue(args, "--report") ?? (dryRun ? defaultReportPath() : undefined);
  const db = openDatabase(config.dbPath);
  migrate(db);

  const runner = new SyncRunner({
    db,
    config,
    logger,
    dryRun,
    reportSnapshots,
    reportPath,
    google: new GoogleCalendarProvider({
      clientId: credentials.google.clientId,
      clientSecret: credentials.google.clientSecret,
      refreshToken: credentials.google.refreshToken
    }),
    icloud: new ICloudCalDavProvider({
      username: credentials.icloud.username,
      appSpecificPassword: credentials.icloud.appSpecificPassword,
      serverUrl: credentials.icloud.caldavUrl
    })
  });

  if (dryRun) {
    logger.warn(
      { reportPath },
      "sync is running in dry-run mode; pass --apply to write"
    );
  }

  if (once) {
    await runner.runOnce();
    db.close();
    return;
  }

  logger.info(
    { pollIntervalSeconds: config.sync.pollIntervalSeconds },
    "starting watch sync loop"
  );

  const runLoop = async () => {
    try {
      await runner.runOnce();
    } catch (error) {
      logger.error({ error }, "sync run failed");
    }
  };

  await runLoop();
  setInterval(runLoop, config.sync.pollIntervalSeconds * 1000);
}

async function conflicts(args: string[]): Promise<void> {
  const { config, logger } = await loadRuntime();
  const db = openDatabase(config.dbPath);
  migrate(db);

  const details = args.includes("--details");
  const reason = readFlagValue(args, "--reason");
  const pair = readFlagValue(args, "--pair");
  const csvPath = readFlagValue(args, "--csv");
  const limit = Number(readFlagValue(args, "--limit") ?? 100);

  if (args.includes("--dedupe")) {
    const resolved = dedupeUnresolvedConflicts(db);
    db.close();
    logger.info({ resolved }, "deduped unresolved conflicts");
    return;
  }

  if (details || reason || pair) {
    const rows = listUnresolvedConflicts(db, {
      reason,
      syncPairId: pair,
      limit: Number.isFinite(limit) ? limit : 100
    });
    db.close();

    if (csvPath) {
      writeCsv(csvPath, rows);
      logger.info({ csvPath, rows: rows.length }, "wrote conflict details");
      return;
    }

    console.table(rows);
    return;
  }

  const rows = listUnresolvedConflictSummaries(db);
  db.close();

  if (csvPath) {
    writeCsv(csvPath, rows);
    logger.info({ csvPath, rows: rows.length }, "wrote conflict summary");
    return;
  }

  console.table(rows);
}

function isCommand(value: string): value is Command {
  return (
    value === "auth" ||
    value === "calendars" ||
    value === "conflicts" ||
    value === "doctor" ||
    value === "migrate" ||
    value === "setup" ||
    value === "sync"
  );
}

function printUsage(): void {
  console.log(`Usage:
  bun src/index.ts setup
  bun src/index.ts auth google
  bun src/index.ts calendars google
  bun src/index.ts calendars icloud
  bun src/index.ts conflicts
  bun src/index.ts conflicts --details --reason both_sides_changed
  bun src/index.ts conflicts --csv .insync/reports/conflicts.csv
  bun src/index.ts doctor
  bun src/index.ts migrate
  bun src/index.ts sync --once
  bun src/index.ts sync --once --report .insync/reports/report.csv
  bun src/index.ts sync --once --report-all
  bun src/index.ts sync --once --apply
  bun src/index.ts sync --watch --apply`);
}

function readFlagValue(args: string[], flag: string): string | undefined {
  const exactIndex = args.indexOf(flag);
  if (exactIndex >= 0) {
    return args[exactIndex + 1];
  }

  const prefix = `${flag}=`;
  return args.find((arg) => arg.startsWith(prefix))?.slice(prefix.length);
}

function defaultReportPath(): string {
  const stamp = new Date().toISOString().replace(/[:.]/g, "-");
  return `.insync/reports/dry-run-${stamp}.csv`;
}

function writeCsv(path: string, rows: Array<Record<string, unknown>>): void {
  mkdirSync(dirname(path), { recursive: true });
  if (rows.length === 0) {
    writeFileSync(path, "\n");
    return;
  }

  const headers = Object.keys(rows[0] ?? {});
  const lines = [
    headers.join(","),
    ...rows.map((row) => headers.map((header) => csvEscape(String(row[header] ?? ""))).join(","))
  ];
  writeFileSync(path, `${lines.join("\n")}\n`);
}

function csvEscape(value: string): string {
  if (!/[",\n\r]/.test(value)) {
    return value;
  }

  return `"${value.replace(/"/g, '""')}"`;
}

async function auth(args: string[]): Promise<void> {
  const provider = args[0];
  if (provider !== "google") {
    throw new Error("Only `auth google` is supported; iCloud uses an app-specific password.");
  }

  const { config, credentials, logger } = await loadRuntime();
  if (!credentials.google.clientId || !credentials.google.clientSecret) {
    throw new Error("Set google.clientId and google.clientSecret before running auth.");
  }

  const port = Number(args.find((arg) => arg.startsWith("--port="))?.split("=")[1] ?? 53682);
  const redirectUri = `http://127.0.0.1:${port}/oauth2/callback`;
  const state = crypto.randomUUID();
  const url = googleAuthUrl({
    clientId: credentials.google.clientId,
    redirectUri,
    state
  });

  logger.info({ redirectUri }, "starting google oauth callback server");
  console.log(`Open this URL in your browser:\n\n${url}\n`);

  await new Promise<void>((resolve, reject) => {
    const server = Bun.serve({
      hostname: "127.0.0.1",
      port,
      async fetch(request) {
        const callbackUrl = new URL(request.url);
        if (callbackUrl.pathname !== "/oauth2/callback") {
          return new Response("Not found", { status: 404 });
        }

        try {
          if (callbackUrl.searchParams.get("state") !== state) {
            throw new Error("OAuth state mismatch.");
          }

          const code = callbackUrl.searchParams.get("code");
          if (!code) {
            throw new Error(callbackUrl.searchParams.get("error") ?? "Missing OAuth code.");
          }

          const token = await exchangeGoogleAuthCode({
            clientId: credentials.google.clientId as string,
            clientSecret: credentials.google.clientSecret as string,
            redirectUri,
            code
          });

          if (!token.refreshToken) {
            throw new Error("Google did not return a refresh token. Re-run with consent prompt.");
          }

          await storeGoogleRefreshToken({
            config,
            configPath: env.INSYNC_CONFIG,
            refreshToken: token.refreshToken
          });
          console.log(`\nGoogle refresh token stored via secretStore=${config.secretStore}.\n`);
          server.stop(true);
          resolve();
          return new Response("Google Calendar auth complete. You can close this tab.");
        } catch (error) {
          server.stop(true);
          reject(error);
          return new Response("Auth failed. Check the insync terminal.", { status: 500 });
        }
      }
    });
  });
}

async function calendars(args: string[]): Promise<void> {
  const provider = args[0] === "google" || args[0] === "icloud" ? args[0] : undefined;
  if (!provider) {
    throw new Error("Use `calendars google` or `calendars icloud`.");
  }

  const { config, credentials } = await loadRuntime();
  const client =
    provider === "google"
      ? new GoogleCalendarProvider({
          clientId: credentials.google.clientId,
          clientSecret: credentials.google.clientSecret,
          refreshToken: credentials.google.refreshToken
        })
      : provider === "icloud"
        ? new ICloudCalDavProvider({
            username: credentials.icloud.username,
            appSpecificPassword: credentials.icloud.appSpecificPassword,
            serverUrl: credentials.icloud.caldavUrl
          })
        : undefined;

  if (!client) {
    throw new Error("Calendar provider was not configured.");
  }

  const calendars = await client.listCalendars();
  const db = openDatabase(config.dbPath);
  migrate(db);
  cacheProviderCalendars(db, {
    provider,
    accountLabel: provider === "google" ? config.google.accountLabel : config.icloud.accountLabel,
    calendars
  });
  db.close();

  console.table(
    calendars.map((calendar) => ({
      id: calendar.id,
      name: calendar.name,
      timezone: calendar.timezone ?? "",
      writable: calendar.writable
    }))
  );
}

async function setup(args: string[]): Promise<void> {
  const target = Bun.file(env.INSYNC_CONFIG);
  if (!(await target.exists())) {
    const example = await Bun.file("insync.example.json").text();
    await Bun.write(env.INSYNC_CONFIG, example);
    logger.info(
      { configPath: env.INSYNC_CONFIG },
      "created local config from insync.example.json"
    );
  }

  if (!isInteractive() || args.includes("--check")) {
    await doctor();
    return;
  }

  const rl = createInterface({
    input: process.stdin,
    output: process.stdout
  });

  try {
    const config = await loadServiceConfig(env.INSYNC_CONFIG);
    const runGuide = await promptBoolean(
      rl,
      `Run guided setup for ${env.INSYNC_CONFIG}?`,
      true
    );
    if (!runGuide) {
      await doctor();
      return;
    }

    config.secretStore = await promptChoice(rl, "Secret store", ["none", "os"], config.secretStore);
    config.dbPath = await promptText(rl, "SQLite DB path", config.dbPath);
    config.logLevel = await promptChoice(
      rl,
      "Log level",
      ["info", "debug", "warn", "error", "silent"],
      config.logLevel
    );

    config.google.accountLabel = await promptText(
      rl,
      "Google account label",
      config.google.accountLabel
    );
    config.google.clientId = await promptText(
      rl,
      "Google client ID",
      config.google.clientId ?? ""
    );
    config.google.clientSecret = await promptText(
      rl,
      "Google client secret",
      config.google.clientSecret ?? ""
    );

    config.icloud.accountLabel = await promptText(
      rl,
      "iCloud account label",
      config.icloud.accountLabel
    );
    config.icloud.username = await promptText(
      rl,
      "iCloud username",
      config.icloud.username ?? ""
    );
    config.icloud.appSpecificPassword = await promptText(
      rl,
      "iCloud app-specific password",
      config.icloud.appSpecificPassword ?? ""
    );
    config.icloud.caldavUrl = await promptText(
      rl,
      "iCloud CalDAV URL",
      config.icloud.caldavUrl
    );

    writeConfig(config);

    if (!config.google.refreshToken && config.google.clientId && config.google.clientSecret) {
      const runAuth = await promptBoolean(rl, "Run Google OAuth now?", true);
      if (runAuth) {
        await auth(["google"]);
      }
    }

    const chooseCalendars = await promptBoolean(rl, "Discover and choose calendars now?", true);
    if (chooseCalendars) {
      await chooseCalendarPair(rl);
    }
  } finally {
    rl.close();
  }

  await doctor();
}

async function chooseCalendarPair(
  rl: ReturnType<typeof createInterface>
): Promise<void> {
  const { config, credentials } = await loadRuntime();
  const google = new GoogleCalendarProvider({
    clientId: credentials.google.clientId,
    clientSecret: credentials.google.clientSecret,
    refreshToken: credentials.google.refreshToken
  });
  const icloud = new ICloudCalDavProvider({
    username: credentials.icloud.username,
    appSpecificPassword: credentials.icloud.appSpecificPassword,
    serverUrl: credentials.icloud.caldavUrl
  });

  const [googleCalendars, icloudCalendars] = await Promise.all([
    google.listCalendars(),
    icloud.listCalendars()
  ]);

  const db = openDatabase(config.dbPath);
  migrate(db);
  cacheProviderCalendars(db, {
    provider: "google",
    accountLabel: config.google.accountLabel,
    calendars: googleCalendars
  });
  cacheProviderCalendars(db, {
    provider: "icloud",
    accountLabel: config.icloud.accountLabel,
    calendars: icloudCalendars
  });
  db.close();

  console.table(
    googleCalendars.map((calendar, index) => ({
      index,
      id: calendar.id,
      name: calendar.name,
      writable: calendar.writable
    }))
  );
  const googleIndex = await promptIndex(rl, "Google calendar index", googleCalendars.length, 0);

  console.table(
    icloudCalendars.map((calendar, index) => ({
      index,
      id: calendar.id,
      name: calendar.name,
      writable: calendar.writable
    }))
  );
  const icloudIndex = await promptIndex(rl, "iCloud calendar index", icloudCalendars.length, 0);

  const pairId = await promptText(rl, "Sync pair ID", config.sync.pairs[0]?.id ?? "personal");
  config.sync.pairs = [
    {
      id: pairId,
      enabled: true,
      direction: "two_way",
      googleCalendarId: googleCalendars[googleIndex]?.id ?? "primary",
      icloudCalendarId: icloudCalendars[icloudIndex]?.id ?? ""
    }
  ];
  writeConfig(config);
}

async function loadRuntime(): Promise<Runtime> {
  const config = await loadServiceConfig(env.INSYNC_CONFIG);
  logger = createLogger(env.INSYNC_LOG_LEVEL ?? config.logLevel);
  const credentials = await resolveCredentials({
    config,
    configPath: env.INSYNC_CONFIG,
    logger
  });
  return { config, credentials, logger };
}

function writeConfig(config: ResolvedServiceConfig): void {
  writeFileSync(env.INSYNC_CONFIG, `${JSON.stringify(config, null, 2)}\n`);
}

function isInteractive(): boolean {
  return Boolean(process.stdin.isTTY && process.stdout.isTTY);
}

async function promptText(
  rl: ReturnType<typeof createInterface>,
  question: string,
  current: string
): Promise<string> {
  const suffix = current ? ` [${current}]` : "";
  const answer = await rl.question(`${question}${suffix}: `);
  return answer.trim() || current;
}

async function promptBoolean(
  rl: ReturnType<typeof createInterface>,
  question: string,
  current: boolean
): Promise<boolean> {
  const answer = await rl.question(`${question} [${current ? "Y/n" : "y/N"}]: `);
  if (!answer.trim()) {
    return current;
  }
  return ["y", "yes", "true", "1"].includes(answer.trim().toLowerCase());
}

async function promptChoice<T extends string>(
  rl: ReturnType<typeof createInterface>,
  question: string,
  choices: readonly T[],
  current: T
): Promise<T> {
  const answer = await rl.question(`${question} (${choices.join("/")}) [${current}]: `);
  const value = answer.trim() as T;
  return choices.includes(value) ? value : current;
}

async function promptIndex(
  rl: ReturnType<typeof createInterface>,
  question: string,
  count: number,
  current: number
): Promise<number> {
  const answer = await rl.question(`${question} [${current}]: `);
  const value = Number(answer.trim());
  return Number.isInteger(value) && value >= 0 && value < count ? value : current;
}

main().catch((error) => {
  logger.error({ error }, "command failed");
  process.exitCode = 1;
});
