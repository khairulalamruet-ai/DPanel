export interface ServerMemoryInfo {
  total_mb: number;
  free_mb: number;
  available_mb: number;
  buffers_mb: number;
  cached_mb: number;
}

export interface ServerSwapInfo {
  total_mb: number;
  used_mb: number;
  free_mb: number;
  swappiness: number;
}

export interface ServerOverview {
  node_id: string;
  hostname: string;
  os: string;
  kernel: string;
  uptime_seconds: number;
  uptime_human: string;
  current_time: string;
  timezone: string;
  memory: ServerMemoryInfo;
  swap: ServerSwapInfo;
}

export interface SshConfigInfo {
  port: number;
  permit_root_login: "yes" | "no" | "prohibit-password";
  password_authentication: boolean;
  pubkey_authentication: boolean;
  max_auth_tries: number;
  authorized_keys_count: number;
  ciphers: string[];
  security_rating: "hardened" | "moderate" | "weak";
}

export interface SshBasicConfig {
  port: number;
  permit_root_login: "yes" | "no" | "prohibit-password";
  password_authentication: boolean;
  pubkey_authentication: boolean;
  max_auth_tries?: number;
}

export interface SshAuthorizedKey {
  id: string;
  name: string;
  key_type: string;
  fingerprint: string;
  public_key: string;
  created_at: string;
}

export type RebootMode = "graceful" | "hard";

export interface MigrationRequest {
  remote_host: string;
  port: number;
  user: string;
  password?: string;
  transfer_accounts?: boolean;
  transfer_configs?: boolean;
  transfer_databases?: boolean;
}

export interface MigrationStep {
  id: string;
  name: string;
  status: "pending" | "in_progress" | "completed" | "failed";
}

export interface MigrationStatus {
  id: string;
  status: "idle" | "running" | "connecting" | "transferring" | "verifying" | "completed" | "failed";
  progress: number;
  current_step: string;
  logs: string[];
  steps: MigrationStep[];
  started_at?: string;
  error?: string;
}

export interface JoinServerRequest {
  address: string;
  name: string;
  join_code: string;
}

export interface JoinCodeResult {
  code: string;
  expires_at: string;
  install_command: string;
}

export interface ClusterNodeSummary {
  id: string;
  name: string;
  hostname: string;
  ip_address?: string;
  status: "online" | "offline" | "draining" | "revoked";
  role: "primary" | "worker";
  agent_version: string;
  last_ping_at?: string;
}

export interface ServerAuditEvent {
  id: string;
  action: string;
  actor: string;
  details: string;
  created_at: string;
  status: "success" | "failure";
}
