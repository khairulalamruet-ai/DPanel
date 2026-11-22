import { apiFetch } from "@builtpanel/api";

const BASE = "/api/v1/client";

export interface CacheServer {
  kind: string;
  status: string;
  memory_mb: number;
  host?: string | null;
  port?: number | null;
  password?: string | null;
  error?: string | null;
}

export interface GitDeploy {
  id: string;
  name: string;
  repo_url: string;
  branch: string;
  dest: string;
  status: string;
  has_token: boolean;
  webhook_path: string;
  webhook_secret?: string | null;
  last_commit?: string | null;
  last_deployed_at?: number | null;
  last_error?: string | null;
}

export interface MalwareScan {
  id: string;
  state: string;
  requested_at: number;
  scanned?: number | null;
  error?: string | null;
  findings: { path: string; signature: string; quarantined: boolean }[];
}

export interface VulnScan {
  id: string;
  state: string;
  requested_at: number;
  error?: string | null;
  issues: { component: string; version: string; fixed_in: string; severity: string; title: string; path: string }[];
}

export interface Traffic {
  requests: number;
  bytes: number;
  bots: number;
  status_2xx: number;
  status_3xx: number;
  status_4xx: number;
  status_5xx: number;
  top_paths: { path: string; requests: number }[];
}

export async function listCache(): Promise<{ items: CacheServer[]; limit_mb?: number | null }> {
  return apiFetch(`${BASE}/cache`);
}
export async function createCache(kind: string, memory_mb: number): Promise<CacheServer> {
  return apiFetch(`${BASE}/cache`, { method: "POST", body: JSON.stringify({ kind, memory_mb }) });
}
export async function deleteCache(kind: string): Promise<void> {
  await apiFetch(`${BASE}/cache/${encodeURIComponent(kind)}`, { method: "DELETE" });
}

export async function listDeploys(): Promise<GitDeploy[]> {
  const res = await apiFetch<{ items: GitDeploy[] }>(`${BASE}/git-deploys`);
  return res.items;
}
export async function createDeploy(input: {
  name: string;
  repo_url: string;
  branch: string;
  dest: string;
  token?: string;
}): Promise<GitDeploy> {
  const { token, ...rest } = input;
  return apiFetch(`${BASE}/git-deploys`, {
    method: "POST",
    body: JSON.stringify({ ...rest, token: token || null, deploy_now: true }),
  });
}
export async function deployNow(id: string): Promise<GitDeploy> {
  return apiFetch(`${BASE}/git-deploys/${encodeURIComponent(id)}/deploy`, { method: "POST" });
}
export async function newWebhookSecret(id: string): Promise<GitDeploy> {
  return apiFetch(`${BASE}/git-deploys/${encodeURIComponent(id)}/webhook-secret`, { method: "POST" });
}
export async function deleteDeploy(id: string): Promise<void> {
  await apiFetch(`${BASE}/git-deploys/${encodeURIComponent(id)}`, { method: "DELETE" });
}

export async function listMalwareScans(): Promise<MalwareScan[]> {
  return (await apiFetch<{ items: MalwareScan[] }>(`${BASE}/malware-scans`)).items;
}
export async function startMalwareScan(quarantine: boolean): Promise<MalwareScan> {
  return apiFetch(`${BASE}/malware-scans`, { method: "POST", body: JSON.stringify({ quarantine }) });
}
export async function listVulnScans(): Promise<VulnScan[]> {
  return (await apiFetch<{ items: VulnScan[] }>(`${BASE}/vulnerability-scans`)).items;
}
export async function startVulnScan(): Promise<VulnScan> {
  return apiFetch(`${BASE}/vulnerability-scans`, { method: "POST" });
}

export async function getTraffic(): Promise<Traffic> {
  const { untilAnswered } = await import("@builtpanel/api");
  return untilAnswered(() => apiFetch<Traffic>(`${BASE}/traffic`));
}
