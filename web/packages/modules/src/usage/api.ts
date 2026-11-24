import { apiFetch } from "@builtpanel/api";
import type { AdminUsageOverview, ClientUsageSummary, MetricStat, TimeRange, TopTenantUsageItem, UsageDataPoint } from "./types";

interface UsagePoint {
  /** Start of the point (Unix seconds). */
  at: number;
  /** Average thousandths of a CPU. */
  cpu_millis: number;
  memory_bytes: number;
  net_rx_bytes: number;
  net_tx_bytes: number;
}

interface UsageView {
  limits: { cpu_millis?: number | null; memory_bytes?: number | null };
  points: UsagePoint[];
  step_seconds: number;
}

interface NodePoint {
  at: number;
  cpu_busy_millis: number;
  memory_total_bytes: number;
  memory_used_bytes: number;
  disk_total_bytes: number;
  disk_used_bytes: number;
}

const MB = 1024 ** 2;
const GB = 1024 ** 3;

const labelOf = (at: number, range: TimeRange): string => {
  const d = new Date(at * 1000);
  return range === "24h" ? d.toISOString().slice(11, 16) : d.toISOString().slice(5, 10);
};

const stat = (values: number[], limit: number, unit: string): MetricStat => ({
  current: values.length ? values[values.length - 1] : 0,
  average: values.length ? values.reduce((a, b) => a + b, 0) / values.length : 0,
  peak: values.length ? Math.max(...values) : 0,
  limit,
  unit,
});

const round = (n: number, digits = 1) => Math.round(n * 10 ** digits) / 10 ** digits;

export async function loadClientUsage(range: TimeRange): Promise<ClientUsageSummary> {
  const [u, files] = await Promise.all([
    apiFetch<UsageView>(`/api/v1/client/usage?range=${encodeURIComponent(range)}`),
    apiFetch<{ total_bytes: number }>("/api/v1/client/files/usage").catch(() => null),
  ]);
  const cpuLimit = u.limits.cpu_millis ?? 1000;
  const memLimit = u.limits.memory_bytes ?? 0;
  const diskGb = round((files?.total_bytes ?? 0) / GB, 2);
  const history: UsageDataPoint[] = u.points.map((p) => ({
    timestamp: new Date(p.at * 1000).toISOString(),
    label: labelOf(p.at, range),
    cpu_percent: round((p.cpu_millis / cpuLimit) * 100),
    ram_mb: round(p.memory_bytes / MB),
    ram_percent: memLimit ? round((p.memory_bytes / memLimit) * 100) : 0,
    disk_gb: diskGb,
    disk_percent: 0,
    net_in_kbps: round(((p.net_rx_bytes * 8) / 1000) / Math.max(1, u.step_seconds)),
    net_out_kbps: round(((p.net_tx_bytes * 8) / 1000) / Math.max(1, u.step_seconds)),
  }));
  const bytes = u.points.reduce((sum, p) => sum + p.net_rx_bytes + p.net_tx_bytes, 0);
  return {
    range,
    cpu: stat(history.map((h) => h.cpu_percent), 100, "%"),
    ram: stat(history.map((h) => h.ram_mb), round(memLimit / MB), "MB"),
    disk: { current: diskGb, average: diskGb, peak: diskGb, limit: 0, unit: "GB" },
    bandwidth: { current: round(bytes / GB, 2), average: 0, peak: 0, limit: 0, unit: "GB" },
    history,
  };
}

const BY: Record<"cpu" | "ram" | "disk" | "bandwidth", string> = { cpu: "cpu", ram: "memory", disk: "io", bandwidth: "network" };

async function top(by: "cpu" | "ram" | "disk" | "bandwidth", range: TimeRange): Promise<TopTenantUsageItem[]> {
  const res = await apiFetch<{ items: { tenant_id: string; username: string; cpu_millis: number; memory_bytes: number; net_bytes: number; io_bytes: number }[] }>(
    `/api/v1/admin/usage/top?by=${BY[by]}&range=${encodeURIComponent(range)}&limit=10`,
  );
  return res.items.map((i) => ({
    tenant_id: i.tenant_id,
    tenant_name: i.username,
    plan: "",
    cpu_percent: round(i.cpu_millis / 10),
    ram_mb: round(i.memory_bytes / MB),
    ram_limit_mb: 0,
    disk_gb: round(i.io_bytes / GB, 2),
    disk_quota_gb: 0,
    bandwidth_30d_gb: round(i.net_bytes / GB, 2),
    status: "normal",
  }));
}

export async function loadAdminUsage(nodeId: string, nodeName: string, range: TimeRange): Promise<AdminUsageOverview> {
  const [n, cpu, ram, disk, bandwidth, nodes] = await Promise.all([
    apiFetch<{ points: NodePoint[]; step_seconds: number }>(`/api/v1/admin/usage/nodes/${encodeURIComponent(nodeId)}?range=${encodeURIComponent(range)}`),
    top("cpu", range),
    top("ram", range),
    top("disk", range),
    top("bandwidth", range),
    apiFetch<{ items: { id: string; capabilities: { cpu_cores?: number } }[] }>("/api/v1/admin/nodes").catch(() => ({ items: [] })),
  ]);
  const cores = nodes.items.find((x) => x.id === nodeId)?.capabilities?.cpu_cores ?? 0;
  const step = Math.max(1, n.step_seconds);
  const history: UsageDataPoint[] = n.points.map((p) => ({
    timestamp: new Date(p.at * 1000).toISOString(),
    label: labelOf(p.at, range),
    cpu_percent: round(Math.min(100, (p.cpu_busy_millis / (step * 1000)) * 100)),
    ram_mb: round(p.memory_used_bytes / MB),
    ram_percent: p.memory_total_bytes ? round((p.memory_used_bytes / p.memory_total_bytes) * 100) : 0,
    disk_gb: round(p.disk_used_bytes / GB, 2),
    disk_percent: p.disk_total_bytes ? round((p.disk_used_bytes / p.disk_total_bytes) * 100) : 0,
    net_in_kbps: 0,
    net_out_kbps: 0,
  }));
  const last = n.points[n.points.length - 1];
  return {
    node_id: nodeId,
    node_name: nodeName,
    range,
    node_cpu: stat(history.map((h) => h.cpu_percent), cores, "%"),
    node_ram: stat(history.map((h) => round(h.ram_mb / 1024, 1)), last ? round(last.memory_total_bytes / GB, 1) : 0, "GB"),
    node_disk: stat(history.map((h) => h.disk_gb), last ? round(last.disk_total_bytes / GB, 1) : 0, "GB"),
    node_bandwidth: { current: 0, average: 0, peak: 0, limit: 0, unit: "GB" },
    history,
    top_tenants_cpu: cpu,
    top_tenants_ram: ram,
    top_tenants_disk: disk,
    top_tenants_bandwidth: bandwidth,
  };
}
