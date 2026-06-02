import { AsyncEntry } from "@napi-rs/keyring";

export type SecretStoreKind = "none" | "os";

export interface SecretStore {
  readonly kind: SecretStoreKind;
  get(key: string): Promise<string | undefined>;
  set(key: string, value: string): Promise<void>;
  delete(key: string): Promise<void>;
}

export function createSecretStore(kind: SecretStoreKind): SecretStore {
  return kind === "os" ? new OsSecretStore() : new NoneSecretStore();
}

export function googleClientSecretKey(accountLabel: string): string {
  return `google/${accountLabel}/clientSecret`;
}

export function googleRefreshTokenKey(accountLabel: string): string {
  return `google/${accountLabel}/refreshToken`;
}

export function icloudAppPasswordKey(accountLabel: string): string {
  return `icloud/${accountLabel}/appSpecificPassword`;
}

class NoneSecretStore implements SecretStore {
  readonly kind = "none" as const;

  async get(): Promise<string | undefined> {
    return undefined;
  }

  async set(): Promise<void> {
    throw new Error('Config uses "secretStore": "none"; store this secret directly in config.');
  }

  async delete(): Promise<void> {
    return undefined;
  }
}

class OsSecretStore implements SecretStore {
  readonly kind = "os" as const;

  async get(key: string): Promise<string | undefined> {
    return normalizeMissingSecretError(() => entry(key).getPassword());
  }

  async set(key: string, value: string): Promise<void> {
    await entry(key).setPassword(value);
  }

  async delete(key: string): Promise<void> {
    await normalizeMissingSecretError(() => entry(key).deletePassword());
  }
}

function entry(key: string): AsyncEntry {
  return new AsyncEntry("insync", key);
}

async function normalizeMissingSecretError<T>(read: () => Promise<T>): Promise<T | undefined> {
  try {
    return await read();
  } catch (error) {
    if (error instanceof Error && /no entry|not found|no matching/i.test(error.message)) {
      return undefined;
    }
    throw error;
  }
}
