export type SslStatus = "active" | "pending" | "expired" | "self_signed" | "none";
export type SslType = "lets_encrypt" | "zerossl" | "custom" | "self_signed" | "none";

export interface SslCertificateItem {
  id: string;
  domain_id: string;
  domain_name: string;
  status: SslStatus;
  type: SslType;
  issuer: string | null;
  subject: string;
  sans: string[];
  expires_at: string | null;
  days_remaining: number;
  auto_renew: boolean;
  force_https: boolean;
  hsts_enabled: boolean;
  created_at: string;
  updated_at: string;
  cert_fingerprint?: string | null;
}
