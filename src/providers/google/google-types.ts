export type GoogleCalendarListEntry = {
  id: string;
  summary?: string | undefined;
  timeZone?: string | undefined;
  backgroundColor?: string | undefined;
  accessRole?: string | undefined;
};

export type GoogleCalendarListResponse = {
  items?: GoogleCalendarListEntry[] | undefined;
  nextPageToken?: string | undefined;
};

export type GoogleEventsResponse = {
  items?: GoogleEvent[] | undefined;
  nextPageToken?: string | undefined;
  nextSyncToken?: string | undefined;
};

export type GoogleEvent = {
  id?: string | undefined;
  etag?: string | undefined;
  status?: "confirmed" | "tentative" | "cancelled" | undefined;
  htmlLink?: string | undefined;
  created?: string | undefined;
  updated?: string | undefined;
  summary?: string | undefined;
  description?: string | undefined;
  location?: string | undefined;
  colorId?: string | undefined;
  creator?: { email?: string | undefined; displayName?: string | undefined; self?: boolean | undefined } | undefined;
  organizer?: { email?: string | undefined; displayName?: string | undefined; self?: boolean | undefined } | undefined;
  start?: GoogleEventDateTime | undefined;
  end?: GoogleEventDateTime | undefined;
  recurrence?: string[] | undefined;
  recurringEventId?: string | undefined;
  originalStartTime?: GoogleEventDateTime | undefined;
  transparency?: "opaque" | "transparent" | undefined;
  visibility?: "default" | "public" | "private" | "confidential" | undefined;
  iCalUID?: string | undefined;
  sequence?: number | undefined;
  attendees?: GoogleAttendee[] | undefined;
  reminders?: {
    useDefault?: boolean | undefined;
    overrides?: GoogleReminder[] | undefined;
  } | undefined;
  extendedProperties?: {
    private?: Record<string, string> | undefined;
    shared?: Record<string, string> | undefined;
  } | undefined;
};

export type GoogleEventDateTime =
  | {
      date: string;
      dateTime?: never;
      timeZone?: string | undefined;
    }
  | {
      date?: never;
      dateTime: string;
      timeZone?: string | undefined;
    };

export type GoogleAttendee = {
  email?: string | undefined;
  displayName?: string | undefined;
  optional?: boolean | undefined;
  responseStatus?: "needsAction" | "declined" | "tentative" | "accepted" | undefined;
};

export type GoogleReminder = {
  method?: "email" | "popup" | undefined;
  minutes?: number | undefined;
};
