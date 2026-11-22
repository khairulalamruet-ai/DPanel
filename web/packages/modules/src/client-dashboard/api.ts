import { apiFetch } from "@builtpanel/api";
import type {
  ClientDashboardData,
  ClientRecentEvent,
  ClientSslWarning,
  ClientLastBackupInfo,
  ClientShortcut,
} from "./types";

/**
 * What the customer's home page shows. The backend has no single answer for it: the page is
 * made from what each part of the panel says, and a part that does not answer is left out
 * rather than breaking the page.
 */

interface Me {
  id: string;
  username: string;
  email: string;
}

interface DomainView {
  id: string;
  fqdn: string;
  unicode: string;
  kind: string;
  docroot: string;
  created_at: string;
}

interface Ssl {
  state: string;
  issuer?: string | null;
  days_left?: number | null;
}

const SHORTCUTS: ClientShortcut[] = [
  { id: "files", title: "File Manager", icon: "files", href: "/files", phase: 8 },
  { id: "domains", title: "Domains", icon: "domains", href: "/domains", phase: 5 },
  { id: "databases", title: "Databases", icon: "databases", href: "/databases", phase: 7 },
  { id: "ssl", title: "SSL/TLS", icon: "ssl", href: "/ssl", phase: 5 },
  { id: "backups", title: "Backups", icon: "backups", href: "/backups", phase: 9 },
  { id: "cron", title: "Cron Jobs", icon: "cron", href: "/cron", phase: 10 },
  { id: "dns", title: "DNS", icon: "dns", href: "/dns", phase: 5 },
  { id: "logs", title: "Logs", icon: "logs", href: "/logs", phase: 10 },
];

const get = <T,>(path: string): Promise<T | null> => apiFetch<T>(path).catch(() => null);
const mb = (bytes: number) => Math.round(bytes / 1048576);

export async function loadClientDashboard(limits: Record<string, number> = {}): Promise<ClientDashboardData> {
  const [me, domains, services, usage, files, dbs, backups, activity, ns, php, mail] = await Promise.all([
    get<Me>("/api/v1/auth/me"),
    get<{ items: DomainView[] }>("/api/v1/client/domains"),
    get<{ items: { name: string; role: string; state: string; image: string }[]; operation_id?: string }>("/api/v1/client/services"),
    get<{ limits: { cpu_millis?: number | null; memory_bytes?: number | null }; points: { cpu_millis: number; memory_bytes: number }[] }>(
      "/api/v1/client/usage",
    ),
    get<{ total_bytes: number; inodes: number }>("/api/v1/client/files/usage"),
    get<{ items: unknown[]; limit?: number | null }>("/api/v1/client/databases"),
    get<{ items: { created_at: string; status: string; size_bytes?: number | null; trigger: string }[] }>("/api/v1/client/backups"),
    get<{ items: { id: string; action: string; at: string; ip?: string | null; outcome: string }[] }>("/api/v1/client/activity"),
    get<{ nameservers: string[] }>("/api/v1/client/dns/nameservers"),
    get<{ account_version: string }>("/api/v1/client/php"),
    get<{ items: unknown[]; limit?: number | null }>("/api/v1/email/mailboxes"),
  ]);

  const list = domains?.items ?? [];
  const primary = list.find((d) => d.kind === "primary") ?? list[0];
  const ssls = await Promise.all(
    list.map(async (d) => ({ d, ssl: await get<Ssl>(`/api/v1/client/domains/${encodeURIComponent(d.id)}/ssl`) })),
  );
  const primarySsl = ssls.find((x) => x.d.id === primary?.id)?.ssl;

  const containers = services?.items ?? [];
  const running = containers.length > 0 && containers.every((c) => c.state === "running");
  const last = usage?.points[usage.points.length - 1];
  const cpuLimit = usage?.limits.cpu_millis ?? 0;
  const memLimit = usage?.limits.memory_bytes ?? 0;

  const ssl_warnings: ClientSslWarning[] = ssls
    .filter((x) => x.ssl && (x.ssl.state === "expiring" || x.ssl.state === "expired"))
    .map((x) => ({
      domain: x.d.unicode || x.d.fqdn,
      issuer: x.ssl?.issuer ?? "",
      expires_in_days: x.ssl?.days_left ?? 0,
      severity: x.ssl?.state === "expired" ? "critical" : "warning",
    }));

  const newest = backups?.items.find((b) => b.status === "ready") ?? backups?.items[0];
  const last_backup: ClientLastBackupInfo | undefined = newest
    ? {
        created_at: newest.created_at,
        size_mb: mb(newest.size_bytes ?? 0),
        status: newest.status === "ready" ? "completed" : newest.status === "failed" ? "failed" : "in_progress",
        type: newest.trigger === "manual" ? "manual" : "automatic",
      }
    : undefined;

  const recent_events: ClientRecentEvent[] = (activity?.items ?? []).slice(0, 8).map((e) => ({
    id: e.id,
    action: e.action,
    timestamp: e.at,
    ip_address: e.ip ?? "",
    description: `${e.action} (${e.outcome})`,
  }));

  return {
    tenant: {
      id: me?.id ?? "",
      username: me?.username ?? "",
      email: me?.email ?? "",
      primary_domain: primary?.unicode ?? primary?.fqdn ?? "",
      plan_name: "",
      node_name: "",
      server_ip: "",
      nameservers: ns?.nameservers ?? [],
      created_at: primary?.created_at ?? "",
      status: "active",
    },
    container: {
      status: containers.length === 0 ? "stopped" : running ? "running" : "degraded",
      name: containers[0]?.name ?? "",
      image: containers[0]?.image ?? "",
      php_version: php?.account_version ?? "",
      uptime: "",
      ip_address: "",
    },
    resources: {
      // The disk and bandwidth quotas are the plan's; the dashboard shows use against what is known.
      disk_limit_mb: limits.disk_mb ?? 0,
      disk_used_mb: mb(files?.total_bytes ?? 0),
      bandwidth_limit_mb: -1,
      bandwidth_used_mb: 0,
      memory_limit_mb: mb(memLimit),
      memory_used_mb: mb(last?.memory_bytes ?? 0),
      cpu_cores: cpuLimit ? Math.max(1, Math.round(cpuLimit / 1000)) : 1,
      cpu_used_pct: cpuLimit && last ? Math.min(100, Math.round((last.cpu_millis / cpuLimit) * 100)) : 0,
      inodes_limit: limits.inodes ?? -1,
      inodes_used: files?.inodes ?? 0,
      databases_limit: dbs?.limit ?? limits.max_databases ?? -1,
      databases_used: dbs?.items.length ?? 0,
      domains_limit: limits.max_domains ?? -1,
      domains_used: list.length,
      emails_limit: mail?.limit ?? limits.max_mailboxes ?? -1,
      emails_used: mail?.items.length ?? 0,
    },
    primary_domain: {
      domain: primary?.unicode ?? primary?.fqdn ?? "",
      docroot: primary ? `/${primary.docroot}` : "",
      ssl_status: primarySsl?.state ?? "none",
      ssl_issuer: primarySsl?.issuer ?? "",
      ssl_expires_in_days: primarySsl?.days_left ?? 0,
      php_version: php?.account_version ? `PHP ${php.account_version}` : "",
    },
    shortcuts: SHORTCUTS,
    recent_events,
    last_backup,
    ssl_warnings,
  };
}
