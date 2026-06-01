export class ProviderNotConfiguredError extends Error {
  constructor(provider: string, detail: string) {
    super(`${provider} provider is not configured: ${detail}`);
    this.name = "ProviderNotConfiguredError";
  }
}

export class ProviderOperationNotImplementedError extends Error {
  constructor(provider: string, operation: string) {
    super(`${provider}.${operation} is not implemented yet`);
    this.name = "ProviderOperationNotImplementedError";
  }
}

export class ProviderHttpError extends Error {
  constructor(
    readonly provider: string,
    readonly status: number,
    readonly body: string
  ) {
    super(`${provider} request failed with ${status}: ${body}`);
    this.name = "ProviderHttpError";
  }
}

export class ProviderSyncTokenExpiredError extends Error {
  constructor(provider: string) {
    super(`${provider} sync token expired`);
    this.name = "ProviderSyncTokenExpiredError";
  }
}
