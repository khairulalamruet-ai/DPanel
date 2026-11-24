export type MigrationSourceType = "cpanel" | "cyberpanel" | "openpanel" | "builtpanel_archive";

export type TransferStatus = "pending" | "running" | "completed" | "failed" | "cancelled";

export interface TransferStep {
  id: string;
  name: string;
  status: "pending" | "running" | "completed" | "failed";
  started_at?: string;
  completed_at?: string;
  details?: string;
}

export interface TransferJob {
  id: string;
  source_type: MigrationSourceType;
  source_host?: string;
  tenant_username: string;
  target_domain: string;
  status: TransferStatus;
  progress_percent: number;
  current_step: string;
  steps: TransferStep[];
  logs: string[];
  size_bytes?: number;
  created_at: string;
  updated_at: string;
  error_message?: string;
}

export interface RemoteMigrationPayload {
  source_type: MigrationSourceType;
  remote_host: string;
  remote_ssh_port: number;
  remote_user: string;
  auth_method: "password" | "ssh_key";
  password?: string;
  ssh_key?: string;
  remote_backup_path?: string;
  target_domain: string;
  tenant_username: string;
  dry_run: boolean;
}
