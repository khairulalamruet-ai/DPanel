import { apiFetch } from "@builtpanel/api";
import type { ServerAuditEvent, ServerOverview, SshConfigInfo } from "./types";

interface NodeServerView {
  hostname: string;
  os: string;
  kernel: string;
  uptime_seconds: number;
  timezone: string;
  memory_total_bytes: number;
  memory_available_bytes: number;
  swap_total_bytes: number;
  swap_used_bytes: number;
  /** The sshd settings the node reports. */
  ssh: { name: string; value: string }[];
}

const MB = 1024 * 1024;
const N = "/api/v1/admin/nodes";
const enc = encodeURIComponent;

export interface NodeChoice {
  id: string;
  name: string;
}

export async function listNodeChoices(): Promise<NodeChoice[]> {
  const res = await apiFetch<{ items: { id: string; name: string; status: string }[] }>(N);
  return res.items.filter((n) => n.status !== "revoked").map((n) => ({ id: n.id, name: n.name }));
}

function uptime(seconds: number): string {
  const d = Math.floor(seconds / 86400);
  const h = Math.floor((seconds % 86400) / 3600);
  const m = Math.floor((seconds % 3600) / 60);
  return d > 0 ? `${d}d ${h}h` : h > 0 ? `${h}h ${m}m` : `${m}m`;
}

const setting = (ssh: { name: string; value: string }[], name: string): string | undefined =>
  ssh.find((s) => s.name.toLowerCase() === name.toLowerCase())?.value;

export function toOverview(nodeId: string, v: NodeServerView): ServerOverview {
  return {
    node_id: nodeId,
    hostname: v.hostname,
    os: v.os,
    kernel: v.kernel,
    uptime_seconds: v.uptime_seconds,
    uptime_human: uptime(v.uptime_seconds),
    current_time: new Date().toISOString(),
    timezone: v.timezone,
    memory: {
      total_mb: Math.round(v.memory_total_bytes / MB),
      free_mb: Math.round(v.memory_available_bytes / MB),
      available_mb: Math.round(v.memory_available_bytes / MB),
      buffers_mb: 0,
      cached_mb: 0,
    },
    swap: {
      total_mb: Math.round(v.swap_total_bytes / MB),
      used_mb: Math.round(v.swap_used_bytes / MB),
      free_mb: Math.round((v.swap_total_bytes - v.swap_used_bytes) / MB),
      swappiness: 0,
    },
  };
}

export function toSsh(v: NodeServerView): SshConfigInfo {
  const yes = (name: string, fallback: boolean) => {
    const s = setting(v.ssh, name);
    return s === undefined ? fallback : s.toLowerCase() === "yes";
  };
  const root = (setting(v.ssh, "PermitRootLogin") ?? "prohibit-password").toLowerCase();
  const permit = root === "yes" ? "yes" : root === "no" ? "no" : "prohibit-password";
  const passwords = yes("PasswordAuthentication", true);
  return {
    port: Number(setting(v.ssh, "Port") ?? 22),
    permit_root_login: permit,
    password_authentication: passwords,
    pubkey_authentication: yes("PubkeyAuthentication", true),
    max_auth_tries: Number(setting(v.ssh, "MaxAuthTries") ?? 6),
    authorized_keys_count: 0,
    ciphers: [],
    security_rating: permit === "yes" || passwords ? (permit === "yes" && passwords ? "weak" : "moderate") : "hardened",
  };
}

export async function loadServer(nodeId: string): Promise<{ overview: ServerOverview; ssh: SshConfigInfo }> {
  const v = await apiFetch<NodeServerView>(`${N}/${enc(nodeId)}/server`);
  return { overview: toOverview(nodeId, v), ssh: toSsh(v) };
}

/** What was done to the node, newest first (the audit log of that node). */
export async function loadAudit(nodeId: string): Promise<ServerAuditEvent[]> {
  const res = await apiFetch<{
    items: { id: string; action: string; actor_id?: string | null; at: string; outcome: string; metadata: Record<string, unknown> }[];
  }>(`/api/v1/admin/audit-logs?target_type=node&target_id=${enc(nodeId)}&limit=20`);
  return res.items.map((e) => ({
    id: e.id,
    action: e.action,
    actor: e.actor_id ?? "",
    details: Object.keys(e.metadata ?? {}).length ? JSON.stringify(e.metadata) : "",
    created_at: e.at,
    status: e.outcome === "success" ? "success" : "failure",
  }));
}

/** Server changes ask for a recent password check ("sudo mode"); this does that first. */
async function sudo(password: string): Promise<void> {
  await apiFetch("/api/v1/auth/sudo", { method: "POST", body: JSON.stringify({ password }) });
}

export async function setTimezone(nodeId: string, zone: string, password: string): Promise<void> {
  await sudo(password);
  await apiFetch(`${N}/${enc(nodeId)}/server/timezone`, { method: "POST", body: JSON.stringify({ zone }) });
}

/** `0` takes the swap file away. */
export async function setSwap(nodeId: string, megabytes: number, password: string): Promise<void> {
  await sudo(password);
  await apiFetch(`${N}/${enc(nodeId)}/server/swap`, { method: "POST", body: JSON.stringify({ megabytes }) });
}

export async function dropCaches(nodeId: string, password: string): Promise<void> {
  await sudo(password);
  await apiFetch(`${N}/${enc(nodeId)}/server/drop-caches`, { method: "POST", body: JSON.stringify({}) });
}

/** The backend wants the node's name, typed as it is. */
export async function reboot(nodeId: string, nodeName: string, password: string): Promise<void> {
  await sudo(password);
  await apiFetch(`${N}/${enc(nodeId)}/server/reboot`, { method: "POST", body: JSON.stringify({ confirm: nodeName }) });
}
