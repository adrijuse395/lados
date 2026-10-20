// Numeric formats follow the regional settings without mixing languages into the English UI.
const DATE_PARTS: Intl.DateTimeFormatOptions = { year: "numeric", month: "2-digit", day: "2-digit" };
const dateTimeFormat = new Intl.DateTimeFormat(undefined, { ...DATE_PARTS, hour: "2-digit", minute: "2-digit" });
const dateFormat = new Intl.DateTimeFormat(undefined, DATE_PARTS);
const DAY_MS = 86_400_000;

/** Entries expiring within this many days are highlighted. */
export const EXPIRY_WARNING_DAYS = 14;

export type ExpiryState = "expired" | "soon" | "later";

export function formatDateTime(iso: string): string {
  const date = new Date(iso);
  return Number.isNaN(date.getTime()) ? iso : dateTimeFormat.format(date);
}

/** Reads a `YYYY-MM-DD` value as a calendar date in the local time zone. */
export function parseDate(value: string): Date | null {
  const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(value);
  return match ? new Date(Number(match[1]), Number(match[2]) - 1, Number(match[3])) : null;
}

export function formatDate(value: string): string {
  const date = parseDate(value);
  return date ? dateFormat.format(date) : value;
}

export function todayIso(): string {
  const now = new Date();
  const month = String(now.getMonth() + 1).padStart(2, "0");
  const day = String(now.getDate()).padStart(2, "0");
  return `${now.getFullYear()}-${month}-${day}`;
}

export function daysUntil(value: string): number | null {
  const date = parseDate(value);
  if (!date) return null;
  const today = new Date();
  today.setHours(0, 0, 0, 0);
  return Math.round((date.getTime() - today.getTime()) / DAY_MS);
}

export function expiryState(value: string | null): ExpiryState | null {
  if (!value) return null;
  const days = daysUntil(value);
  if (days === null) return null;
  if (days < 0) return "expired";
  return days <= EXPIRY_WARNING_DAYS ? "soon" : "later";
}

export function describeExpiry(value: string): string {
  const days = daysUntil(value);
  if (days === null) return value;
  if (days < -1) return `Expired ${-days} days ago`;
  if (days === -1) return "Expired yesterday";
  if (days === 0) return "Expires today";
  if (days === 1) return "Expires tomorrow";
  if (days <= EXPIRY_WARNING_DAYS) return `Expires in ${days} days`;
  return `Expires on ${formatDate(value)}`;
}

export function plural(count: number, singular: string, pluralForm = `${singular}s`): string {
  return `${count} ${count === 1 ? singular : pluralForm}`;
}
