import { apiFetch } from "@builtpanel/api";
import type { SslCertificateItem, SslStatus, SslType } from "./types";

interface DomainView {
  id: string;
  fqdn: string;
  unicode: string;
  force_https: boolean;
  hsts: boolean;
  created_at: string;
}

/** A certificate as the backend returns it (`SslView`). */
interface SslView {
  /** `none`, `valid`, `expiring` (ends within 14 days) or `expired`. */
  state: string;
  /** `acme` (the web server got it) or `custom` (uploaded). */
  kind?: string | null;
  issuer?: string | null;
  not_before?: string | null;
  not_after?: string | null;
  days_left?: number | null;
  last_seen_at?: string | null;
}

const enc = encodeURIComponent;

const statusOf = (state: string): SslStatus => {
  switch (state) {
    case "valid":
    case "expiring":
      return "active";
    case "expired":
      return "expired";
    default:
      return "none";
  }
};

const typeOf = (kind: string | null | undefined): SslType =>
  kind === "custom" ? "custom" : kind === "acme" ? "lets_encrypt" : "none";

export function toCertificate(d: DomainView, ssl: SslView): SslCertificateItem {
  const name = d.unicode || d.fqdn;
  return {
    id: d.id,
    domain_id: d.id,
    domain_name: name,
    status: statusOf(ssl.state),
    type: typeOf(ssl.kind),
    issuer: ssl.issuer ?? null,
    subject: d.fqdn,
    sans: [],
    expires_at: ssl.not_after ?? null,
    days_remaining: ssl.days_left ?? 0,
    // The web server renews the certificates it obtained itself.
    auto_renew: ssl.kind !== "custom",
    force_https: d.force_https,
    hsts_enabled: d.hsts,
    created_at: d.created_at,
    updated_at: ssl.last_seen_at ?? d.created_at,
  };
}

async function oneCertificate(d: DomainView): Promise<SslCertificateItem> {
  return toCertificate(d, await apiFetch<SslView>(`/api/v1/client/domains/${enc(d.id)}/ssl`));
}

export async function loadCertificates(): Promise<SslCertificateItem[]> {
  const list = await apiFetch<{ items: DomainView[] }>("/api/v1/client/domains");
  return Promise.all(list.items.map(oneCertificate));
}

export async function loadCertificate(domainId: string): Promise<SslCertificateItem> {
  return oneCertificate(await apiFetch<DomainView>(`/api/v1/client/domains/${enc(domainId)}`));
}

/** Uses an uploaded certificate; the chain, if any, goes after the certificate in the same text. */
export async function installCustomCertificate(
  domainId: string,
  input: { certificate: string; chain?: string | null; privateKey: string },
): Promise<SslCertificateItem> {
  await apiFetch(`/api/v1/client/domains/${enc(domainId)}/ssl/custom`, {
    method: "PUT",
    body: JSON.stringify({
      certificate: [input.certificate.trim(), input.chain?.trim()].filter(Boolean).join("\n"),
      private_key: input.privateKey.trim(),
    }),
  });
  return loadCertificate(domainId);
}

/** Goes back to the web server's own certificate. */
export async function removeCustomCertificate(domainId: string): Promise<SslCertificateItem> {
  await apiFetch(`/api/v1/client/domains/${enc(domainId)}/ssl/custom`, { method: "DELETE" });
  return loadCertificate(domainId);
}

export async function saveHttpsSettings(
  domainId: string,
  settings: { force_https: boolean; hsts: boolean },
): Promise<SslCertificateItem> {
  await apiFetch(`/api/v1/client/domains/${enc(domainId)}`, { method: "PATCH", body: JSON.stringify(settings) });
  return loadCertificate(domainId);
}
