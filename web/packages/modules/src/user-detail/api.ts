import { apiFetch } from "@builtpanel/api";
import type { TenantActivityEntry, TenantDomainItem, UserDetail } from "./types";

interface UserView {
  id: string;
  username: string;
  email: string;
  account_status: string;
  created_at: string;
  tenant?: {
    id: string;
    username: string;
    node_id: string;
    node_name: string;
    plan_id: string;
    plan_name: string;
    status: string;
  } | null;
}

interface DomainRow {
  id: string;
  fqdn: string;
  unicode: string;
  kind: string;
  docroot: string;
  status: string;
}

const enc = encodeURIComponent;
const get = <T,>(path: string): Promise<T | null> => apiFetch<T>(path).catch(() => null);
const MB = 1024 * 1024;

/** One user's page, made of what the panel knows: the account, its domains, what was done to it. */
export async function loadUserDetail(id: string): Promise<UserDetail> {
  const user = await apiFetch<UserView>(`/api/v1/admin/users/${enc(id)}`);
  const [domains, audit, usage] = await Promise.all([
    get<{ items: DomainRow[] }>(`/api/v1/admin/domains?user_id=${enc(id)}&limit=100`),
    get<{ items: { id: string; action: string; actor_id?: string | null; at: string; outcome: string; metadata: Record<string, unknown> }[] }>(
      `/api/v1/admin/audit-logs?target_id=${enc(id)}&limit=30`,
    ),
    get<{ items: { tenant_id: string; cpu_millis: number; memory_bytes: number }[] }>("/api/v1/admin/usage/top"),
  ]);
  const t = user.tenant;
  const mine = usage?.items.find((u) => u.tenant_id === t?.id);
  const list: TenantDomainItem[] = (domains?.items ?? []).map((d) => ({
    id: d.id,
    domain: d.unicode || d.fqdn,
    is_primary: d.kind === "primary",
    docroot: `/${d.docroot}`,
    ssl_status: "none",
    php_version: "",
  }));
  const activities: TenantActivityEntry[] = (audit?.items ?? []).map((e) => ({
    id: e.id,
    action: e.action,
    actor: e.actor_id ?? "",
    timestamp: e.at,
    status: e.outcome === "success" ? "success" : "failure",
    details: Object.keys(e.metadata ?? {}).length ? JSON.stringify(e.metadata) : "",
  }));
  const status = t?.status ?? user.account_status;
  return {
    id: user.id,
    username: user.username,
    email: user.email,
    status: status === "suspended" || status === "disabled" ? "suspended" : status === "active" ? "active" : "pending",
    uid: 0,
    root_path: "",
    plan_id: t?.plan_id ?? "",
    plan_name: t?.plan_name ?? "",
    node_id: t?.node_id ?? "",
    node_name: t?.node_name ?? "",
    primary_domain: list.find((d) => d.is_primary)?.domain ?? list[0]?.domain,
    created_at: user.created_at,
    resources: {
      cpu_cores: -1,
      cpu_used_pct: 0,
      memory_limit_mb: -1,
      memory_used_mb: mine ? Math.round(mine.memory_bytes / MB) : 0,
      disk_limit_mb: -1,
      disk_used_mb: 0,
      inodes_limit: -1,
      inodes_used: 0,
      bandwidth_limit_mb: -1,
      bandwidth_used_mb: 0,
      pids_limit: -1,
      pids_used: 0,
    },
    container: { id: "", name: t?.username ?? "", status: status === "active" ? "running" : "stopped", image: "", ip_address: "", started_at: "", uptime: "" },
    domains: list,
    databases: [],
    logs: [],
    activities,
  };
}
