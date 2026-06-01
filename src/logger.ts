import pino from "pino";
import type { Env } from "./config/env";

export function createLogger(env: Env) {
  if (env.INSYNC_LOG_LEVEL === "silent") {
    return pino({ level: env.INSYNC_LOG_LEVEL });
  }

  return pino({
    level: env.INSYNC_LOG_LEVEL,
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
