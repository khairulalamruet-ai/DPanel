import { apiFetch } from "@builtpanel/api";

const BASE = "/api/v1/admin/security";

export interface ImageRow {
  reference: string;
  role: string;
  note: string;
  tag?: string | null;
  update_available: boolean;
  critical?: number | null;
  high?: number | null;
  medium?: number | null;
  low?: number | null;
  scanned_at?: number | null;
}
export interface ImageFinding {
  id: string;
  package: string;
  installed: string;
  fixed: string;
  severity: string;
  title: string;
}
export interface UserAgentRule {
  id: string;
  pattern: string;
  note?: string | null;
}
export interface Advisory {
  id: string;
  component: string;
  slug: string;
  fixed_in: string;
  severity: string;
  title: string;
  reference?: string | null;
}

export async function listImages(): Promise<ImageRow[]> {
  return (await apiFetch<{ items: ImageRow[] }>(`${BASE}/images`)).items;
}
export async function scanImage(reference: string): Promise<void> {
  await apiFetch(`${BASE}/images/scan`, { method: "POST", body: JSON.stringify({ reference }) });
}
export async function imageFindings(reference: string): Promise<ImageFinding[]> {
  return (await apiFetch<{ items: ImageFinding[] }>(`${BASE}/images/vulnerabilities`, { params: { reference } })).items;
}
export async function listUserAgents(): Promise<UserAgentRule[]> {
  return (await apiFetch<{ items: UserAgentRule[] }>(`${BASE}/user-agents`)).items;
}
export async function addUserAgent(pattern: string, note: string): Promise<void> {
  await apiFetch(`${BASE}/user-agents`, { method: "POST", body: JSON.stringify({ pattern, note: note || null }) });
}
export async function removeUserAgent(id: string): Promise<void> {
  await apiFetch(`${BASE}/user-agents/${encodeURIComponent(id)}`, { method: "DELETE" });
}
export async function listAdvisories(): Promise<Advisory[]> {
  return (await apiFetch<{ items: Advisory[] }>(`${BASE}/wp-advisories`)).items;
}
export async function addAdvisory(a: Omit<Advisory, "id">): Promise<void> {
  await apiFetch(`${BASE}/wp-advisories`, { method: "POST", body: JSON.stringify({ ...a, reference: a.reference || null }) });
}
export async function removeAdvisory(id: string): Promise<void> {
  await apiFetch(`${BASE}/wp-advisories/${encodeURIComponent(id)}`, { method: "DELETE" });
}
