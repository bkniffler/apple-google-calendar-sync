import { createDAVClient, type DAVCalendar, type DAVCalendarObject } from "tsdav";
import { ProviderHttpError, ProviderNotConfiguredError } from "../errors";
import type {
  CalendarProvider,
  CanonicalEvent,
  ProviderCalendar,
  ProviderChangeSet,
  ProviderEventMeta,
  ProviderSyncCursor
} from "../types";
import {
  canonicalToICal,
  eventFilename,
  icalMetaFromObject,
  icalObjectToCanonical
} from "./ical-mapper";

export type ICloudProviderOptions = {
  username?: string | undefined;
  appSpecificPassword?: string | undefined;
  serverUrl?: string | undefined;
};

export class ICloudCalDavProvider implements CalendarProvider {
  readonly name = "icloud" as const;
  private client?: Awaited<ReturnType<typeof createDAVClient>>;
  private calendars?: DAVCalendar[];

  constructor(private readonly options: ICloudProviderOptions = {}) {}

  async listCalendars(): Promise<ProviderCalendar[]> {
    this.assertConfigured();
    const calendars = await this.fetchCalendars();
    return calendars.map((calendar) => ({
      id: calendar.url,
      name: displayName(calendar),
      timezone: calendar.timezone,
      writable: true,
      raw: calendar
    }));
  }

  async getChanges(
    calendarId: string,
    cursor: ProviderSyncCursor
  ): Promise<ProviderChangeSet> {
    this.assertConfigured();
    void cursor;
    const client = await this.getClient();
    const calendar = await this.findCalendar(calendarId);
    const objects = (await client.fetchCalendarObjects({
      calendar,
      useMultiGet: true
    })) as DAVCalendarObject[];

    return {
      provider: this.name,
      calendarId,
      syncToken: calendar.syncToken,
      events: objects.flatMap((object) => {
        const calendarObject = {
          url: object.url,
          ...(object.etag ? { etag: object.etag } : {}),
          ...(typeof object.data === "string" ? { data: object.data } : {})
        };
        return icalObjectToCanonical(calendarId, calendarObject);
      })
    };
  }

  async createEvent(
    calendarId: string,
    event: CanonicalEvent
  ): Promise<ProviderEventMeta> {
    this.assertConfigured();
    const client = await this.getClient();
    const calendar = await this.findCalendar(calendarId);
    const filename = eventFilename(event);
    const response = await client.createCalendarObject({
      calendar,
      filename,
      iCalString: canonicalToICal(event)
    });
    await assertDavOk(this.name, response);

    const href = new URL(filename, ensureTrailingSlash(calendar.url)).toString();
    return icalMetaFromObject(calendarId, href, response.headers.get("etag") ?? undefined);
  }

  async updateEvent(
    calendarId: string,
    remoteEventId: string,
    event: CanonicalEvent,
    etag?: string
  ): Promise<ProviderEventMeta> {
    this.assertConfigured();
    const client = await this.getClient();
    const calendarObject = {
        url: remoteEventId,
        data: canonicalToICal(event),
        ...(etag ? { etag } : {})
      };
    const response = await client.updateCalendarObject({ calendarObject });
    await assertDavOk(this.name, response);

    return icalMetaFromObject(calendarId, remoteEventId, response.headers.get("etag") ?? etag);
  }

  async deleteEvent(
    calendarId: string,
    remoteEventId: string,
    etag?: string
  ): Promise<void> {
    this.assertConfigured();
    const client = await this.getClient();
    void calendarId;
    const calendarObject = {
        url: remoteEventId,
        ...(etag ? { etag } : {})
      };
    const response = await client.deleteCalendarObject({ calendarObject });
    await assertDavOk(this.name, response);
  }

  private assertConfigured(): void {
    if (!this.options.username || !this.options.appSpecificPassword) {
      throw new ProviderNotConfiguredError(
        this.name,
        "set ICLOUD_USERNAME and ICLOUD_APP_SPECIFIC_PASSWORD"
      );
    }
  }

  private async getClient(): Promise<Awaited<ReturnType<typeof createDAVClient>>> {
    if (this.client) {
      return this.client;
    }

    this.client = await createDAVClient({
      serverUrl: this.options.serverUrl ?? "https://caldav.icloud.com",
      credentials: {
        username: this.options.username as string,
        password: this.options.appSpecificPassword as string
      },
      authMethod: "Basic",
      defaultAccountType: "caldav"
    });
    return this.client;
  }

  private async fetchCalendars(): Promise<DAVCalendar[]> {
    if (this.calendars) {
      return this.calendars;
    }

    const client = await this.getClient();
    this.calendars = await client.fetchCalendars();
    return this.calendars;
  }

  private async findCalendar(calendarId: string): Promise<DAVCalendar> {
    const calendars = await this.fetchCalendars();
    const calendar = calendars.find(
      (candidate) =>
        candidate.url === calendarId ||
        candidate.url.endsWith(calendarId) ||
        displayName(candidate) === calendarId
    );

    if (!calendar) {
      throw new ProviderNotConfiguredError(
        this.name,
        `calendar "${calendarId}" was not found; run calendars icloud`
      );
    }

    return calendar;
  }
}

function displayName(calendar: DAVCalendar): string {
  return typeof calendar.displayName === "string"
    ? calendar.displayName
    : JSON.stringify(calendar.displayName ?? calendar.url);
}

function ensureTrailingSlash(value: string): string {
  return value.endsWith("/") ? value : `${value}/`;
}

async function assertDavOk(provider: string, response: Response): Promise<void> {
  if (!response.ok) {
    throw new ProviderHttpError(provider, response.status, await response.text());
  }
}
