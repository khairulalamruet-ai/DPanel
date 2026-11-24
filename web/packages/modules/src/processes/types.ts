export type ProcessSignal = "SIGTERM" | "SIGKILL" | "SIGHUP";

export interface ProcessItem {
  pid: number;
  ppid: number;
  user: string;
  cpu_percent: number;
  mem_percent: number;
  mem_rss_mb: number;
  vsz_mb: number;
  stat: string; // "R" (running), "S" (sleeping), "D" (disk sleep), "Z" (zombie), "T" (stopped)
  start_time: string;
  cpu_time: string;
  command: string;
  is_killable: boolean;
  /** The container it runs in (a hosting account has a web and a PHP container). */
  container?: string;
  tenant?: string; // only present in admin scope
  node?: string; // only present in admin scope
}

export interface ProcessOverviewStats {
  total_processes: number;
  running_processes: number;
  sleeping_processes: number;
  zombie_processes: number;
  total_cpu_percent: number;
  total_mem_rss_mb: number;
}

export interface KillProcessPayload {
  pid: number;
  signal: ProcessSignal;
  sudo_password?: string;
}
