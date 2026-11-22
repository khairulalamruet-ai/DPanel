export type DomainType = "primary" | "addon" | "subdomain" | "alias";
export type SslStatus = "active" | "pending" | "expired" | "self_signed" | "none";
export type DnsStatus = "active" | "pending" | "proxied" | "error";

export interface DomainRedirect {
  enabled: boolean;
  target_url: string;
  status_code: 301 | 302;
}

export interface ClientDomainItem {
  id: string;
  name: string;
  type: DomainType;
  docroot: string;
  is_primary: boolean;
  ssl_status: SslStatus;
  ssl_expires_at?: string | null;
  ssl_issuer?: string | null;
  dns_status: DnsStatus;
  redirect?: DomainRedirect | null;
  php_version?: string;
  created_at: string;
}

export interface ClientDomainsResponse {
  items: ClientDomainItem[];
  total: number;
  limits: {
    max_domains: number; // -1 for unlimited
    used_domains: number;
  };
}

export interface CreateDomainPayload {
  name: string;
  type: DomainType;
  docroot?: string;
  redirect?: DomainRedirect;
  enable_ssl?: boolean;
}

export interface UpdateDocrootPayload {
  docroot: string;
}

export interface UpdateRedirectPayload {
  enabled: boolean;
  target_url: string;
  status_code: 301 | 302;
}

export interface PreviewLinkInfo {
  enabled: boolean;
  domain_id: string;
  target_domain: string;
  url?: string;
  token?: string;
  created_at?: string;
  expires_at?: string;
  is_expired?: boolean;
  /** Whether the preview-link feature is on for this account. */
  feature_available?: boolean;
}

export interface GeneratePreviewLinkPayload {
  ttl_hours?: number;
}

export type DomainStatus = "active" | "suspended";

export interface AdminDomainItem extends ClientDomainItem {
  tenant_id: string;
  tenant_username: string;
  node_id: string;
  node_name: string;
  status: DomainStatus;
  suspended_at?: string | null;
  suspended_reason?: string | null;
}

export interface AdminDomainsResponse {
  items: AdminDomainItem[];
  total: number;
  page: number;
  per_page: number;
  total_pages: number;
  stats: {
    total_domains: number;
    active_domains: number;
    suspended_domains: number;
    ssl_active_count: number;
  };
}

export interface BulkCreateDomainsPayload {
  tenant_id: string;
  node_id: string;
  domains: string[];
  enable_ssl?: boolean;
}

export interface BulkCreateDomainsResponse {
  operation_id: string;
  total_requested: number;
  enqueued_count: number;
  skipped_count: number;
  skipped_domains?: string[];
  status: "queued" | "in_progress" | "completed";
}

