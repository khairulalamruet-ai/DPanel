import { apiFetch } from "@builtpanel/api";
import type { FtpAccount, FtpConnectionInfo } from "./types";

/** An account as the backend returns it (`FtpAccountView`). */
interface FtpAccountView {
  id: string;
  name: string;
  home: string;
  quota_mb?: number | null;
  enabled: boolean;
  status: string;
  sftp: boolean;
  ftps: boolean;
  ftp_plain: boolean;
  read_only: boolean;
}

interface FtpAccountList {
  items: FtpAccountView[];
  host: string;
  ftp_port: number;
  sftp_port: number;
  prefix: string;
  limit?: number | null;
  plain_allowed: boolean;
}

interface AccountWithPassword {
  account: FtpAccountView;
  password?: string | null;
}

const BASE = "/api/v1/client/ftp-accounts";
const MIB = 1024 * 1024;

function formatMb(mb: number): string {
  return mb >= 1024 ? `${(mb / 1024).toFixed(1)} GB` : `${mb} MB`;
}

export function toFtpAccount(a: FtpAccountView): FtpAccount {
  const unlimited = a.quota_mb == null || a.quota_mb <= 0;
  const protocols = [a.sftp && "SFTP", a.ftps && "FTPS", a.ftp_plain && "FTP"].filter(Boolean).join(" · ");
  return {
    id: a.id,
    username: a.name,
    home_directory: `/${a.home}`.replace(/\/+$/, "") || "/",
    quota_bytes: unlimited ? -1 : (a.quota_mb as number) * MIB,
    quota_formatted: unlimited ? "∞" : formatMb(a.quota_mb as number),
    // The backend does not report how much of the quota an account has used.
    used_bytes: 0,
    used_formatted: "—",
    status: a.enabled && a.status === "ready" ? "active" : a.status === "ready" ? "suspended" : a.status,
    access: protocols + (a.read_only ? " · read-only" : ""),
  };
}

export async function listFtp(): Promise<{
  accounts: FtpAccount[];
  info: FtpConnectionInfo;
  prefix: string;
  limit: number | null;
}> {
  const res = await apiFetch<FtpAccountList>(BASE);
  return {
    accounts: res.items.map(toFtpAccount),
    prefix: res.prefix,
    limit: res.limit ?? null,
    info: {
      host: res.host || window.location.hostname,
      port: res.ftp_port,
      protocol: "FTP",
      encryption: "FTPS (explicit TLS) / SFTP",
      sftp_port: res.sftp_port,
    },
  };
}

export async function createFtp(input: {
  name: string;
  password: string;
  home: string;
  quotaMb: number | null;
}): Promise<{ account: FtpAccount; password: string | null }> {
  const res = await apiFetch<AccountWithPassword>(BASE, {
    method: "POST",
    body: JSON.stringify({
      name: input.name,
      password: input.password,
      home: input.home,
      ...(input.quotaMb ? { quota_mb: input.quotaMb } : {}),
    }),
  });
  return { account: toFtpAccount(res.account), password: res.password ?? null };
}

export async function changeFtpPassword(id: string, password: string): Promise<void> {
  await apiFetch(`${BASE}/${encodeURIComponent(id)}/password`, {
    method: "PUT",
    body: JSON.stringify({ password }),
  });
}

/** `null` removes the limit (the backend takes `0` for that). */
export async function changeFtpQuota(id: string, quotaMb: number | null): Promise<void> {
  await apiFetch(`${BASE}/${encodeURIComponent(id)}`, {
    method: "PUT",
    body: JSON.stringify({ quota_mb: quotaMb ?? 0 }),
  });
}

export async function deleteFtp(id: string): Promise<void> {
  await apiFetch(`${BASE}/${encodeURIComponent(id)}`, { method: "DELETE" });
}
