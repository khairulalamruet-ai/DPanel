import React, { useState, useEffect, useCallback, useRef } from "react";
import { Card, Button, Badge, Input, useToast } from "@builtpanel/ui";
import { useTranslation } from "@builtpanel/i18n";
import { Terminal, Download, Search, Play, Pause, RefreshCw, AlertTriangle } from "lucide-react";
import { SystemLogEntry, SystemLogLevel } from "./types";
import { VirtualizedLogList } from "./VirtualizedLogList";
import { CONTROL_PLANE, listSystemSources, readSystemLog } from "./api";
import { listNodeChoices, type NodeChoice } from "../nodes/api";

/** The control plane's own log, and the core services (database, phpMyAdmin, SFTP) of each node. */
export function AdminSystemLogsPage() {
  const { t } = useTranslation();
  const { toast } = useToast();

  const [nodes, setNodes] = useState<NodeChoice[]>([]);
  const [node, setNode] = useState("");
  const [sources, setSources] = useState<string[]>([CONTROL_PLANE]);
  const [source, setSource] = useState(CONTROL_PLANE);
  const [level, setLevel] = useState<SystemLogLevel>("all");
  const [searchQuery, setSearchQuery] = useState("");
  const [isLiveTail, setIsLiveTail] = useState(false);
  const [entries, setEntries] = useState<SystemLogEntry[]>([]);
  const [truncated, setTruncated] = useState(false);
  const [isLoading, setIsLoading] = useState(false);

  const fail = useCallback(
    (err: any) => toast({ title: t("common.error"), description: err?.message || t("common.unknownError"), variant: "destructive" }),
    [t, toast],
  );

  useEffect(() => {
    listNodeChoices()
      .then((list) => {
        setNodes(list);
        setNode((current) => current || list[0]?.id || "");
      })
      .catch(fail);
  }, [fail]);

  useEffect(() => {
    listSystemSources(node)
      .then((list) => {
        setSources(list);
        setSource((current) => (list.includes(current) ? current : CONTROL_PLANE));
      })
      .catch(() => setSources([CONTROL_PLANE]));
  }, [node]);

  const fetchLogs = useCallback(async () => {
    try {
      setIsLoading(true);
      // The control plane's log is read with a level; a node service's log has none.
      const res = await readSystemLog({ source, nodeId: node, level, lines: 1000 });
      setEntries(res.entries);
      setTruncated(res.truncated);
    } catch (err) {
      fail(err);
    } finally {
      setIsLoading(false);
    }
  }, [source, node, level, fail]);

  useEffect(() => {
    void fetchLogs();
  }, [fetchLogs]);

  // Live tail: ask for what was written since the last line, every few seconds
  const lastAt = useRef(0);
  useEffect(() => {
    lastAt.current = entries.length ? Date.parse(entries[entries.length - 1].timestamp) : Date.now();
  }, [entries]);

  useEffect(() => {
    if (!isLiveTail) return;
    const timer = setInterval(async () => {
      try {
        const res = await readSystemLog({ source, nodeId: node, level, sinceMs: lastAt.current, lines: 500 });
        if (res.entries.length > 0) {
          setEntries((prev) => [...prev, ...res.entries.map((e, i) => ({ ...e, lineNumber: prev.length + i + 1 }))]);
        }
      } catch {
        // The next round tries again.
      }
    }, 3000);
    return () => clearInterval(timer);
  }, [isLiveTail, source, node, level]);

  const handleDownloadLog = () => {
    if (entries.length === 0) return;
    const blob = new Blob([entries.map((e) => e.raw).join("\n")], { type: "text/plain;charset=utf-8" });
    const url = URL.createObjectURL(blob);
    const link = document.createElement("a");
    link.href = url;
    link.download = `system-${source}-${new Date().toISOString().slice(0, 10)}.log`;
    document.body.appendChild(link);
    link.click();
    document.body.removeChild(link);
    URL.revokeObjectURL(url);
  };

  const errorCount = entries.filter((e) => e.level === "error").length;
  const bytes = entries.reduce((sum, e) => sum + e.raw.length + 1, 0);

  return (
    <div className="p-6 space-y-4" data-testid="admin-system-logs-page">
      <div className="flex flex-col sm:flex-row sm:items-center sm:justify-between gap-4">
        <div>
          <h1 className="text-2xl font-bold tracking-tight" data-testid="admin-system-logs-title">
            {t("adminSystemLogs.title")}
          </h1>
          <p className="text-sm text-muted-foreground">{t("adminSystemLogs.description")}</p>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <Button
            size="sm"
            variant={isLiveTail ? "default" : "outline"}
            onClick={() => setIsLiveTail(!isLiveTail)}
            className="gap-1.5 text-xs"
            data-testid="btn-toggle-admin-live-mode"
          >
            {isLiveTail ? <Pause className="w-3 h-3" /> : <Play className="w-3 h-3 text-green-500" />}
            {isLiveTail ? t("adminSystemLogs.liveTailActive") : t("adminSystemLogs.startLiveTail")}
          </Button>
          <Button size="sm" variant="outline" onClick={handleDownloadLog} disabled={entries.length === 0} className="gap-1.5 text-xs" data-testid="btn-download-admin-log">
            <Download className="w-3.5 h-3.5" />
            {t("common.download")}
          </Button>
          <Button size="sm" variant="ghost" onClick={fetchLogs} disabled={isLoading} className="h-8 w-8 p-0" data-testid="btn-refresh-admin-logs">
            <RefreshCw className={`w-3.5 h-3.5 ${isLoading ? "animate-spin" : ""}`} />
          </Button>
        </div>
      </div>

      <Card className="p-3">
        <div className="flex flex-col lg:flex-row lg:items-center justify-between gap-3">
          <div className="flex flex-wrap items-center gap-3">
            <div className="flex items-center rounded-lg bg-muted p-1 text-xs flex-wrap">
              {sources.map((s) => (
                <button
                  key={s}
                  type="button"
                  onClick={() => setSource(s)}
                  className={`px-3 py-1 rounded-md font-medium transition-colors ${
                    source === s ? "bg-background text-foreground shadow-sm" : "text-muted-foreground hover:text-foreground"
                  }`}
                  data-testid={`tab-service-${s}`}
                >
                  {s}
                </button>
              ))}
            </div>
            <select
              value={node}
              onChange={(e) => setNode(e.target.value)}
              className="h-8 text-xs rounded-md border border-input bg-background px-2.5"
              data-testid="select-admin-log-node"
            >
              {nodes.map((n) => (
                <option key={n.id} value={n.id}>
                  {n.name}
                </option>
              ))}
            </select>
            <select
              value={level}
              onChange={(e) => setLevel(e.target.value as SystemLogLevel)}
              disabled={source !== CONTROL_PLANE}
              className="h-8 text-xs rounded-md border border-input bg-background px-2.5"
              data-testid="select-admin-log-level"
            >
              <option value="all">{t("adminSystemLogs.levelAll")}</option>
              <option value="debug">debug</option>
              <option value="info">info</option>
              <option value="warn">warn</option>
              <option value="error">error</option>
            </select>
          </div>
          <div className="relative w-full lg:w-72">
            <Search className="w-3.5 h-3.5 absolute left-2.5 top-1/2 -translate-y-1/2 text-muted-foreground" />
            <Input
              placeholder={t("adminSystemLogs.searchPlaceholder")}
              value={searchQuery}
              onChange={(e) => setSearchQuery(e.target.value)}
              className="pl-8 text-xs h-8"
              data-testid="input-admin-log-search"
            />
          </div>
        </div>
      </Card>

      <div className="flex flex-wrap items-center justify-between text-xs text-muted-foreground px-1">
        <div className="flex items-center gap-4">
          <div>
            <span className="font-semibold text-foreground" data-testid="metric-admin-lines">
              {entries.length.toLocaleString()}
            </span>{" "}
            {t("adminSystemLogs.linesLoaded")}
          </div>
          <div>
            <span className="font-semibold text-foreground" data-testid="metric-admin-size">
              {bytes > 1048576 ? `${(bytes / 1048576).toFixed(1)} MB` : `${(bytes / 1024).toFixed(1)} KB`}
            </span>
          </div>
          {errorCount > 0 && (
            <div className="text-destructive font-medium flex items-center gap-1">
              <AlertTriangle className="w-3 h-3" />
              <span data-testid="metric-admin-errors">{errorCount}</span> {t("adminSystemLogs.errorsDetected")}
            </div>
          )}
        </div>
        <div className="flex items-center gap-2">
          <Badge variant="outline" className="text-[11px]" data-testid="badge-admin-live-status">
            {isLiveTail ? t("adminSystemLogs.streamLive") : t("adminSystemLogs.streamPaused")}
          </Badge>
          {truncated && (
            <Badge variant="secondary" className="text-[11px]">
              {t("logs.onlyNewest", "Only the newest lines are shown")}
            </Badge>
          )}
          <Badge variant="secondary" className="font-mono text-[11px] gap-1" data-testid="badge-service-active">
            <Terminal className="w-3 h-3" />
            {source}
          </Badge>
        </div>
      </div>

      <div className="h-[580px] w-full">
        <VirtualizedLogList entries={entries} searchQuery={searchQuery} autoScrollBottom={isLiveTail} />
      </div>
    </div>
  );
}
