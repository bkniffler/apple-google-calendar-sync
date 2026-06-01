import { loadEnv } from "./config/env";
import { loadServiceConfig } from "./config/service-config";
import { openDatabase } from "./db/database";
import { migrate } from "./db/migrations";
import { seedConfiguredPairs } from "./db/repositories";
import { createLogger } from "./logger";
import { GoogleCalendarProvider, ICloudCalDavProvider } from "./providers";
import { exchangeGoogleAuthCode, googleAuthUrl } from "./providers/google/oauth";
import { SyncRunner } from "./sync/runner";

const env = loadEnv();
const logger = createLogger(env);

type Command = "auth" | "calendars" | "doctor" | "migrate" | "sync";

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

  await sync(args);
}

async function doctor(): Promise<void> {
  const config = await loadServiceConfig(env.INSYNC_CONFIG);
  const db = openDatabase(env.INSYNC_DB_PATH);
  migrate(db);
  seedConfiguredPairs(db, config);
  db.close();

  logger.info(
    {
      dbPath: env.INSYNC_DB_PATH,
      configPath: env.INSYNC_CONFIG,
      pairCount: config.pairs.length,
      pollIntervalSeconds: config.pollIntervalSeconds
    },
    "insync doctor passed"
  );

  if (!env.GOOGLE_CLIENT_ID || !env.GOOGLE_CLIENT_SECRET || !env.GOOGLE_REFRESH_TOKEN) {
    logger.warn("google credentials are not configured yet");
  }

  if (!env.ICLOUD_USERNAME || !env.ICLOUD_APP_SPECIFIC_PASSWORD) {
    logger.warn("icloud credentials are not configured yet");
  }
}

async function runMigrations(): Promise<void> {
  const config = await loadServiceConfig(env.INSYNC_CONFIG);
  const db = openDatabase(env.INSYNC_DB_PATH);
  migrate(db);
  seedConfiguredPairs(db, config);
  db.close();
  logger.info({ dbPath: env.INSYNC_DB_PATH }, "database migrated");
}

async function sync(args: string[]): Promise<void> {
  const watch = args.includes("--watch");
  const once = args.includes("--once") || !watch;
  const apply = args.includes("--apply");
  const config = await loadServiceConfig(env.INSYNC_CONFIG);
  const dryRun = !apply || config.dryRun;
  const reportPath = readFlagValue(args, "--report") ?? (dryRun ? defaultReportPath() : undefined);
  const db = openDatabase(env.INSYNC_DB_PATH);
  migrate(db);

  const runner = new SyncRunner({
    db,
    config,
    logger,
    dryRun,
    reportPath,
    google: new GoogleCalendarProvider({
      clientId: env.GOOGLE_CLIENT_ID,
      clientSecret: env.GOOGLE_CLIENT_SECRET,
      refreshToken: env.GOOGLE_REFRESH_TOKEN
    }),
    icloud: new ICloudCalDavProvider({
      username: env.ICLOUD_USERNAME,
      appSpecificPassword: env.ICLOUD_APP_SPECIFIC_PASSWORD,
      serverUrl: env.ICLOUD_CALDAV_URL
    })
  });

  if (dryRun) {
    logger.warn(
      { reportPath },
      "sync is running in dry-run mode; pass --apply and set dryRun=false in config to write"
    );
  }

  if (once) {
    await runner.runOnce();
    db.close();
    return;
  }

  logger.info(
    { pollIntervalSeconds: config.pollIntervalSeconds },
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
  setInterval(runLoop, config.pollIntervalSeconds * 1000);
}

function isCommand(value: string): value is Command {
  return (
    value === "auth" ||
    value === "calendars" ||
    value === "doctor" ||
    value === "migrate" ||
    value === "sync"
  );
}

function printUsage(): void {
  console.log(`Usage:
  bun src/index.ts auth google
  bun src/index.ts calendars google
  bun src/index.ts calendars icloud
  bun src/index.ts doctor
  bun src/index.ts migrate
  bun src/index.ts sync --once
  bun src/index.ts sync --once --report .insync/reports/report.csv
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

  if (!env.GOOGLE_CLIENT_ID || !env.GOOGLE_CLIENT_SECRET) {
    throw new Error("Set GOOGLE_CLIENT_ID and GOOGLE_CLIENT_SECRET before running auth.");
  }

  const port = Number(args.find((arg) => arg.startsWith("--port="))?.split("=")[1] ?? 53682);
  const redirectUri = `http://127.0.0.1:${port}/oauth2/callback`;
  const state = crypto.randomUUID();
  const url = googleAuthUrl({
    clientId: env.GOOGLE_CLIENT_ID,
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
            clientId: env.GOOGLE_CLIENT_ID as string,
            clientSecret: env.GOOGLE_CLIENT_SECRET as string,
            redirectUri,
            code
          });

          if (!token.refreshToken) {
            throw new Error("Google did not return a refresh token. Re-run with consent prompt.");
          }

          console.log(`\nAdd this to .env:\n\nGOOGLE_REFRESH_TOKEN=${token.refreshToken}\n`);
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
  const provider = args[0];
  const client =
    provider === "google"
      ? new GoogleCalendarProvider({
          clientId: env.GOOGLE_CLIENT_ID,
          clientSecret: env.GOOGLE_CLIENT_SECRET,
          refreshToken: env.GOOGLE_REFRESH_TOKEN
        })
      : provider === "icloud"
        ? new ICloudCalDavProvider({
            username: env.ICLOUD_USERNAME,
            appSpecificPassword: env.ICLOUD_APP_SPECIFIC_PASSWORD,
            serverUrl: env.ICLOUD_CALDAV_URL
          })
        : undefined;

  if (!client) {
    throw new Error("Use `calendars google` or `calendars icloud`.");
  }

  const calendars = await client.listCalendars();
  console.table(
    calendars.map((calendar) => ({
      id: calendar.id,
      name: calendar.name,
      timezone: calendar.timezone ?? "",
      writable: calendar.writable
    }))
  );
}

main().catch((error) => {
  logger.error({ error }, "command failed");
  process.exitCode = 1;
});
