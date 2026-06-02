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

export const deleteConflictPolicySchema = z.enum([
  "manual",
  "delete_wins",
  "update_wins"
]);

export const uidCollisionPolicySchema = z.enum([
  "manual",
  "ignore_known"
]);

export const serviceConfigSchema = z.object({
  version: z.literal(1).default(1),
  secretStore: z.enum(["none", "os"]).default("none"),
  dbPath: z.string().default(".insync/insync.db"),
  logLevel: z
    .enum(["fatal", "error", "warn", "info", "debug", "trace", "silent"])
    .default("info"),
  google: z.object({
    accountLabel: z.string().min(1).default("default"),
    clientId: z.string().optional(),
    clientSecret: z.string().optional(),
    refreshToken: z.string().optional()
  }),
  icloud: z.object({
    accountLabel: z.string().min(1).default("default"),
    username: z.string().optional(),
    appSpecificPassword: z.string().optional(),
    caldavUrl: z.string().url().default("https://caldav.icloud.com")
  }),
  sync: z.object({
    pollIntervalSeconds: z.number().int().min(30).default(300),
    conflictPolicy: conflictPolicySchema.default("manual"),
    conflicts: z
      .object({
        default: conflictPolicySchema.default("manual"),
        bothSidesChanged: conflictPolicySchema.optional(),
        unlinkedSameUid: conflictPolicySchema.optional(),
        deleteVsUpdate: deleteConflictPolicySchema.default("update_wins"),
        icloudUidCollision: uidCollisionPolicySchema.default("ignore_known")
      })
      .default({
        default: "manual",
        deleteVsUpdate: "update_wins",
        icloudUidCollision: "ignore_known"
      }),
    pairs: z
      .array(
        z.object({
          id: z.string().min(1),
          enabled: z.boolean().default(true),
          direction: directionSchema.default("two_way"),
          googleCalendarId: z.string().min(1),
          icloudCalendarId: z.string().min(1)
        })
      )
      .default([])
  })
});

export type SyncDirection = z.infer<typeof directionSchema>;
export type ConflictPolicy = z.infer<typeof conflictPolicySchema>;
export type DeleteConflictPolicy = z.infer<typeof deleteConflictPolicySchema>;
export type UidCollisionPolicy = z.infer<typeof uidCollisionPolicySchema>;
export type ServiceConfig = z.input<typeof serviceConfigSchema>;
export type ResolvedServiceConfig = z.infer<typeof serviceConfigSchema>;

export async function loadServiceConfig(path: string): Promise<ResolvedServiceConfig> {
  const file = Bun.file(path);
  if (!(await file.exists())) {
    throw new Error(`Config file not found: ${path}. Run \`bun run setup\` or copy insync.example.json.`);
  }

  return serviceConfigSchema.parse(await file.json());
}
