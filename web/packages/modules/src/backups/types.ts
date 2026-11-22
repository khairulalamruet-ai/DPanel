export type BackupStatus = "completed" | "in_progress" | "failed";
export type BackupType = "full" | "files" | "databases";

export interface BackupManifest {
  domains: string[];
  databases: string[];
  has_mail?: boolean;
  has_dns?: boolean;
}

export interface BackupItem {
  id: string;
  name: string;
  type: BackupType;
  size_bytes: number;
  size_formatted: string;
  status: BackupStatus;
  created_at: string;
  completed_at?: string;
  destination: string;
  download_url?: string;
  sha256: string;
  files_count?: number;
  databases_count?: number;
  manifest: BackupManifest;
  /** Why it failed. */
  error?: string;
}

export interface CreateBackupPayload {
  type: BackupType;
  include_databases: boolean;
  include_files: boolean;
  include_mail: boolean;
  note?: string;
}

export interface RestoreBackupPayload {
  backup_id: string;
  restore_mode: "all" | "files_only" | "databases_only" | "selective";
  selected_databases?: string[];
  selected_domains?: string[];
  take_snapshot_before_restore: boolean;
}

// Admin Backup Types (T09.10)
export type BackupDestinationType = "local" | "s3" | "sftp";

export interface BackupDestination {
  id: string;
  name: string;
  type: BackupDestinationType;
  is_default: boolean;
  status: "connected" | "error";
  config_summary: string;
  last_tested_at?: string;
  error_message?: string;
}

export interface BackupSchedule {
  id: string;
  name: string;
  cron_expression: string;
  cron_readable: string;
  destination_id: string;
  destination_name: string;
  retention_daily: number;
  retention_weekly: number;
  retention_monthly: number;
  scope: "all_tenants" | "system" | "selective";
  enabled: boolean;
  last_run_at?: string;
  next_run_at: string;
}

export interface BackupJob {
  id: string;
  job_type: "tenant" | "system";
  target_name: string; // e.g. "tenant:demo" or "system:control-plane"
  destination_name: string;
  status: "completed" | "in_progress" | "failed";
  size_bytes: number;
  size_formatted: string;
  started_at: string;
  completed_at?: string;
  error_message?: string;
  notification_sent: boolean; // DoD requirement
}

export interface AdminBackupOverview {
  total_jobs: number;
  active_jobs: number;
  failed_jobs: number;
  destinations_count: number;
  schedules_count: number;
  last_system_backup_at?: string;
  last_system_backup_size: string;
  system_backup_healthy: boolean;
}

