/** `active` and `suspended` are derived; the rest is the backend status (`pending`, `failed`, `deleting`). */
export type FtpAccountStatus = string;

export interface FtpAccount {
  id: string;
  username: string;
  home_directory: string;
  quota_bytes: number; // -1 for unlimited
  quota_formatted: string;
  used_bytes: number;
  used_formatted: string;
  status: FtpAccountStatus;
  /** Protocols the account may use. */
  access: string;
}

export interface FtpConnectionInfo {
  host: string;
  port: number;
  protocol: string;
  encryption: string;
  sftp_port?: number;
}

export interface CreateFtpAccountPayload {
  username: string;
  password: string;
  home_directory: string;
  quota_mb: number; // -1 for unlimited
}

export interface ChangeFtpPasswordPayload {
  password: string;
}

export interface UpdateFtpQuotaPayload {
  quota_mb: number;
}
