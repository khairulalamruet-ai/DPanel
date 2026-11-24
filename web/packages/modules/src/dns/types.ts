export type DnsRecordType =
  | "A"
  | "AAAA"
  | "CNAME"
  | "MX"
  | "TXT"
  | "SRV"
  | "CAA"
  | "NS";

export interface DnsRecord {
  id: string;
  zone_id: string;
  name: string; // e.g. "@", "www", "mail", "sub"
  type: DnsRecordType;
  content: string; // IPv4, IPv6, FQDN target, TXT string, etc.
  ttl: number; // e.g. 300, 3600, 86400
  priority?: number | null; // For MX, SRV
  weight?: number | null; // For SRV
  port?: number | null; // For SRV
  flags?: number | null; // For CAA
  tag?: string | null; // For CAA ("issue", "issuewild", "iodef")
  created_at: string;
  updated_at: string;
}

export interface DnsZone {
  id: string;
  domain_id: string;
  domain_name: string;
  tenant_id: string;
  nameservers: string[];
  soa_serial: number;
  records_count: number;
  is_custom_ns: boolean;
  dnssec_enabled: boolean;
  updated_at: string;
  /** `pending` (changes not on the DNS server yet), `active` or `error`. */
  status?: string;
  last_error?: string | null;
}

export interface DnsZoneDetailResponse {
  zone: DnsZone;
  records: DnsRecord[];
}

export interface CreateDnsRecordPayload {
  name: string;
  type: DnsRecordType;
  content: string;
  ttl: number;
  priority?: number | null;
  weight?: number | null;
  port?: number | null;
  flags?: number | null;
  tag?: string | null;
}

export interface UpdateDnsRecordPayload {
  name?: string;
  type?: DnsRecordType;
  content?: string;
  ttl?: number;
  priority?: number | null;
  weight?: number | null;
  port?: number | null;
  flags?: number | null;
  tag?: string | null;
}

export interface BindImportPayload {
  bind_text: string;
  overwrite: boolean;
}

export interface BindImportResponse {
  imported_count: number;
  skipped_count: number;
  warnings: string[];
  records: DnsRecord[];
}

export interface BindExportResponse {
  zone_name: string;
  soa_serial: number;
  bind_text: string;
}

// --- DNS Cluster Types (T16.04) ---

export type DnsServerRole = "primary" | "secondary" | "replica";
export type DnsServerStatus = "online" | "offline" | "syncing" | "error";
export type DnsSyncStrategy = "native_db" | "axfr" | "api_push";

export interface DnsClusterServer {
  id: string;
  name: string;
  hostname: string;
  ip_address: string;
  port: number;
  api_url: string;
  role: DnsServerRole;
  status: DnsServerStatus;
  sync_strategy: DnsSyncStrategy;
  zones_count: number;
  last_sync_at: string;
  latency_ms: number;
  version?: string;
  is_draining?: boolean;
}

export interface DnsClusterOverview {
  total_servers: number;
  online_servers: number;
  primary_count: number;
  secondary_count: number;
  total_zones: number;
  cluster_healthy: boolean;
  replication_lag_ms: number;
}

export interface CreateDnsServerPayload {
  name: string;
  hostname: string;
  ip_address: string;
  port?: number;
  api_url: string;
  api_key?: string;
  role: DnsServerRole;
  sync_strategy: DnsSyncStrategy;
}

export interface DnsResolutionTestPayload {
  domain: string;
  record_type?: string;
  simulate_offline_server_id?: string | null;
}

export interface DnsResolutionTestResult {
  domain: string;
  record_type: string;
  resolved: boolean;
  answering_server: string;
  answering_ip: string;
  offline_servers_detected: string[];
  redundancy_active: boolean;
  records: string[];
  resolution_time_ms: number;
  message: string;
}
