import { apiFetch } from "@builtpanel/api";
import type { AdminDashboardSummary } from "./types";

/** A part that does not answer is left at zero: the home page never makes numbers up. */
const get = <T,>(path: string): Promise<T | null> => apiFetch<T>(path).catch(() => null);

interface Node {
  id: string;
  status: string;
}

interface UsagePoint {
  cpu_busy_millis: number;
  memory_total_bytes: number;
  memory_used_bytes: number;
  disk_total_bytes: number;
  disk_used_bytes: number;
}

const GB = 1024 ** 3;
const MB = 1024 ** 2;

/** What each part of the panel says, put together for the administrators' home page. */
export async function loadAdminSummary(): Promise<AdminDashboardSummary> {
  const [nodes, users, domains, dbs, ops, license] = await Promise.all([
    get<{ items: Node[] }>("/api/v1/admin/nodes"),
    get<{ items: { account_status: string; tenant?: { status: string } | null }[]; total: number }>("/api/v1/admin/users?limit=200"),
    get<{ total: number }>("/api/v1/admin/domains?limit=1"),
    get<{ databases: number }>("/api/v1/admin/databases/overview"),
    get<{ items: { id: string; kind: string; status: string; created_at: string; finished_at?: string | null }[] }>(
      "/api/v1/admin/operations?limit=5",
    ),
    get<{ license_status?: string | null; plan?: string | null; mode: string; license_expires_at?: number | null; last_error?: string | null }>(
      "/api/v1/admin/license",
    ),
  ]);

  const nodeList = nodes?.items ?? [];
  const lastPoints = (
    await Promise.all(
      nodeList.map((n) =>
        get<{ points: UsagePoint[]; step_seconds: number }>(`/api/v1/admin/usage/nodes/${encodeURIComponent(n.id)}`).then((u) => ({
          last: u?.points[u.points.length - 1],
          step: u?.step_seconds ?? 60,
        })),
      ),
    )
  ).filter((x) => x.last);

  const sum = (f: (p: UsagePoint) => number) => lastPoints.reduce((acc, x) => acc + f(x.last as UsagePoint), 0);
  const memTotal = sum((p) => p.memory_total_bytes);
  const memUsed = sum((p) => p.memory_used_bytes);
  // CPU: busy milliseconds of the last step against the time that step had.
  const cpuBusy = lastPoints.length
    ? lastPoints.reduce((acc, x) => acc + Math.min(100, ((x.last as UsagePoint).cpu_busy_millis / (x.step * 1000)) * 100), 0) / lastPoints.length
    : 0;

  const tenants = (users?.items ?? []).filter((u) => u.tenant);
  const expires = license?.license_expires_at ? Math.max(0, Math.round((license.license_expires_at * 1000 - Date.now()) / 86400000)) : 0;
  const lic = license?.license_status ?? "";

  return {
    nodes: {
      total: nodeList.length,
      online: nodeList.filter((n) => n.status === "online").length,
      avg_cpu_percent: Math.round(cpuBusy),
      avg_ram_percent: memTotal ? Math.round((memUsed / memTotal) * 100) : 0,
    },
    tenants: {
      total: tenants.length,
      active: tenants.filter((u) => u.tenant?.status === "active").length,
      domains_count: domains?.total ?? 0,
      databases_count: dbs?.databases ?? 0,
    },
    resources: {
      cpu_percent: Math.round(cpuBusy),
      memory_used_mb: Math.round(memUsed / MB),
      memory_total_mb: Math.round(memTotal / MB),
      disk_used_gb: Math.round(sum((p) => p.disk_used_bytes) / GB),
      disk_total_gb: Math.round(sum((p) => p.disk_total_bytes) / GB),
      network_rx_mb: 0,
      network_tx_mb: 0,
    },
    operations: (ops?.items ?? []).map((o) => ({
      id: o.id,
      type: o.kind,
      status: o.status === "succeeded" || o.status === "done" ? "completed" : o.status === "failed" ? "failed" : o.status === "running" || o.status === "dispatched" ? "running" : "pending",
      duration_ms: o.finished_at ? Math.max(0, Date.parse(o.finished_at) - Date.parse(o.created_at)) : undefined,
      created_at: o.created_at,
    })),
    // The security counters have no source yet: the WAF and firewall pages show their own lists.
    security: { blocked_threats_24h: 0, active_waf_rules: 0, ssh_bans: 0 },
    license: {
      tier: license?.plan ?? license?.mode ?? "",
      status: lic === "active" || lic === "" ? "active" : lic === "grace" ? "grace_period" : "restricted",
      signature_valid: !license?.last_error,
      expires_in_days: expires,
    },
  };
}
