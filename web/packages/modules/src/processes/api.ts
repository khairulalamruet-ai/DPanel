import { apiFetch, untilAnswered } from "@builtpanel/api";
import type { ProcessItem, ProcessOverviewStats, ProcessSignal } from "./types";

interface ProcessView {
  container: string;
  pid: number;
  uid: number;
  /** One letter of `ps`: `R`, `S`, `D`, `Z`, `T`. */
  state: string;
  command: string;
}

export function toProcess(p: ProcessView): ProcessItem {
  return {
    pid: p.pid,
    // The node reports what a process is and in what state; load figures are not part of it.
    ppid: 0,
    user: String(p.uid),
    cpu_percent: 0,
    mem_percent: 0,
    mem_rss_mb: 0,
    vsz_mb: 0,
    stat: p.state,
    start_time: "",
    cpu_time: "",
    command: p.command,
    // The first process of a container (pid 1) is the container itself.
    is_killable: p.pid >= 2,
    container: p.container,
  };
}

export function statsOf(list: ProcessItem[]): ProcessOverviewStats {
  const count = (s: string) => list.filter((p) => p.stat.startsWith(s)).length;
  return {
    total_processes: list.length,
    running_processes: count("R"),
    sleeping_processes: count("S") + count("D"),
    zombie_processes: count("Z"),
    total_cpu_percent: 0,
    total_mem_rss_mb: 0,
  };
}

export async function listProcesses(): Promise<{ items: ProcessItem[]; stats: ProcessOverviewStats }> {
  const res = await untilAnswered(() => apiFetch<{ items: ProcessView[] }>("/api/v1/client/processes"));
  const items = res.items.map(toProcess);
  return { items, stats: statsOf(items) };
}

/** Ends a process: the signal, or with `escalate` SIGTERM first and SIGKILL if it stays. */
export async function signalProcess(container: string, pid: number, signal: ProcessSignal): Promise<void> {
  await apiFetch(`/api/v1/client/processes/${encodeURIComponent(container)}/${encodeURIComponent(pid)}/signal`, {
    method: "POST",
    body: JSON.stringify({ signal: signal === "SIGKILL" ? "SIGKILL" : "SIGTERM", escalate: signal === "SIGTERM" }),
  });
}
