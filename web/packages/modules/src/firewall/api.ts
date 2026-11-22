import { apiFetch } from "@builtpanel/api";
import type { AdminFirewallOverview, GlobalIpBan, NodePortRule } from "./types";

/** A node's firewall policy as the backend keeps it (`FirewallPolicy`). */
export interface FirewallPolicy {
  /** Addresses allowed to everything (before `blocked` is looked at). */
  allowed?: string[];
  /** Addresses that always get in; with `default_deny` at least one is needed. */
  always_allow?: string[];
  /** Addresses refused everything. */
  blocked?: string[];
  default_deny?: boolean;
  open_ports?: { port: number; to?: number | null; proto: string; from?: string[] }[];
  rate_limits?: { port: number; proto: string; per_minute: number }[];
}

const enc = encodeURIComponent;
const path = (nodeId: string) => `/api/v1/admin/nodes/${enc(nodeId)}/firewall`;

const KNOWN: Record<number, string> = {
  22: "SSH",
  25: "SMTP",
  53: "DNS",
  80: "HTTP",
  110: "POP3",
  143: "IMAP",
  443: "HTTPS",
  465: "SMTPS",
  587: "Submission",
  993: "IMAPS",
  995: "POP3S",
  2022: "SFTP",
  3306: "MariaDB",
  5432: "PostgreSQL",
};

export async function getPolicy(nodeId: string): Promise<FirewallPolicy> {
  const res = await apiFetch<{ policy?: FirewallPolicy | null }>(path(nodeId));
  return res.policy ?? {};
}

/** The node is told within seconds (or when it connects). */
export async function putPolicy(nodeId: string, policy: FirewallPolicy): Promise<void> {
  await apiFetch(path(nodeId), { method: "PUT", body: JSON.stringify(policy) });
}

export function toOverview(nodeId: string, p: FirewallPolicy): AdminFirewallOverview {
  const active_ports: NodePortRule[] = (p.open_ports ?? []).map((r) => ({
    id: `${r.proto}/${r.port}${r.to ? `-${r.to}` : ""}`,
    port: r.port,
    protocol: r.proto === "udp" ? "udp" : "tcp",
    service_name: KNOWN[r.port] ?? (r.from?.length ? r.from.join(", ") : ""),
    is_system_protected: false,
    status: "open",
  }));
  const active_bans: GlobalIpBan[] = (p.blocked ?? []).map((ip) => ({
    id: ip,
    ip,
    reason: "",
    source: "admin_manual",
    service: "manual",
    banned_at: "",
    expires_at: null,
  }));
  return {
    node_id: nodeId,
    status: "active",
    nftables_version: "",
    active_ports,
    active_bans,
    rate_limit_enabled: (p.rate_limits ?? []).length > 0,
    rate_limit_rps: Math.round(((p.rate_limits ?? [])[0]?.per_minute ?? 0) / 60),
  };
}

export async function loadFirewall(nodeId: string): Promise<{ overview: AdminFirewallOverview; policy: FirewallPolicy }> {
  const policy = await getPolicy(nodeId);
  return { overview: toOverview(nodeId, policy), policy };
}

export async function openPort(nodeId: string, port: number, proto: "tcp" | "udp"): Promise<void> {
  const p = await getPolicy(nodeId);
  const open = (p.open_ports ?? []).filter((r) => !(r.port === port && r.proto === proto && !r.to));
  await putPolicy(nodeId, { ...p, open_ports: [...open, { port, proto }] });
}

export async function closePort(nodeId: string, id: string): Promise<void> {
  const p = await getPolicy(nodeId);
  const keep = (p.open_ports ?? []).filter((r) => `${r.proto}/${r.port}${r.to ? `-${r.to}` : ""}` !== id);
  await putPolicy(nodeId, { ...p, open_ports: keep });
}

export async function blockAddress(nodeId: string, ip: string): Promise<void> {
  const p = await getPolicy(nodeId);
  const blocked = Array.from(new Set([...(p.blocked ?? []), ip]));
  await putPolicy(nodeId, { ...p, blocked });
}

export async function unblockAddress(nodeId: string, ip: string): Promise<void> {
  const p = await getPolicy(nodeId);
  await putPolicy(nodeId, { ...p, blocked: (p.blocked ?? []).filter((a) => a !== ip) });
}
