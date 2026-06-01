import ICAL from "ical.js";
import type {
  CanonicalEvent,
  EventAttendee,
  EventDateTime,
  EventReminder,
  EventStatus,
  EventVisibility,
  ProviderEventMeta
} from "../types";

type CalendarObjectLike = {
  url: string;
  etag?: string | undefined;
  data?: string | undefined;
};

export function icalObjectToCanonical(
  calendarId: string,
  object: CalendarObjectLike
): CanonicalEvent[] {
  if (!object.data) {
    return [];
  }

  const root = ICAL.Component.fromString(object.data);
  const events = root.getAllSubcomponents("vevent");

  return events.map((component) => componentToCanonical(calendarId, component, object));
}

export function canonicalToICal(event: CanonicalEvent): string {
  const calendar = new ICAL.Component("vcalendar");
  calendar.updatePropertyWithValue("version", "2.0");
  calendar.updatePropertyWithValue("prodid", "-//insync//calendar sync//EN");
  calendar.updatePropertyWithValue("calscale", "GREGORIAN");

  const component = new ICAL.Component("vevent");
  component.updatePropertyWithValue("uid", event.canonicalUid);
  component.updatePropertyWithValue("dtstamp", ICAL.Time.fromJSDate(new Date(), true));
  component.updatePropertyWithValue("summary", event.title);

  setOptionalText(component, "description", event.description);
  setOptionalText(component, "location", event.location);
  component.updatePropertyWithValue("status", event.status.toUpperCase());
  component.updatePropertyWithValue("class", event.visibility.toUpperCase());
  setTimeProperty(component, "dtstart", event.start);
  setTimeProperty(component, "dtend", event.end);

  if (event.recurrence?.sequence !== undefined) {
    component.updatePropertyWithValue("sequence", event.recurrence.sequence);
  }

  if (event.recurrence?.rrule) {
    component.addProperty(ICAL.Property.fromString(`RRULE:${event.recurrence.rrule}`));
  }

  for (const exdate of event.recurrence?.exdates ?? []) {
    component.addProperty(ICAL.Property.fromString(`EXDATE:${exdate}`));
  }

  if (event.recurrence?.recurrenceId) {
    component.addProperty(
      ICAL.Property.fromString(`RECURRENCE-ID:${event.recurrence.recurrenceId}`)
    );
  }

  for (const attendee of event.attendees) {
    const property = new ICAL.Property("attendee");
    property.setValue(`mailto:${attendee.email}`);
    if (attendee.name) {
      property.setParameter("cn", attendee.name);
    }
    if (attendee.responseStatus) {
      property.setParameter("partstat", attendee.responseStatus.toUpperCase());
    }
    if (attendee.optional) {
      property.setParameter("role", "OPT-PARTICIPANT");
    }
    component.addProperty(property);
  }

  for (const reminder of event.reminders) {
    component.addSubcomponent(reminderToAlarm(reminder));
  }

  calendar.addSubcomponent(component);
  return calendar.toString();
}

export function icalMetaFromObject(
  calendarId: string,
  objectUrl: string,
  etag?: string
): ProviderEventMeta {
  return {
    provider: "icloud",
    calendarId,
    href: objectUrl,
    etag
  };
}

export function eventFilename(event: CanonicalEvent): string {
  const safeUid = event.canonicalUid.replace(/[^a-zA-Z0-9._-]/g, "_");
  return `${safeUid || crypto.randomUUID()}.ics`;
}

function componentToCanonical(
  calendarId: string,
  component: InstanceType<typeof ICAL.Component>,
  object: CalendarObjectLike
): CanonicalEvent {
  const uid = stringValue(component, "uid") ?? crypto.randomUUID();
  const status = statusValue(component);
  const visibility = visibilityValue(component);

  return {
    canonicalUid: uid,
    title: stringValue(component, "summary") ?? "",
    description: stringValue(component, "description") ?? undefined,
    location: stringValue(component, "location") ?? undefined,
    status,
    visibility,
    start: timePropertyToCanonical(component, "dtstart"),
    end: timePropertyToCanonical(component, "dtend"),
    recurrence: recurrenceValue(component),
    attendees: attendeesValue(component),
    reminders: remindersValue(component),
    providerMeta: {
      provider: "icloud",
      calendarId,
      href: object.url,
      etag: object.etag,
      iCalUid: uid,
      deleted: status === "cancelled"
    },
    raw: {
      href: object.url,
      etag: object.etag,
      ics: object.data
    }
  };
}

function stringValue(
  component: InstanceType<typeof ICAL.Component>,
  propertyName: string
): string | null {
  const value = component.getFirstPropertyValue(propertyName);
  return typeof value === "string" ? value : value?.toString() ?? null;
}

function statusValue(component: InstanceType<typeof ICAL.Component>): EventStatus {
  const status = stringValue(component, "status")?.toLowerCase();
  if (status === "tentative" || status === "cancelled") {
    return status;
  }
  return "confirmed";
}

function visibilityValue(component: InstanceType<typeof ICAL.Component>): EventVisibility {
  const visibility = stringValue(component, "class")?.toLowerCase();
  if (visibility === "public" || visibility === "private" || visibility === "confidential") {
    return visibility;
  }
  return "default";
}

function timePropertyToCanonical(
  component: InstanceType<typeof ICAL.Component>,
  propertyName: string
): EventDateTime {
  const property = component.getFirstProperty(propertyName);
  const value = property?.getFirstValue();

  if (!value || typeof value === "string") {
    return { kind: "dateTime", value: new Date(0).toISOString(), timezone: "UTC" };
  }

  const time = value as InstanceType<typeof ICAL.Time>;
  if (time.isDate) {
    return { kind: "date", value: formatDate(time) };
  }

  return {
    kind: "dateTime",
    value: time.toJSDate().toISOString(),
    timezone: property?.getFirstParameter("tzid") ?? time.zone?.tzid ?? undefined
  };
}

function setTimeProperty(
  component: InstanceType<typeof ICAL.Component>,
  propertyName: string,
  value: EventDateTime
): void {
  const property = new ICAL.Property(propertyName);

  if (value.kind === "date") {
    property.resetType("date");
    property.setValue(ICAL.Time.fromDateString(value.value));
  } else {
    property.resetType("date-time");
    if (value.timezone && value.timezone !== "UTC") {
      property.setParameter("tzid", value.timezone);
    }
    property.setValue(ICAL.Time.fromJSDate(new Date(value.value), true));
  }

  component.removeAllProperties(propertyName);
  component.addProperty(property);
}

function recurrenceValue(component: InstanceType<typeof ICAL.Component>): CanonicalEvent["recurrence"] {
  const rrule = component.getFirstProperty("rrule")?.getFirstValue()?.toString();
  const exdates = component
    .getAllProperties("exdate")
    .flatMap((property) => property.getValues().map((value) => value?.toString()))
    .filter((value): value is string => Boolean(value));
  const recurrenceId = component.getFirstProperty("recurrence-id")?.getFirstValue()?.toString();
  const sequence = component.getFirstPropertyValue("sequence");

  if (!rrule && exdates.length === 0 && !recurrenceId && sequence === null) {
    return undefined;
  }

  return {
    rrule,
    exdates,
    recurrenceId,
    sequence: typeof sequence === "number" ? sequence : undefined
  };
}

function attendeesValue(component: InstanceType<typeof ICAL.Component>): EventAttendee[] {
  return component
    .getAllProperties("attendee")
    .map((property) => {
      const rawEmail = property.getFirstValue()?.toString() ?? "";
      const email = rawEmail.replace(/^mailto:/i, "");
      return {
        email,
        name: property.getFirstParameter("cn") ?? undefined,
        responseStatus: normalizePartstat(property.getFirstParameter("partstat")),
        optional: property.getFirstParameter("role") === "OPT-PARTICIPANT"
      };
    })
    .filter((attendee) => attendee.email);
}

function remindersValue(component: InstanceType<typeof ICAL.Component>): EventReminder[] {
  return component
    .getAllSubcomponents("valarm")
    .map((alarm) => {
      const trigger = alarm.getFirstPropertyValue("trigger")?.toString() ?? "";
      const minutes = durationToMinutes(trigger);
      return {
        method: "display" as const,
        minutesBeforeStart: minutes
      };
    })
    .filter((reminder) => Number.isFinite(reminder.minutesBeforeStart));
}

function reminderToAlarm(reminder: EventReminder): InstanceType<typeof ICAL.Component> {
  const alarm = new ICAL.Component("valarm");
  alarm.updatePropertyWithValue("action", reminder.method === "email" ? "EMAIL" : "DISPLAY");
  alarm.updatePropertyWithValue("description", "Reminder");
  alarm.updatePropertyWithValue("trigger", `-PT${reminder.minutesBeforeStart}M`);
  return alarm;
}

function normalizePartstat(
  partstat: string | undefined
): EventAttendee["responseStatus"] | undefined {
  switch (partstat?.toUpperCase()) {
    case "DECLINED":
      return "declined";
    case "TENTATIVE":
      return "tentative";
    case "ACCEPTED":
      return "accepted";
    case "NEEDS-ACTION":
      return "needsAction";
    default:
      return undefined;
  }
}

function durationToMinutes(value: string): number {
  const match = value.match(/^-?PT(?:(\d+)H)?(?:(\d+)M)?/);
  if (!match) {
    return 0;
  }

  const hours = Number(match[1] ?? 0);
  const minutes = Number(match[2] ?? 0);
  return hours * 60 + minutes;
}

function setOptionalText(
  component: InstanceType<typeof ICAL.Component>,
  propertyName: string,
  value: string | undefined
): void {
  if (value) {
    component.updatePropertyWithValue(propertyName, value);
  } else {
    component.removeAllProperties(propertyName);
  }
}

function formatDate(time: InstanceType<typeof ICAL.Time>): string {
  return `${time.year.toString().padStart(4, "0")}-${time.month
    .toString()
    .padStart(2, "0")}-${time.day.toString().padStart(2, "0")}`;
}
