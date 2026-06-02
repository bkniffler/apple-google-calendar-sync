import { z } from "zod";

const envSchema = z.object({
  INSYNC_CONFIG: z.string().default("./insync.local.json"),
  INSYNC_LOG_LEVEL: z
    .enum(["fatal", "error", "warn", "info", "debug", "trace", "silent"])
    .optional()
});

export type Env = z.infer<typeof envSchema>;

export function loadEnv(): Env {
  return envSchema.parse(process.env);
}
