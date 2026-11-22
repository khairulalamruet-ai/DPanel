import { apiFetch } from "@builtpanel/api";
import type {
  AdminBackupOverview,
  BackupDestination,
  BackupDestinationType,
  BackupJob,
  BackupSchedule,
} from "./types";

/** A destination as the backend returns it (`DestinationView`). */
interface DestinationView {
  id: string;
  name: string;
  /** `local`, `s3` or `sftp`. */
  kind: string;
  settings: Record<string, unknown>;
  is_default: boolean;
  enabled: boolean;
  has_credentials: boolean;
  last_tested_at?: string | null;
  last_test_ok?: boolean | null;
  last_test_error?: string | null;
}

interface SystemBackupView {
  id: string;
  status: string;
  trigger: string;
  size_bytes?: number | null;
  error?: string | null;
  created_at: string;
  finished_at?: string | null;
}

export interface SystemSchedule {
  cron: string;
  enabled: boolean;
  keep_daily: number;
  keep_weekly: number;
}

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

function summary(d: DestinationView): string {
  const s = d.settings ?? {};
  if (d.kind === "local") return String(s.path ?? "");
  if (d.kind === "s3") return `${s.bucket ?? ""}${s.endpoint ? ` @ ${s.endpoint}` : ""}`;
  return `${s.user ?? ""}@${s.host ?? ""}:${s.port ?? 22}${s.path ?? ""}`;
}

export function toDestination(d: DestinationView): BackupDestination {
  return {
    id: d.id,
    name: d.name,
    type: (d.kind === "s3" || d.kind === "sftp" ? d.kind : "local") as BackupDestinationType,
    is_default: d.is_default,
    status: d.last_test_ok === false ? "error" : "connected",
    config_summary: summary(d),
    last_tested_at: d.last_tested_at ?? undefined,
    error_message: d.last_test_error ?? undefined,
  };
}

const jobStatus = (s: string): BackupJob["status"] => (s === "ready" ? "completed" : s === "failed" ? "failed" : "in_progress");

export function toJob(b: SystemBackupView): BackupJob {
  const size = b.size_bytes ?? 0;
  return {
    id: b.id,
    job_type: "system",
    target_name: "system:control-plane",
    destination_name: "",
    status: jobStatus(b.status),
    size_bytes: size,
    size_formatted: formatSize(size),
    started_at: b.created_at,
    completed_at: b.finished_at ?? undefined,
    error_message: b.error ?? undefined,
    // A failed backup is reported to the administrators through the notifications.
    notification_sent: b.status === "failed",
  };
}

export function toSchedule(s: SystemSchedule): BackupSchedule {
  return {
    id: "system",
    name: "Control plane backup",
    cron_expression: s.cron,
    cron_readable: s.cron,
    destination_id: "",
    destination_name: "",
    retention_daily: s.keep_daily,
    retention_weekly: s.keep_weekly,
    retention_monthly: 0,
    scope: "system",
    enabled: s.enabled,
    next_run_at: "",
  };
}

export async function loadAdminBackups(): Promise<{
  overview: AdminBackupOverview;
  jobs: BackupJob[];
  destinations: BackupDestination[];
  schedules: BackupSchedule[];
}> {
  const [dests, backups, schedule] = await Promise.all([
    apiFetch<{ items: DestinationView[] }>("/api/v1/admin/backup-destinations"),
    apiFetch<{ items: SystemBackupView[] }>("/api/v1/admin/system-backups"),
    apiFetch<SystemSchedule>("/api/v1/admin/system-backup-schedule").catch((err: any) => {
      if (err?.status === 404) return null;
      throw err;
    }),
  ]);
  const jobs = backups.items.map(toJob);
  const last = backups.items.find((b) => b.status === "ready");
  return {
    jobs,
    destinations: dests.items.map(toDestination),
    schedules: schedule ? [toSchedule(schedule)] : [],
    overview: {
      total_jobs: jobs.length,
      active_jobs: jobs.filter((j) => j.status === "in_progress").length,
      failed_jobs: jobs.filter((j) => j.status === "failed").length,
      destinations_count: dests.items.length,
      schedules_count: schedule ? 1 : 0,
      last_system_backup_at: last?.finished_at ?? last?.created_at,
      last_system_backup_size: formatSize(last?.size_bytes ?? 0),
      system_backup_healthy: backups.items[0]?.status !== "failed",
    },
  };
}

export async function runSystemBackup(): Promise<void> {
  await apiFetch("/api/v1/admin/system-backups", { method: "POST" });
}

export async function saveSystemSchedule(s: SystemSchedule): Promise<void> {
  await apiFetch("/api/v1/admin/system-backup-schedule", { method: "PUT", body: JSON.stringify(s) });
}

export async function removeSystemSchedule(): Promise<void> {
  await apiFetch("/api/v1/admin/system-backup-schedule", { method: "DELETE" });
}

export async function setDefaultDestination(id: string): Promise<void> {
  await apiFetch(`/api/v1/admin/backup-destinations/${encodeURIComponent(id)}/default`, { method: "POST" });
}

export async function deleteDestination(id: string): Promise<void> {
  await apiFetch(`/api/v1/admin/backup-destinations/${encodeURIComponent(id)}`, { method: "DELETE" });
}

export interface NewDestination {
  type: BackupDestinationType;
  name: string;
  local?: { path: string };
  s3?: { endpoint: string; bucket: string; region: string; accessKey: string; secretKey: string };
  sftp?: { host: string; port: number; user: string; path: string; password: string };
}

export async function createDestination(d: NewDestination): Promise<void> {
  const body =
    d.type === "local"
      ? { kind: "local", name: d.name, settings: { path: d.local?.path } }
      : d.type === "s3"
        ? {
            kind: "s3",
            name: d.name,
            settings: { endpoint: d.s3?.endpoint, bucket: d.s3?.bucket, region: d.s3?.region },
            credentials: { access_key: d.s3?.accessKey, secret_key: d.s3?.secretKey },
          }
        : {
            kind: "sftp",
            name: d.name,
            settings: { host: d.sftp?.host, port: d.sftp?.port, user: d.sftp?.user, path: d.sftp?.path },
            credentials: { password: d.sftp?.password },
          };
  await apiFetch("/api/v1/admin/backup-destinations", { method: "POST", body: JSON.stringify(body) });
}
