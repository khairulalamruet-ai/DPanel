import { apiFetch } from "@builtpanel/api";
import type { BackupItem, BackupStatus, RestoreBackupPayload } from "./types";

interface PartView {
  /** `files`, `database`, `dns`, ... */
  kind: string;
  name: string;
  database?: string | null;
  stored_bytes: number;
}

/** A backup as the backend returns it (`BackupView`). */
interface BackupView {
  id: string;
  /** `queued`, `running`, `ready`, `failed`, ... */
  status: string;
  /** `manual`, `schedule`, ... */
  trigger: string;
  size_bytes?: number | null;
  error?: string | null;
  created_at: string;
  finished_at?: string | null;
  parts: PartView[];
}

const B = "/api/v1/client/backups";
const enc = encodeURIComponent;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let v = bytes / 1024;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v.toFixed(1)} ${units[i]}`;
}

const statusOf = (s: string): BackupStatus => (s === "ready" ? "completed" : s === "failed" || s === "cancelled" ? "failed" : "in_progress");

export function toBackup(b: BackupView): BackupItem {
  const dbs = b.parts.filter((p) => p.kind === "database").map((p) => p.database || p.name);
  const hasFiles = b.parts.some((p) => p.kind === "files");
  const stamp = b.created_at.slice(0, 16).replace(/[-:T]/g, "").replace(/^(\d{8})(\d{4})$/, "$1-$2");
  return {
    id: b.id,
    name: `backup-${stamp}-${b.trigger}`,
    type: hasFiles && dbs.length ? "full" : dbs.length ? "databases" : "files",
    size_bytes: b.size_bytes ?? b.parts.reduce((sum, p) => sum + p.stored_bytes, 0),
    size_formatted: formatSize(b.size_bytes ?? b.parts.reduce((sum, p) => sum + p.stored_bytes, 0)),
    status: statusOf(b.status),
    created_at: b.created_at,
    completed_at: b.finished_at ?? undefined,
    destination: b.trigger,
    sha256: "",
    manifest: { domains: [], databases: dbs, has_dns: b.parts.some((p) => p.kind === "dns") },
    ...(b.error ? { error: b.error } : {}),
  };
}

export async function listBackups(): Promise<BackupItem[]> {
  const res = await apiFetch<{ items: BackupView[] }>(B);
  return res.items.map(toBackup);
}

/** Makes a full backup now; it is queued, and the list shows it until it is ready. */
export async function createBackup(): Promise<BackupItem> {
  return toBackup(await apiFetch<BackupView>(B, { method: "POST" }));
}

export async function deleteBackup(id: string): Promise<void> {
  await apiFetch(`${B}/${enc(id)}`, { method: "DELETE" });
}

export async function restoreBackup(backup: BackupItem, p: RestoreBackupPayload): Promise<void> {
  const all = backup.manifest.databases;
  const body =
    p.restore_mode === "files_only"
      ? { files: true, databases: [], dns: false }
      : p.restore_mode === "databases_only"
        ? { files: false, databases: all, dns: false }
        : p.restore_mode === "selective"
          ? { files: (p.selected_domains?.length ?? 0) > 0, databases: p.selected_databases ?? [], dns: false }
          : { files: true, databases: all, dns: !!backup.manifest.has_dns };
  await apiFetch(`${B}/${enc(backup.id)}/restore`, {
    method: "POST",
    body: JSON.stringify({ ...body, snapshot: p.take_snapshot_before_restore }),
  });
}

interface BackupExportView {
  id: string;
  backup_id: string;
  /** `queued`, `running`, `ready` or `failed`. */
  status: string;
  error?: string | null;
  /** The file's name at the destination, once ready. */
  object?: string | null;
  size_bytes?: number | null;
}

/**
 * Packs a backup into one file (an account export another panel can import), sealed with the
 * passphrase. The file is kept at the backup destination; this waits until it is there.
 */
export async function exportBackup(
  id: string,
  passphrase: string,
  opts: { intervalMs?: number; tries?: number } = {},
): Promise<BackupExportView> {
  const { intervalMs = 2000, tries = 150 } = opts;
  let exp = await apiFetch<BackupExportView>(`${B}/${enc(id)}/export`, { method: "POST", body: JSON.stringify({ passphrase }) });
  for (let i = 0; i < tries && exp.status !== "ready"; i++) {
    if (exp.status === "failed") throw new Error(exp.error || "The export failed");
    await sleep(intervalMs);
    const list = await apiFetch<{ items: BackupExportView[] }>("/api/v1/client/backup-exports");
    exp = list.items.find((e) => e.id === exp.id) ?? exp;
  }
  if (exp.status !== "ready") throw new Error("The export is taking too long; look at the list of exports in a moment");
  return exp;
}

// ---- the account's own schedule

export interface ClientBackupSchedule {
  cron: string;
  enabled: boolean;
  keep_daily: number;
  keep_weekly: number;
}

/** `null` when the account has no schedule. */
export async function getSchedule(): Promise<ClientBackupSchedule | null> {
  try {
    return await apiFetch<ClientBackupSchedule>("/api/v1/client/backup-schedule");
  } catch (err: any) {
    if (err?.status === 404) return null;
    throw err;
  }
}

export async function saveSchedule(s: ClientBackupSchedule): Promise<ClientBackupSchedule> {
  return apiFetch<ClientBackupSchedule>("/api/v1/client/backup-schedule", { method: "PUT", body: JSON.stringify(s) });
}

export async function removeSchedule(): Promise<void> {
  await apiFetch("/api/v1/client/backup-schedule", { method: "DELETE" });
}
