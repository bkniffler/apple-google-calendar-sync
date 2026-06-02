import { loadEnv } from "./config/env";
import { resolveCredentials, storeGoogleRefreshToken, type ResolvedCredentials } from "./config/credentials";
import { loadServiceConfig } from "./config/service-config";
import { openDatabase } from "./db/database";
import { migrate } from "./db/migrations";
import { cacheProviderCalendars, seedConfiguredPairs } from "./db/repositories";
import { createLogger } from "./logger";
import { GoogleCalendarProvider, ICloudCalDavProvider } from "./providers";
import { exchangeGoogleAuthCode, googleAuthUrl } from "./providers/google/oauth";
import { SyncRunner } from "./sync/runner";
import type { Logger } from "pino";
import type { ResolvedServiceConfig } from "./config/service-config";

const env = loadEnv();
let logger = createLogger(env.INSYNC_LOG_LEVEL ?? "info");

type Command = "auth" | "calendars" | "doctor" | "migrate" | "setup" | "sync";

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

  if (command === "migrate") {
    await runMigrations();
    return;
  }

  if (command === "setup") {
    await setup();
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

function isCommand(value: string): value is Command {
  return (
    value === "auth" ||
    value === "calendars" ||
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

async function setup(): Promise<void> {
  const target = Bun.file(env.INSYNC_CONFIG);
  if (!(await target.exists())) {
    const example = await Bun.file("insync.example.json").text();
    await Bun.write(env.INSYNC_CONFIG, example);
    logger.info(
      { configPath: env.INSYNC_CONFIG },
      "created local config from insync.example.json"
    );
  }

  await doctor();
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

main().catch((error) => {
  logger.error({ error }, "command failed");
  process.exitCode = 1;
});
