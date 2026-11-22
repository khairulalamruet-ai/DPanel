import { apiFetch } from "@builtpanel/api";
import type { CronJob, CreateCronJobInput } from "./types";

/** A job as the backend returns it (`JobView`). */
export interface JobView {
  id: string;
  name: string;
  schedule: string;
  command: string;
  enabled: boolean;
  timeout_seconds: number;
  last_run_at?: string | null;
  last_status?: string | null;
  last_exit_code?: number | null;
  last_output?: string | null;
}

interface JobList {
  items: JobView[];
  limit?: number | null;
}

const BASE = "/api/v1/client/cron-jobs";

function parseField(field: string, min: number, max: number): Set<number> | null {
  const out = new Set<number>();
  for (const part of field.split(",")) {
    const [range, stepText] = part.split("/");
    const step = stepText === undefined ? 1 : Number(stepText);
    if (!Number.isInteger(step) || step < 1) return null;
    let from = min;
    let to = max;
    if (range !== "*") {
      const [a, b] = range.split("-");
      from = Number(a);
      to = b === undefined ? (stepText === undefined ? from : max) : Number(b);
    }
    if (!Number.isInteger(from) || !Number.isInteger(to) || from < min || to > max || from > to) return null;
    for (let v = from; v <= to; v += step) out.add(v);
  }
  return out;
}

/** The next time a five-field schedule fires (the panel runs schedules in UTC), or `null`. */
export function nextRun(schedule: string, from: Date = new Date()): Date | null {
  const parts = schedule.trim().split(/\s+/);
  if (parts.length !== 5) return null;
  const min = parseField(parts[0], 0, 59);
  const hour = parseField(parts[1], 0, 23);
  const dom = parseField(parts[2], 1, 31);
  const month = parseField(parts[3], 1, 12);
  const dowRaw = parseField(parts[4].replace(/\b7\b/g, "0"), 0, 6);
  if (!min || !hour || !dom || !month || !dowRaw) return null;
  const domAny = parts[2] === "*";
  const dowAny = parts[4] === "*";
  const t = new Date(Date.UTC(from.getUTCFullYear(), from.getUTCMonth(), from.getUTCDate(), from.getUTCHours(), from.getUTCMinutes() + 1));
  for (let i = 0; i < 60 * 24 * 366; i++) {
    const dayOk =
      domAny && dowAny
        ? true
        : domAny
          ? dowRaw.has(t.getUTCDay())
          : dowAny
            ? dom.has(t.getUTCDate())
            : dom.has(t.getUTCDate()) || dowRaw.has(t.getUTCDay());
    if (month.has(t.getUTCMonth() + 1) && dayOk && hour.has(t.getUTCHours()) && min.has(t.getUTCMinutes())) {
      return new Date(t);
    }
    t.setUTCMinutes(t.getUTCMinutes() + 1);
  }
  return null;
}

function status(raw: string | null | undefined): CronJob["last_status"] {
  switch (raw) {
    case "ok":
      return "success";
    case "failed":
    case "timeout":
      return "failed";
    default:
      return "idle";
  }
}

export function toCronJob(j: JobView): CronJob {
  const next = j.enabled ? nextRun(j.schedule) : null;
  return {
    id: j.id,
    name: j.name,
    command: j.command,
    schedule: j.schedule,
    enabled: j.enabled,
    timeout_seconds: j.timeout_seconds,
    last_run_at: j.last_run_at ?? null,
    next_run_at: next ? next.toISOString() : null,
    last_status: status(j.last_status),
    last_exit_code: j.last_exit_code ?? null,
    last_output: j.last_output ?? null,
  };
}

export async function listCronJobs(): Promise<{ jobs: CronJob[]; limit: number | null }> {
  const res = await apiFetch<JobList>(BASE);
  return { jobs: res.items.map(toCronJob), limit: res.limit ?? null };
}

export async function createCronJob(input: CreateCronJobInput): Promise<CronJob> {
  const res = await apiFetch<JobView>(BASE, {
    method: "POST",
    body: JSON.stringify({
      name: input.name,
      command: input.command,
      schedule: input.schedule,
      timeout_seconds: input.timeout_seconds ?? 300,
      enabled: true,
    }),
  });
  return toCronJob(res);
}

/** The backend replaces every field on a change, so the whole job goes back. */
export async function setCronEnabled(job: CronJob, enabled: boolean): Promise<CronJob> {
  const res = await apiFetch<JobView>(`${BASE}/${encodeURIComponent(job.id)}`, {
    method: "PUT",
    body: JSON.stringify({
      name: job.name,
      command: job.command,
      schedule: job.schedule,
      timeout_seconds: job.timeout_seconds,
      enabled,
    }),
  });
  return toCronJob(res);
}

/** Asks for a run now; the result shows as the job's last run within about a minute. */
export async function runCronJob(id: string): Promise<void> {
  await apiFetch(`${BASE}/${encodeURIComponent(id)}/run`, { method: "POST" });
}

export async function deleteCronJob(id: string): Promise<void> {
  await apiFetch(`${BASE}/${encodeURIComponent(id)}`, { method: "DELETE" });
}
