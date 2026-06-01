import { z } from "zod";

export const directionSchema = z.enum([
  "two_way",
  "left_to_right",
  "right_to_left"
]);

export const conflictPolicySchema = z.enum([
  "manual",
  "google_wins",
  "icloud_wins",
  "newest_updated_wins"
]);

export const serviceConfigSchema = z.object({
  pollIntervalSeconds: z.number().int().min(30).default(300),
  dryRun: z.boolean().default(true),
  conflictPolicy: conflictPolicySchema.default("manual"),
  pairs: z
    .array(
      z.object({
        id: z.string().min(1),
        enabled: z.boolean().default(true),
        direction: directionSchema.default("two_way"),
        google: z.object({
          accountEmail: z.string().email(),
          calendarId: z.string().min(1)
        }),
        icloud: z.object({
          accountEmail: z.string().email(),
          calendarPath: z.string().min(1)
        })
      })
    )
    .default([])
});

export type SyncDirection = z.infer<typeof directionSchema>;
export type ConflictPolicy = z.infer<typeof conflictPolicySchema>;
export type ServiceConfig = z.input<typeof serviceConfigSchema>;
export type ResolvedServiceConfig = z.infer<typeof serviceConfigSchema>;

export async function loadServiceConfig(path: string): Promise<ResolvedServiceConfig> {
  const modulePath = path.startsWith(".") ? `${process.cwd()}/${path}` : path;
  const imported = await import(modulePath);
  return serviceConfigSchema.parse(imported.default);
}
