export type ProviderName = "google" | "icloud";

export type ProviderCalendar = {
  id: string;
  name: string;
  timezone?: string | undefined;
  writable: boolean;
  raw: unknown;
};

export type EventVisibility = "default" | "public" | "private" | "confidential";
export type EventStatus = "confirmed" | "tentative" | "cancelled";

export type CanonicalEvent = {
  canonicalUid: string;
  title: string;
  description?: string | undefined;
  location?: string | undefined;
  status: EventStatus;
  visibility: EventVisibility;
  start: EventDateTime;
  end: EventDateTime;
  recurrence?: RecurrenceData | undefined;
  attendees: EventAttendee[];
  reminders: EventReminder[];
  providerMeta: ProviderEventMeta;
  raw: unknown;
};

export type EventDateTime =
  | {
      kind: "dateTime";
      value: string;
      timezone?: string | undefined;
    }
  | {
      kind: "date";
      value: string;
    };

export type RecurrenceData = {
  rrule?: string | undefined;
  exdates?: string[] | undefined;
  recurrenceId?: string | undefined;
  sequence?: number | undefined;
};

export type EventAttendee = {
  email: string;
  name?: string | undefined;
  responseStatus?: "needsAction" | "declined" | "tentative" | "accepted" | undefined;
  optional?: boolean | undefined;
};

export type EventReminder = {
  method: "popup" | "email" | "display" | "audio" | "unknown";
  minutesBeforeStart: number;
};

export type ProviderEventMeta = {
  provider: ProviderName;
  calendarId: string;
  eventId?: string | undefined;
  href?: string | undefined;
  etag?: string | undefined;
  iCalUid?: string | undefined;
  updatedAt?: string | undefined;
  deleted?: boolean | undefined;
};

export type ProviderChangeSet = {
  provider: ProviderName;
  calendarId: string;
  syncToken?: string | undefined;
  events: CanonicalEvent[];
};

export type ProviderSyncCursor = {
  syncToken?: string | undefined;
  fullSync: boolean;
};

export interface CalendarProvider {
  readonly name: ProviderName;

  listCalendars(): Promise<ProviderCalendar[]>;

  getChanges(
    calendarId: string,
    cursor: ProviderSyncCursor
  ): Promise<ProviderChangeSet>;

  createEvent(calendarId: string, event: CanonicalEvent): Promise<ProviderEventMeta>;

  updateEvent(
    calendarId: string,
    remoteEventId: string,
    event: CanonicalEvent,
    etag?: string
  ): Promise<ProviderEventMeta>;

  deleteEvent(calendarId: string, remoteEventId: string, etag?: string): Promise<void>;
}
