import {
  ProviderHttpError,
  ProviderNotConfiguredError,
  ProviderSyncTokenExpiredError
} from "../errors";
import type {
  CalendarProvider,
  CanonicalEvent,
  ProviderCalendar,
  ProviderChangeSet,
  ProviderEventMeta,
  ProviderSyncCursor
} from "../types";
import { canonicalToGoogle, googleMetaFromResponse, googleToCanonical } from "./google-mapper";
import type {
  GoogleCalendarListResponse,
  GoogleEvent,
  GoogleEventsResponse
} from "./google-types";

export type GoogleProviderOptions = {
  clientId?: string | undefined;
  clientSecret?: string | undefined;
  refreshToken?: string | undefined;
};

export class GoogleCalendarProvider implements CalendarProvider {
  readonly name = "google" as const;
  private accessToken?: string;
  private accessTokenExpiresAt = 0;

  constructor(private readonly options: GoogleProviderOptions = {}) {}

  async listCalendars(): Promise<ProviderCalendar[]> {
    this.assertConfigured();
    const calendars: ProviderCalendar[] = [];
    let pageToken: string | undefined;

    do {
      const response = await this.request<GoogleCalendarListResponse>(
        "/users/me/calendarList",
        { pageToken }
      );

      for (const item of response.items ?? []) {
        calendars.push({
          id: item.id,
          name: item.summary ?? item.id,
          timezone: item.timeZone,
          writable: item.accessRole === "owner" || item.accessRole === "writer",
          raw: item
        });
      }

      pageToken = response.nextPageToken;
    } while (pageToken);

    return calendars;
  }

  async getChanges(
    calendarId: string,
    cursor: ProviderSyncCursor
  ): Promise<ProviderChangeSet> {
    this.assertConfigured();
    const events: CanonicalEvent[] = [];
    let pageToken: string | undefined;
    let nextSyncToken: string | undefined;

    do {
      const response = await this.request<GoogleEventsResponse>(
        `/calendars/${encodeURIComponent(calendarId)}/events`,
        {
          maxResults: "2500",
          pageToken,
          showDeleted: "true",
          singleEvents: "false",
          syncToken: cursor.fullSync ? undefined : cursor.syncToken
        }
      );

      for (const item of response.items ?? []) {
        events.push(googleToCanonical(calendarId, item));
      }

      pageToken = response.nextPageToken;
      nextSyncToken = response.nextSyncToken ?? nextSyncToken;
    } while (pageToken);

    return {
      provider: this.name,
      calendarId,
      syncToken: nextSyncToken,
      events
    };
  }

  async createEvent(
    calendarId: string,
    event: CanonicalEvent
  ): Promise<ProviderEventMeta> {
    this.assertConfigured();
    const created = await this.request<GoogleEvent>(
      `/calendars/${encodeURIComponent(calendarId)}/events`,
      { sendUpdates: "none" },
      {
        method: "POST",
        body: JSON.stringify(canonicalToGoogle(event, "icloud"))
      }
    );

    return googleMetaFromResponse(calendarId, created);
  }

  async updateEvent(
    calendarId: string,
    remoteEventId: string,
    event: CanonicalEvent,
    etag?: string
  ): Promise<ProviderEventMeta> {
    this.assertConfigured();
    const init: RequestInit = {
      method: "PATCH",
      body: JSON.stringify(canonicalToGoogle(event, "icloud"))
    };
    if (etag) {
      init.headers = { "If-Match": etag };
    }

    const updated = await this.request<GoogleEvent>(
      `/calendars/${encodeURIComponent(calendarId)}/events/${encodeURIComponent(remoteEventId)}`,
      { sendUpdates: "none" },
      init
    );

    return googleMetaFromResponse(calendarId, updated);
  }

  async deleteEvent(
    calendarId: string,
    remoteEventId: string,
    etag?: string
  ): Promise<void> {
    this.assertConfigured();
    const init: RequestInit = { method: "DELETE" };
    if (etag) {
      init.headers = { "If-Match": etag };
    }

    await this.request(
      `/calendars/${encodeURIComponent(calendarId)}/events/${encodeURIComponent(remoteEventId)}`,
      { sendUpdates: "none" },
      init
    );
  }

  private assertConfigured(): void {
    if (!this.options.clientId || !this.options.clientSecret || !this.options.refreshToken) {
      throw new ProviderNotConfiguredError(
        this.name,
        "set GOOGLE_CLIENT_ID, GOOGLE_CLIENT_SECRET, and GOOGLE_REFRESH_TOKEN"
      );
    }
  }

  private async request<T = unknown>(
    path: string,
    query: Record<string, string | undefined> = {},
    init: RequestInit = {}
  ): Promise<T> {
    const accessToken = await this.getAccessToken();
    const url = new URL(`https://www.googleapis.com/calendar/v3${path}`);
    for (const [key, value] of Object.entries(query)) {
      if (value !== undefined) {
        url.searchParams.set(key, value);
      }
    }

    const headers = new Headers(init.headers);
    headers.set("Authorization", `Bearer ${accessToken}`);
    if (init.body && !headers.has("Content-Type")) {
      headers.set("Content-Type", "application/json");
    }

    const response = await fetch(url, { ...init, headers });
    const body = await response.text();

    if (response.status === 410) {
      throw new ProviderSyncTokenExpiredError(this.name);
    }

    if (!response.ok) {
      throw new ProviderHttpError(this.name, response.status, body);
    }

    if (!body) {
      return undefined as T;
    }

    return JSON.parse(body) as T;
  }

  private async getAccessToken(): Promise<string> {
    if (this.accessToken && Date.now() < this.accessTokenExpiresAt - 60_000) {
      return this.accessToken;
    }

    const body = new URLSearchParams({
      client_id: this.options.clientId as string,
      client_secret: this.options.clientSecret as string,
      refresh_token: this.options.refreshToken as string,
      grant_type: "refresh_token"
    });

    const response = await fetch("https://oauth2.googleapis.com/token", {
      method: "POST",
      headers: { "Content-Type": "application/x-www-form-urlencoded" },
      body
    });
    const text = await response.text();

    if (!response.ok) {
      throw new ProviderHttpError(this.name, response.status, text);
    }

    const token = JSON.parse(text) as {
      access_token: string;
      expires_in?: number;
    };

    this.accessToken = token.access_token;
    this.accessTokenExpiresAt = Date.now() + (token.expires_in ?? 3600) * 1000;
    return this.accessToken;
  }
}
