import { apiFetch, untilAnswered } from "@builtpanel/api";
import type { LogEntry, LogType } from "./types";

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
