import { apiFetch } from "@builtpanel/api";
import type {
  ClientDomainItem,
  CreateDomainPayload,
  DomainRedirect,
  DomainType,
  PreviewLinkInfo,
  SslStatus,
  UpdateRedirectPayload,
} from "./types";

/** A domain as the backend returns it (`DomainView`). */
export interface DomainView {
  id: string;
  fqdn: string;
  unicode: string;
  /** `primary`, `addon`, `sub` or `alias`. */
  kind: string;
  docroot: string;
  force_https: boolean;
  hsts: boolean;
  status: string;
  parent_domain_id?: string | null;
  tenant_id: string;
  tenant_username: string;
  suspended_at?: string | null;
  created_at: string;
}

interface SslView {
  state: string;
  issuer?: string | null;
  not_after?: string | null;
  days_left?: number | null;
}

interface RedirectView {
  id: string;
  source_path: string;
  target: string;
  code: number;
}

interface PhpInfo {
  account_version: string;
  domains: { id: string; effective_version: string }[];
}

const BASE = "/api/v1/client/domains";
const enc = encodeURIComponent;

export const kindToType = (kind: string): DomainType => (kind === "sub" ? "subdomain" : (kind as DomainType));
export const typeToKind = (type: DomainType): string => (type === "subdomain" ? "sub" : type);

/** The panel shows paths from the account's files root (`/public_html`); the backend keeps them relative. */
export const toUiDocroot = (docroot: string): string => `/${docroot}`.replace(/\/+$/, "") || "/";
export const toApiDocroot = (docroot: string): string => docroot.trim().replace(/^\/+/, "").replace(/\/+$/, "");

export function sslStatus(state: string | undefined): SslStatus {
  switch (state) {
    case "valid":
    case "expiring":
      return "active";
    case "expired":
      return "expired";
    default:
      return "none";
  }
}

/** The front-page redirect (`/`) is the one the panel shows as "the" redirect of a domain. */
const frontRedirect = (items: RedirectView[]): RedirectView | undefined => items.find((r) => r.source_path === "/");

export function toClientDomain(
  d: DomainView,
  extra: { ssl?: SslView; redirects?: RedirectView[]; php?: string } = {},
): ClientDomainItem {
  const r = frontRedirect(extra.redirects ?? []);
  return {
    id: d.id,
    name: d.unicode || d.fqdn,
    type: kindToType(d.kind),
    docroot: toUiDocroot(d.docroot),
    is_primary: d.kind === "primary",
    ssl_status: sslStatus(extra.ssl?.state),
    ssl_expires_at: extra.ssl?.not_after ?? null,
    ssl_issuer: extra.ssl?.issuer ?? null,
    // The panel does not look names up for the client; the DNS page has the check.
    dns_status: "active",
    redirect: r ? { enabled: true, target_url: r.target, status_code: (r.code === 302 ? 302 : 301) as 301 | 302 } : null,
    php_version: extra.php ? `PHP ${extra.php}` : undefined,
    created_at: d.created_at,
  };
}

export async function loadClientDomains(): Promise<{ items: ClientDomainItem[]; total: number }> {
  const list = await apiFetch<{ items: DomainView[]; total: number }>(BASE);
  const php = await apiFetch<PhpInfo>("/api/v1/client/php").catch(() => null);
  const items = await Promise.all(
    list.items.map(async (d) => {
      const [ssl, redirects] = await Promise.all([
        apiFetch<SslView>(`${BASE}/${enc(d.id)}/ssl`).catch(() => undefined),
        apiFetch<{ items: RedirectView[] }>(`${BASE}/${enc(d.id)}/redirects`).catch(() => ({ items: [] })),
      ]);
      return toClientDomain(d, {
        ssl,
        redirects: redirects.items,
        php: php?.domains.find((x) => x.id === d.id)?.effective_version,
      });
    }),
  );
  return { items, total: list.total };
}

export async function createDomain(payload: CreateDomainPayload): Promise<void> {
  const created = await apiFetch<DomainView>(BASE, {
    method: "POST",
    body: JSON.stringify({
      fqdn: payload.name,
      kind: typeToKind(payload.type),
      ...(payload.docroot ? { docroot: toApiDocroot(payload.docroot) } : {}),
    }),
  });
  if (payload.redirect?.enabled) {
    await setRedirect(created.id, payload.redirect);
  }
}

export async function updateDocroot(domainId: string, docroot: string): Promise<void> {
  await apiFetch(`${BASE}/${enc(domainId)}`, { method: "PATCH", body: JSON.stringify({ docroot: toApiDocroot(docroot) }) });
}

/** Turns the front-page redirect on, changes it or (`enabled: false`) removes it. */
export async function setRedirect(domainId: string, payload: DomainRedirect | UpdateRedirectPayload): Promise<void> {
  const { items } = await apiFetch<{ items: RedirectView[] }>(`${BASE}/${enc(domainId)}/redirects`);
  const current = frontRedirect(items);
  if (!payload.enabled) {
    if (current) await apiFetch(`${BASE}/${enc(domainId)}/redirects/${enc(current.id)}`, { method: "DELETE" });
    return;
  }
  if (current) {
    await apiFetch(`${BASE}/${enc(domainId)}/redirects/${enc(current.id)}`, {
      method: "PATCH",
      body: JSON.stringify({ target: payload.target_url, code: payload.status_code }),
    });
  } else {
    await apiFetch(`${BASE}/${enc(domainId)}/redirects`, {
      method: "POST",
      body: JSON.stringify({ source_path: "/", target: payload.target_url, code: payload.status_code }),
    });
  }
}

export async function deleteDomain(domainId: string): Promise<void> {
  await apiFetch(`${BASE}/${enc(domainId)}`, { method: "DELETE" });
}

/** Force-HTTPS and HSTS of a domain (the certificate itself is kept by the web server). */
export async function setHttpsPolicy(domainId: string, policy: { force_https: boolean; hsts: boolean }): Promise<void> {
  await apiFetch(`${BASE}/${enc(domainId)}`, { method: "PATCH", body: JSON.stringify(policy) });
}

// ---- preview links

interface PreviewView {
  id: string;
  host: string;
  expires_at: string;
}

export async function getPreview(domain: { id: string; name: string }): Promise<PreviewLinkInfo> {
  const res = await apiFetch<{ enabled: boolean; items: PreviewView[] }>(`${BASE}/${enc(domain.id)}/previews`);
  // Several links may exist; the panel shows the one that lasts longest.
  const latest = [...res.items].sort((a, b) => b.expires_at.localeCompare(a.expires_at))[0];
  return toPreviewInfo(domain, latest, res.enabled);
}

export async function createPreview(domain: { id: string; name: string }): Promise<PreviewLinkInfo> {
  const p = await apiFetch<PreviewView>(`${BASE}/${enc(domain.id)}/previews`, { method: "POST" });
  return toPreviewInfo(domain, p, true);
}

export async function deletePreview(domain: { id: string; name: string }): Promise<void> {
  const res = await apiFetch<{ items: PreviewView[] }>(`${BASE}/${enc(domain.id)}/previews`);
  await Promise.all(res.items.map((p) => apiFetch(`${BASE}/${enc(domain.id)}/previews/${enc(p.id)}`, { method: "DELETE" })));
}

function toPreviewInfo(domain: { id: string; name: string }, p: PreviewView | undefined, featureOn: boolean): PreviewLinkInfo {
  if (!p) {
    return { enabled: false, domain_id: domain.id, target_domain: domain.name, feature_available: featureOn };
  }
  return {
    enabled: true,
    domain_id: domain.id,
    target_domain: domain.name,
    url: `https://${p.host}`,
    expires_at: p.expires_at,
    is_expired: new Date(p.expires_at).getTime() < Date.now(),
    feature_available: featureOn,
  };
}
