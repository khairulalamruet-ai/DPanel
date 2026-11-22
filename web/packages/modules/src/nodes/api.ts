import { apiFetch } from "@builtpanel/api";
import type { NodeItem, NodeStatus } from "./types";

/** A node as the backend returns it (`NodeView`). */
interface NodeView {
  id: string;
  name: string;
  hostname?: string | null;
  public_ip?: string | null;
  /** `online`, `offline`, `revoked`, ... */
  status: string;
  agent_version?: string | null;
  /** What the agent reported when it connected. */
  capabilities: {
    cpu_cores?: number;
    memory_bytes?: number;
    disk_total_bytes?: number;
    disk_free_bytes?: number;
    kernel?: string;
    docker_version?: string | null;
    seccomp?: boolean;
    apparmor?: boolean;
    user_namespaces?: boolean;
    cgroup_v2?: boolean;
  };
  last_seen_at?: string | null;
  created_at: string;
}

const statusOf = (s: string): NodeStatus => (s === "online" || s === "draining" || s === "revoked" ? s : "offline");

export function toNode(n: NodeView): NodeItem {
  const c = n.capabilities ?? {};
  const security = [c.seccomp && "seccomp", c.apparmor && "apparmor", c.user_namespaces && "userns", c.cgroup_v2 && "cgroup v2"].filter(
    Boolean,
  ) as string[];
  return {
    id: n.id,
    name: n.name,
    hostname: n.hostname ?? n.name,
    ip_address: n.public_ip ?? "—",
    status: statusOf(n.status),
    version: n.agent_version ?? "",
    fingerprint: "",
    last_seen_at: n.last_seen_at ?? n.created_at,
    enrolled_at: n.created_at,
    capabilities: {
      cpu_cores: c.cpu_cores ?? 0,
      cpu_model: "",
      // The agent reports what the machine has, not how busy it is: the usage page has the load.
      cpu_usage_pct: 0,
      ram_total_bytes: c.memory_bytes ?? 0,
      ram_used_bytes: 0,
      disk_total_bytes: c.disk_total_bytes ?? 0,
      disk_used_bytes: Math.max(0, (c.disk_total_bytes ?? 0) - (c.disk_free_bytes ?? 0)),
      kernel: c.kernel ?? "",
      docker_version: c.docker_version ?? "",
      security_features: security,
    },
  };
}

export async function listNodes(): Promise<NodeItem[]> {
  const res = await apiFetch<{ items: NodeView[] }>("/api/v1/admin/nodes");
  return res.items.map(toNode);
}

/** Asks a node to answer a ping: a liveness check through the whole connection. */
export async function pingNode(id: string): Promise<void> {
  await apiFetch(`/api/v1/admin/nodes/${encodeURIComponent(id)}/ping`, { method: "POST" });
}

/** Revokes a node: its certificates stop working and its connection is closed. */
export async function revokeNode(id: string): Promise<void> {
  await apiFetch(`/api/v1/admin/nodes/${encodeURIComponent(id)}/revoke`, { method: "POST" });
}
