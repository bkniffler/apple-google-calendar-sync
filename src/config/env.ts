import { z } from "zod";

const envSchema = z.object({
  INSYNC_DB_PATH: z.string().default(".insync/insync.db"),
  INSYNC_CONFIG: z.string().default("./insync.config.ts"),
  INSYNC_LOG_LEVEL: z
    .enum(["fatal", "error", "warn", "info", "debug", "trace", "silent"])
    .default("info"),
  GOOGLE_CLIENT_ID: z.string().optional(),
  GOOGLE_CLIENT_SECRET: z.string().optional(),
  GOOGLE_REFRESH_TOKEN: z.string().optional(),
  ICLOUD_USERNAME: z.string().optional(),
  ICLOUD_APP_SPECIFIC_PASSWORD: z.string().optional(),
  ICLOUD_CALDAV_URL: z.string().url().optional()
});

export type Env = z.infer<typeof envSchema>;

export function loadEnv(): Env {
  return envSchema.parse(process.env);
}
