import { writeFileSync } from "node:fs";
import type { Logger } from "pino";
import type { ResolvedServiceConfig } from "./service-config";
import {
  createSecretStore,
  googleClientSecretKey,
  googleRefreshTokenKey,
  icloudAppPasswordKey
} from "../secrets/secret-store";

export type ResolvedCredentials = {
  google: {
    clientId?: string | undefined;
    clientSecret?: string | undefined;
    refreshToken?: string | undefined;
  };
  icloud: {
    username?: string | undefined;
    appSpecificPassword?: string | undefined;
    caldavUrl: string;
  };
};

export async function resolveCredentials(input: {
  config: ResolvedServiceConfig;
  configPath: string;
  logger: Logger;
}): Promise<ResolvedCredentials> {
  const { config, configPath, logger } = input;

  if (config.secretStore === "none") {
    return {
      google: {
        clientId: config.google.clientId,
        clientSecret: config.google.clientSecret,
        refreshToken: config.google.refreshToken
      },
      icloud: {
        username: config.icloud.username,
        appSpecificPassword: config.icloud.appSpecificPassword,
        caldavUrl: config.icloud.caldavUrl
      }
    };
  }

  const store = createSecretStore(config.secretStore);
  let changed = false;

  if (config.google.clientSecret) {
    await store.set(googleClientSecretKey(config.google.accountLabel), config.google.clientSecret);
    config.google.clientSecret = undefined;
    changed = true;
  }

  if (config.google.refreshToken) {
    await store.set(googleRefreshTokenKey(config.google.accountLabel), config.google.refreshToken);
    config.google.refreshToken = undefined;
    changed = true;
  }

  if (config.icloud.appSpecificPassword) {
    await store.set(
      icloudAppPasswordKey(config.icloud.accountLabel),
      config.icloud.appSpecificPassword
    );
    config.icloud.appSpecificPassword = undefined;
    changed = true;
  }

  if (changed) {
    writeConfig(configPath, config);
    logger.warn(
      { configPath, secretStore: config.secretStore },
      "moved inline secrets to configured secret store"
    );
  }

  return {
    google: {
      clientId: config.google.clientId,
      clientSecret: await store.get(googleClientSecretKey(config.google.accountLabel)),
      refreshToken: await store.get(googleRefreshTokenKey(config.google.accountLabel))
    },
    icloud: {
      username: config.icloud.username,
      appSpecificPassword: await store.get(icloudAppPasswordKey(config.icloud.accountLabel)),
      caldavUrl: config.icloud.caldavUrl
    }
  };
}

export async function storeGoogleRefreshToken(input: {
  config: ResolvedServiceConfig;
  configPath: string;
  refreshToken: string;
}): Promise<void> {
  const { config, configPath, refreshToken } = input;

  if (config.secretStore === "os") {
    await createSecretStore("os").set(
      googleRefreshTokenKey(config.google.accountLabel),
      refreshToken
    );
    config.google.refreshToken = undefined;
  } else {
    config.google.refreshToken = refreshToken;
  }

  writeConfig(configPath, config);
}

function writeConfig(path: string, config: ResolvedServiceConfig): void {
  writeFileSync(path, `${JSON.stringify(config, null, 2)}\n`);
}
