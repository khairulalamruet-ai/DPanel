import { apiFetch } from "@builtpanel/api";
import { kindToType, sslStatus, toUiDocroot, type DomainView } from "./api";
import type { AdminDomainItem, AdminDomainsResponse } from "./types";

interface AdminDomainView extends DomainView {
  /** The domain's account (user) is known by `tenant_id` (hosting account) and its system name. */
  suspended_at?: string | null;
}

export interface AdminDomainFilters {
  page: number;
  perPage: number;
  search?: string;
  status?: string;
  type?: string;
}

export function toAdminDomain(d: AdminDomainView): AdminDomainItem {
  return {
    id: d.id,
    name: d.unicode || d.fqdn,
    type: kindToType(d.kind),
    docroot: toUiDocroot(d.docroot),
    is_primary: d.kind === "primary",
    ssl_status: sslStatus(undefined),
    ssl_expires_at: null,
    ssl_issuer: null,
    dns_status: "active",
    redirect: null,
    created_at: d.created_at,
    tenant_id: d.tenant_id,
    tenant_username: d.tenant_username,
    node_id: "",
    node_name: "",
    status: d.status === "suspended" ? "suspended" : "active",
    suspended_at: d.suspended_at ?? null,
  };
}

export async function listAdminDomains(f: AdminDomainFilters): Promise<AdminDomainsResponse> {
  const q = new URLSearchParams({ limit: String(f.perPage), offset: String((f.page - 1) * f.perPage) });
  if (f.search) q.set("q", f.search);
  if (f.status && f.status !== "all") q.set("status", f.status);
  if (f.type && f.type !== "all") q.set("kind", f.type === "subdomain" ? "sub" : f.type);
  const [res, all] = await Promise.all([
    apiFetch<{ items: AdminDomainView[]; total: number }>(`/api/v1/admin/domains?${q}`),
    // The figures at the top count every domain, not only this page.
    apiFetch<{ total: number }>("/api/v1/admin/domains?limit=1").catch(() => null),
  ]);
  const active = await apiFetch<{ total: number }>("/api/v1/admin/domains?limit=1&status=active").catch(() => null);
  return {
    items: res.items.map(toAdminDomain),
    total: res.total,
    page: f.page,
    per_page: f.perPage,
    total_pages: Math.max(1, Math.ceil(res.total / f.perPage)),
    stats: {
      total_domains: all?.total ?? res.total,
      active_domains: active?.total ?? 0,
      suspended_domains: (all?.total ?? res.total) - (active?.total ?? 0),
      ssl_active_count: 0,
    },
  };
}

const enc = encodeURIComponent;

/** One call per domain; a domain that fails does not stop the others. */
export async function bulkSuspend(ids: string[], suspend: boolean): Promise<{ done: number; failed: number }> {
  let done = 0;
  let failed = 0;
  for (const id of ids) {
    try {
      await apiFetch(`/api/v1/admin/domains/${enc(id)}/${suspend ? "suspend" : "unsuspend"}`, { method: "POST" });
      done++;
    } catch {
      failed++;
    }
  }
  if (done === 0 && failed > 0) throw new Error("None of the domains could be changed");
  return { done, failed };
}

export interface BulkCreateResult {
  created: string[];
  failed: { domain: string; message: string }[];
}

/** Adds the domains to an account one after the other. */
export async function bulkCreateDomains(userId: string, domains: string[]): Promise<BulkCreateResult> {
  const created: string[] = [];
  const failed: { domain: string; message: string }[] = [];
  for (const fqdn of domains) {
    try {
      await apiFetch("/api/v1/admin/domains", { method: "POST", body: JSON.stringify({ user_id: userId, fqdn }) });
      created.push(fqdn);
    } catch (err: any) {
      failed.push({ domain: fqdn, message: err?.message ?? "refused" });
    }
  }
  return { created, failed };
}
