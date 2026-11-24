import { apiFetch } from "@builtpanel/api";
import type { WafMode } from "./types";

/** What applies to a domain, all policies together (`EffectiveView`). */
export interface Effective {
  mode: WafMode;
  /** Which policy decided the mode: `domain`, `tenant`, `global` or `default`. */
  mode_from: string;
  disabled_rules: number[];
}

export interface PolicyAnswer {
  effective: Effective;
  /** What is set at this level itself (headers, rate limit); empty when nothing is. */
  policy?: unknown | null;
}

export interface DomainPolicy {
  domain_id: string;
  fqdn: string;
  effective: Effective;
  /** Whether this domain has a policy of its own. */
  own: boolean;
}

const enc = encodeURIComponent;

export async function listClientDomainPolicies(): Promise<DomainPolicy[]> {
  const res = await apiFetch<{ items: { domain_id: string; fqdn: string; effective: Effective; policy?: unknown | null }[] }>(
    "/api/v1/client/waf/domains",
  );
  return res.items.map((d) => ({ domain_id: d.domain_id, fqdn: d.fqdn, effective: d.effective, own: d.effective.mode_from === "domain" || !!d.policy }));
}

/** Sets the policy of a domain: `detect` only logs, `block` also refuses; rules can be switched off. */
export async function setClientDomainPolicy(domainId: string, mode: "detect" | "block", disabled: number[]): Promise<void> {
  await apiFetch(`/api/v1/client/waf/domains/${enc(domainId)}`, {
    method: "PUT",
    body: JSON.stringify({ mode, disabled_rules: disabled }),
  });
}

/** Removes the domain's own policy: the account's or the panel's applies again. */
export async function clearClientDomainPolicy(domainId: string): Promise<void> {
  await apiFetch(`/api/v1/client/waf/domains/${enc(domainId)}`, { method: "DELETE" });
}

// ---- administrators: the panel-wide policy, an account's, a domain's

export type AdminScope = { kind: "global" } | { kind: "tenant"; id: string } | { kind: "domain"; id: string };

const adminPath = (s: AdminScope) =>
  s.kind === "global" ? "/api/v1/admin/waf/policy" : `/api/v1/admin/waf/${s.kind === "tenant" ? "tenants" : "domains"}/${enc(s.id)}`;

export const getAdminPolicy = (s: AdminScope) => apiFetch<PolicyAnswer>(adminPath(s));

export async function setAdminPolicy(s: AdminScope, mode: "detect" | "block", disabled: number[]): Promise<void> {
  await apiFetch(adminPath(s), { method: "PUT", body: JSON.stringify({ mode, disabled_rules: disabled }) });
}

export async function clearAdminPolicy(s: AdminScope): Promise<void> {
  await apiFetch(adminPath(s), { method: "DELETE" });
}
