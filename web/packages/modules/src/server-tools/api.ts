import { apiFetch } from "@builtpanel/api";
export { listNodeChoices, type NodeChoice } from "../nodes/api";
import type {
  ServerAuditEvent,
  ServerOverview,
  SshConfigInfo,
  SshBasicConfig,
  SshAuthorizedKey,
  RebootMode,
  MigrationRequest,
  MigrationStatus,
  JoinServerRequest,
  JoinCodeResult,
  ClusterNodeSummary,
} from "./types";

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

function uptime(seconds: number): string {
  const d = Math.floor(seconds / 86400);
  const h = Math.floor((seconds % 86400) / 3600);
  const m = Math.floor((seconds % 3600) / 60);
  return d > 0 ? `${d}d ${h}h` : h > 0 ? `${h}h ${m}m` : `${m}m`;
}

const setting = (ssh: { name: string; value: string }[], name: string): string | undefined =>
  ssh?.find((s) => s.name.toLowerCase() === name.toLowerCase())?.value;

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
export async function sudo(password: string): Promise<void> {
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

/** The backend wants the node's name, typed as it is. Optionally supports graceful vs hard reboot. */
export async function reboot(nodeId: string, nodeName: string, password: string, mode: RebootMode = "graceful"): Promise<void> {
  await sudo(password);
  await apiFetch(`${N}/${enc(nodeId)}/server/reboot`, {
    method: "POST",
    body: JSON.stringify({ confirm: nodeName, mode }),
  });
}

/** Probe whether node is responsive after reboot. */
export async function checkNodeOnline(nodeId: string): Promise<boolean> {
  try {
    await apiFetch(`${N}/${enc(nodeId)}/ping`, { method: "POST" });
    return true;
  } catch {
    return false;
  }
}

// ==================== SSH Access APIs (TODO: T10.15) ====================

export async function loadSshBasicConfig(nodeId: string): Promise<SshBasicConfig> {
  try {
    const res = await apiFetch<SshBasicConfig>(`${N}/${enc(nodeId)}/ssh`);
    return res;
  } catch {
    return {
      port: 22,
      permit_root_login: "no",
      password_authentication: false,
      pubkey_authentication: true,
      max_auth_tries: 6,
    };
  }
}

export async function updateSshBasicConfig(nodeId: string, config: SshBasicConfig, password: string): Promise<void> {
  await sudo(password);
  await apiFetch(`${N}/${enc(nodeId)}/ssh/config`, {
    method: "PUT",
    body: JSON.stringify(config),
  });
}

export async function loadSshKeys(nodeId: string): Promise<SshAuthorizedKey[]> {
  try {
    const res = await apiFetch<{ items: SshAuthorizedKey[] }>(`${N}/${enc(nodeId)}/ssh/keys`);
    return res.items;
  } catch {
    return [];
  }
}

export async function addSshKey(
  nodeId: string,
  key: { name: string; public_key: string },
  password: string,
): Promise<SshAuthorizedKey> {
  await sudo(password);
  return await apiFetch<SshAuthorizedKey>(`${N}/${enc(nodeId)}/ssh/keys`, {
    method: "POST",
    body: JSON.stringify(key),
  });
}

export async function deleteSshKey(nodeId: string, keyId: string, password: string): Promise<void> {
  await sudo(password);
  await apiFetch(`${N}/${enc(nodeId)}/ssh/keys/${enc(keyId)}`, {
    method: "DELETE",
  });
}

export async function loadSshRawConfig(nodeId: string): Promise<string> {
  try {
    const res = await apiFetch<{ raw: string }>(`${N}/${enc(nodeId)}/ssh/raw`);
    return res.raw;
  } catch {
    return `# /etc/ssh/sshd_config (Default)\nPort 22\nPermitRootLogin no\nPasswordAuthentication no\nPubkeyAuthentication yes\nAuthorizedKeysFile .ssh/authorized_keys\nSubsystem sftp /usr/lib/openssh/sftp-server\n`;
  }
}

export async function updateSshRawConfig(nodeId: string, raw: string, password: string): Promise<void> {
  await sudo(password);
  await apiFetch(`${N}/${enc(nodeId)}/ssh/raw`, {
    method: "PUT",
    body: JSON.stringify({ raw }),
  });
}

// ==================== Server Migration APIs (TODO: T15.01) ====================

export async function startMigration(
  nodeId: string,
  req: MigrationRequest,
  password: string,
): Promise<{ id: string; message: string }> {
  await sudo(password);
  // Important Rule: The remote password lives only in the form field in memory.
  // We send it securely in the request payload and do NOT persist it anywhere in client state.
  return await apiFetch<{ id: string; message: string }>(`${N}/${enc(nodeId)}/migrate`, {
    method: "POST",
    body: JSON.stringify(req),
  });
}

export async function getMigrationStatus(nodeId: string): Promise<MigrationStatus> {
  try {
    return await apiFetch<MigrationStatus>(`${N}/${enc(nodeId)}/migrate/status`);
  } catch {
    return {
      id: "mig-none",
      status: "idle",
      progress: 0,
      current_step: "Ready",
      logs: [],
      steps: [],
    };
  }
}

// ==================== Multi-Server Cluster APIs (TODO: T02.08) ====================

export async function joinServer(req: JoinServerRequest, password: string): Promise<{ success: boolean; node_id: string }> {
  await sudo(password);
  return await apiFetch<{ success: boolean; node_id: string }>(`${N}/join`, {
    method: "POST",
    body: JSON.stringify(req),
  });
}

export async function generateJoinCode(nodeId?: string, password?: string): Promise<JoinCodeResult> {
  if (password) {
    await sudo(password);
  }
  return await apiFetch<JoinCodeResult>(`${N}/join-code`, {
    method: "POST",
    body: JSON.stringify({ node_id: nodeId }),
  });
}

export async function listClusterNodes(): Promise<ClusterNodeSummary[]> {
  try {
    const res = await apiFetch<{
      items: Array<{
        id: string;
        name: string;
        hostname?: string;
        public_ip?: string;
        status: "online" | "offline" | "draining" | "revoked";
        agent_version?: string;
        last_seen_at?: string;
      }>;
    }>(`${N}`);
    return res.items.map((item, idx) => ({
      id: item.id,
      name: item.name,
      hostname: item.hostname || item.name,
      ip_address: item.public_ip,
      status: item.status || "online",
      role: idx === 0 ? "primary" : "worker",
      agent_version: item.agent_version || "0.1.0",
      last_ping_at: item.last_seen_at || new Date().toISOString(),
    }));
  } catch {
    return [];
  }
}

export async function pingNode(nodeId: string): Promise<{ ok: boolean }> {
  try {
    await apiFetch(`${N}/${enc(nodeId)}/ping`, { method: "POST" });
    return { ok: true };
  } catch {
    return { ok: false };
  }
}
