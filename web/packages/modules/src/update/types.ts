export type UpdateChannel = "stable" | "beta";

/** What the update service is doing (or did last), as `GET /api/v1/admin/update/status` reports it. */
export interface UpdateRun {
  state: "running" | "completed" | "rolled_back" | "failed";
  step: "request" | "downloading" | "verifying" | "completed" | "rolled_back" | "failed";
  version: string;
  from_version: string;
  message: string;
  updated_at: string;
}

export interface SignedUpdateManifest {
  version: string;
  release_date: string;
  channel: UpdateChannel;
  ed25519_signature: string;
  key_id: string;
  checksum_sha256: string;
  changelog: string[];
  required_migrations: string[];
  can_rollback: boolean;
}

export interface UpdateHistoryItem {
  id: string;
  from_version: string;
  to_version: string;
  channel: UpdateChannel;
  status: "success" | "rolled_back" | "failed";
  timestamp: string;
  details: string;
}

export interface PanelUpdateStatus {
  current_version: string;
  channel: UpdateChannel;
  last_checked_at?: string | null;
  available_update: SignedUpdateManifest | null;
  previous_version?: string | null;
  can_rollback: boolean;
  /** This server has the update service; without it the installer is run again to update. */
  can_apply: boolean;
  run?: UpdateRun | null;
  history: UpdateHistoryItem[];
}
