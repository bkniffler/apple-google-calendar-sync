import pino from "pino";
import type { ResolvedServiceConfig } from "./config/service-config";

export function createLogger(level: ResolvedServiceConfig["logLevel"] | "info" = "info") {
  if (level === "silent") {
    return pino({ level });
  }

  return pino({
    level,
    transport: {
      target: "pino-pretty",
      options: {
        colorize: true,
        translateTime: "SYS:standard",
        ignore: "pid,hostname"
      }
    }
  });
}
