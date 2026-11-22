import { apiFetch } from "@builtpanel/api";
import type { NotificationCategory, NotificationItem, NotificationSeverity } from "./types";

/** Administrators and customers each have their own inbox, under their own prefix. */
export type Audience = "admin" | "client";
const base = (who: Audience) => `/api/v1/${who}/notifications`;

interface NotificationView {
  id: string;
  event_type: string;
  severity: string;
  subject: string;
  body: string;
  read: boolean;
  /** Unix seconds. */
  created_at: number;
}

const severityOf = (s: string): NotificationSeverity =>
  s === "critical" || s === "error" ? "error" : s === "warning" ? "warning" : s === "success" ? "success" : "info";

const categoryOf = (eventType: string): NotificationCategory => {
  if (eventType.startsWith("ssl") || eventType.includes("certificate")) return "ssl";
  if (eventType.startsWith("backup")) return "backup";
  if (eventType.startsWith("security") || eventType.startsWith("auth") || eventType.includes("login")) return "security";
  if (eventType.startsWith("billing")) return "billing";
  return "system";
};

export function toNotification(n: NotificationView): NotificationItem {
  return {
    id: n.id,
    title: n.subject,
    message: n.body,
    severity: severityOf(n.severity),
    category: categoryOf(n.event_type),
    read: n.read,
    created_at: new Date(n.created_at * 1000).toISOString(),
  };
}

export async function listNotifications(who: Audience): Promise<{ items: NotificationItem[]; unread: number }> {
  const res = await apiFetch<{ items: NotificationView[]; unread: number }>(base(who));
  return { items: res.items.map(toNotification), unread: res.unread };
}

/** Marks some notifications as read, or all of them when no ids are given. */
export async function markRead(who: Audience, ids?: string[]): Promise<void> {
  await apiFetch(`${base(who)}/read`, { method: "POST", body: JSON.stringify(ids ? { ids } : {}) });
}

export interface Channel {
  id: string;
  /** `email` or `webhook`. */
  kind: string;
  name: string;
  target: string;
  events: string[];
  enabled: boolean;
}

export const listChannels = (who: Audience) => apiFetch<Channel[]>(`${base(who)}/channels`);

export const listEvents = async (who: Audience) => (await apiFetch<{ events: string[] }>(`${base(who)}/events`)).events;

/** A webhook's secret is shown once, when it is made. */
export async function addChannel(
  who: Audience,
  input: { kind: "email" | "webhook"; name: string; target: string; events: string[] },
): Promise<{ channel: Channel; secret?: string | null }> {
  return apiFetch(`${base(who)}/channels`, { method: "POST", body: JSON.stringify(input) });
}

export async function changeChannel(who: Audience, id: string, change: { enabled?: boolean; events?: string[] }): Promise<void> {
  await apiFetch(`${base(who)}/channels/${encodeURIComponent(id)}`, { method: "PATCH", body: JSON.stringify(change) });
}

export async function removeChannel(who: Audience, id: string): Promise<void> {
  await apiFetch(`${base(who)}/channels/${encodeURIComponent(id)}`, { method: "DELETE" });
}

export interface Quiet {
  paused: boolean;
  /** Unix seconds. */
  snoozed_until?: number | null;
}

export const getQuiet = (who: Audience) => apiFetch<Quiet>(`${base(who)}/quiet`);

/** Pauses everything, or snoozes for some minutes (`snoozeMinutes: 0` and `paused: false` end it). */
export const setQuiet = (who: Audience, paused: boolean, snoozeMinutes?: number) =>
  apiFetch<Quiet>(`${base(who)}/quiet`, {
    method: "PUT",
    body: JSON.stringify({ paused, ...(snoozeMinutes !== undefined ? { snooze_minutes: snoozeMinutes } : {}) }),
  });

// ---- the panel's own mail server (administrators)

export interface SmtpSettings {
  enabled: boolean;
  host: string;
  port: number;
  /** `none`, `starttls` or `tls`. */
  security: string;
  username?: string | null;
  from_address: string;
  from_name?: string | null;
  has_password: boolean;
}

export const getSmtp = () => apiFetch<SmtpSettings | null>("/api/v1/admin/notifications/smtp");

export async function putSmtp(s: Omit<SmtpSettings, "has_password"> & { password?: string | null }): Promise<void> {
  await apiFetch("/api/v1/admin/notifications/smtp", { method: "PUT", body: JSON.stringify(s) });
}

export async function testSmtp(to: string): Promise<void> {
  await apiFetch("/api/v1/admin/notifications/smtp/test", { method: "POST", body: JSON.stringify({ to }) });
}
