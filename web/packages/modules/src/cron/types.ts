export interface CronJob {
  id: string;
  name: string;
  command: string;
  schedule: string; // 5-field standard cron, e.g. "0 * * * *"
  schedule_human?: string;
  enabled: boolean;
  description?: string;
  timeout_seconds: number;
  created_at?: string;
  last_run_at: string | null;
  next_run_at: string | null;
  last_status: "success" | "failed" | "running" | "idle";
  last_exit_code: number | null;
  last_output: string | null;
  last_duration_ms?: number | null;
}

export interface CreateCronJobInput {
  name: string;
  command: string;
  schedule: string;
  description?: string;
  timeout_seconds?: number;
}

export interface RunCronResult {
  job_id: string;
  status: "success" | "failed";
  exit_code: number;
  output: string;
  duration_ms: number;
  executed_at: string;
}
