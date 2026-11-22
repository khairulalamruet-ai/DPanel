import { apiFetch, untilAnswered } from "@builtpanel/api";
import type { LogEntry, LogType, SystemLogEntry } from "./types";

interface LogLines {
  lines: { at_ms: number; text: string }[];
  truncated: boolean;
}

const SOURCE: Record<LogType, string> = { access: "web_access", error: "web_error", php: "php_error" };

// 203.0.113.5 - - [10/Nov/2026:10:00:01 +0000] "GET /index.php HTTP/1.1" 200 1234 ...
const ACCESS = /^(\S+) \S+ \S+ \[[^\]]+\] "(\S+) (\S+)[^"]*" (\d{3})/;

/** Reads what a log line says when it has the usual shape; otherwise the line is shown as it is. */
export function toEntry(type: LogType, l: { at_ms: number; text: string }, index: number): LogEntry {
  const m = type === "access" ? ACCESS.exec(l.text) : null;
  const status = m ? Number(m[4]) : undefined;
  const lower = l.text.toLowerCase();
  return {
    id: `${l.at_ms}_${index}`,
    lineNumber: index + 1,
    timestamp: new Date(l.at_ms).toISOString(),
    type,
    domain: "",
    status_code: status,
    method: m?.[2],
    path: m?.[3],
    ip: m?.[1],
    level:
      (status ?? 0) >= 500 || /\b(error|fatal|critical)\b/.test(lower)
        ? "error"
        : (status ?? 0) >= 400 || /\b(warn|warning|notice)\b/.test(lower)
          ? "warn"
          : "info",
    message: l.text,
    raw: l.text,
  };
}

/** The newest lines of one of the account's logs. */
export async function readLog(
  type: LogType,
  opts: { lines?: number; sinceMs?: number } = {},
): Promise<{ entries: LogEntry[]; truncated: boolean }> {
  const q = new URLSearchParams({ source: SOURCE[type], lines: String(opts.lines ?? 500) });
  if (opts.sinceMs) q.set("since_ms", String(opts.sinceMs));
  const res = await untilAnswered(() => apiFetch<LogLines>(`/api/v1/client/logs?${q}`));
  return { entries: res.lines.map((l, i) => toEntry(type, l, i)), truncated: res.truncated };
}

// ---- administrators: the control plane's own log and the core services of a node


export const CONTROL_PLANE = "control-plane";

/** What can be read: the control plane, and the core services a node reports. */
export async function listSystemSources(nodeId: string): Promise<string[]> {
  const services = nodeId
    ? await apiFetch<{ items: { name: string }[] }>(`/api/v1/admin/nodes/${encodeURIComponent(nodeId)}/services`).catch(() => ({ items: [] }))
    : { items: [] as { name: string }[] };
  return [CONTROL_PLANE, ...services.items.map((s) => s.name)];
}

interface SysLine {
  at_ms: number;
  level?: string;
  target?: string;
  text: string;
}

const levelOf = (l?: string, text = ""): SystemLogEntry["level"] => {
  const v = (l ?? "").toLowerCase();
  if (v === "error" || v === "warn" || v === "debug" || v === "info") return v;
  const t = text.toLowerCase();
  return /\b(error|fatal|panic)\b/.test(t) ? "error" : /\bwarn(ing)?\b/.test(t) ? "warn" : "info";
};

export async function readSystemLog(opts: {
  source: string;
  nodeId: string;
  level?: string;
  search?: string;
  sinceMs?: number;
  lines?: number;
}): Promise<{ entries: SystemLogEntry[]; truncated: boolean }> {
  const q = new URLSearchParams({ lines: String(opts.lines ?? 500) });
  if (opts.sinceMs) q.set("since_ms", String(opts.sinceMs));
  if (opts.search) q.set("search", opts.search);
  let path: string;
  if (opts.source === CONTROL_PLANE) {
    if (opts.level && opts.level !== "all") q.set("level", opts.level);
    path = `/api/v1/admin/system-logs?${q}`;
  } else {
    path = `/api/v1/admin/nodes/${encodeURIComponent(opts.nodeId)}/services/${encodeURIComponent(opts.source)}/log?${q}`;
  }
  const res = await untilAnswered(() => apiFetch<{ lines?: SysLine[]; items?: SysLine[]; truncated?: boolean }>(path));
  const lines = res.lines ?? res.items ?? [];
  return {
    truncated: !!res.truncated,
    entries: lines.map((l, i) => ({
      id: `${l.at_ms}_${i}`,
      lineNumber: i + 1,
      timestamp: new Date(l.at_ms).toISOString(),
      service: opts.source as SystemLogEntry["service"],
      node: opts.nodeId,
      level: levelOf(l.level, l.text),
      component: l.target,
      message: l.text,
      raw: l.text,
    })),
  };
}
