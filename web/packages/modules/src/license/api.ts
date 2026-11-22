import { apiFetch } from "@builtpanel/api";
import type { LicenseState, LicenseStatus, LicensePlanTier, RecoveryRequestResponse } from "./types";

/** The license as the backend returns it (`LicenseView`); dates are Unix seconds. */
interface LicenseView {
  enforced: boolean;
  /** `unlicensed`, `active`, `grace`, `restricted`. */
  mode: string;
  plan?: string | null;
  installation_id?: string | null;
  license_status?: string | null;
  license_expires_at?: number | null;
  entitlement_expires_at?: number | null;
  grace_ends_at?: number | null;
  last_confirmed_at?: number | null;
  features: string[];
  limits: Record<string, number> | null;
  last_error?: string | null;
  reason?: string | null;
}

const iso = (s?: number | null): string => (s ? new Date(s * 1000).toISOString() : "");

const statusOf = (v: LicenseView): LicenseStatus => {
  if (v.mode === "grace") return "grace_period";
  if (v.mode === "restricted") return v.license_status === "suspended" ? "suspended" : "restricted";
  if (v.license_status === "suspended") return "suspended";
  if (v.license_status === "revoked") return "expired";
  return "active";
};

const tierOf = (plan?: string | null): LicensePlanTier => {
  const p = (plan ?? "").toLowerCase();
  return p.includes("enterprise") ? "enterprise" : p.includes("pro") ? "pro" : p.includes("dev") ? "internal_dev" : "starter";
};

export function toLicense(v: LicenseView): LicenseState {
  const limit = (key: string) => v.limits?.[key] ?? -1;
  const graceMs = v.grace_ends_at ? v.grace_ends_at * 1000 - Date.now() : 0;
  return {
    license_id: v.installation_id ?? "",
    // The key is not kept in the panel after it has been activated.
    license_key: "",
    installation_id: v.installation_id ?? "",
    customer_name: "",
    customer_email: "",
    product_name: "Built Panel",
    plan_tier: tierOf(v.plan),
    status: statusOf(v),
    issued_at: "",
    expires_at: iso(v.license_expires_at ?? v.entitlement_expires_at),
    grace_period_expires_at: v.grace_ends_at ? iso(v.grace_ends_at) : undefined,
    last_heartbeat_at: iso(v.last_confirmed_at),
    next_refresh_at: "",
    signing_key_id: "",
    signature_valid: !v.last_error,
    features: {
      reseller_enabled: v.features.includes("reseller"),
      clustering_enabled: v.features.includes("clustering"),
      advanced_backups: v.features.includes("advanced_backups"),
      commercial_waf: v.features.includes("commercial_waf"),
      whitelabel_enabled: v.features.includes("whitelabel"),
      api_access: v.features.includes("api_access"),
      max_nodes: limit("max_nodes"),
      max_tenants: limit("max_tenants"),
      max_domains: limit("max_domains"),
    },
    usage: { nodes_used: 0, tenants_used: 0, domains_used: 0 },
    offline_grace_hours_remaining: v.mode === "grace" ? Math.max(0, Math.round(graceMs / 3600000)) : undefined,
    restricted_reason: v.reason ?? v.last_error ?? undefined,
  };
}

const L = "/api/v1/admin/license";

export const getLicense = async () => toLicense(await apiFetch<LicenseView>(L));

/** Asks the authority again now, to check the license or after it was changed there. */
export const refreshLicense = async () => toLicense(await apiFetch<LicenseView>(`${L}/refresh`, { method: "POST" }));

export const activateLicense = async (licenseKey: string) =>
  toLicense(await apiFetch<LicenseView>(`${L}/activate`, { method: "POST", body: JSON.stringify({ license_key: licenseKey }) }));

/** Gives the place back so the license can be used on another installation. */
export const deactivateLicense = async () => toLicense(await apiFetch<LicenseView>(`${L}/deactivate`, { method: "POST" }));

/** Asks the license authority's people for help (a failed update, a lost machine). */
export async function requestRecovery(p: { reason: string; contact_email: string; notes?: string }): Promise<RecoveryRequestResponse> {
  const note = [p.reason, p.contact_email && `Contact: ${p.contact_email}`, p.notes].filter(Boolean).join("\n");
  await apiFetch(`${L}/recovery-request`, { method: "POST", body: JSON.stringify({ note }) });
  return { ticket_id: "", status: "pending", created_at: new Date().toISOString(), message: "Sent" };
}
