import type { ServiceConfig } from "./src/config/service-config";

const config: ServiceConfig = {
  pollIntervalSeconds: 300,
  dryRun: true,
  conflictPolicy: "manual",
  pairs: [
    {
      id: "personal",
      enabled: true,
      direction: "two_way",
      google: {
        accountEmail: "you@gmail.com",
        calendarId: "primary"
      },
      icloud: {
        accountEmail: "you@icloud.com",
        calendarPath: "/calendars/example/"
      }
    }
  ]
};

export default config;
