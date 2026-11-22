import React, { useState, useEffect, useCallback, useRef } from "react";
import {
  Card,
  CardHeader,
  CardTitle,
  CardContent,
  Button,
  Badge,
  Input,
  useToast,
} from "@builtpanel/ui";
import { useTranslation } from "@builtpanel/i18n";
import {
  FileText,
  Terminal,
  Download,
  Search,
  Play,
  Pause,
  RefreshCw,
  AlertTriangle,
  ArrowDown,
  Layers,
} from "lucide-react";
import { LogEntry, LogType } from "./types";
import { readLog } from "./api";
import { VirtualizedLogList } from "./VirtualizedLogList";

export function LogViewerPage() {
  const { t } = useTranslation();
  const { toast } = useToast();

  const [logType, setLogType] = useState<LogType>("access");
  const [searchQuery, setSearchQuery] = useState("");
  const [isLiveTail, setIsLiveTail] = useState(false);
  const [entries, setEntries] = useState<LogEntry[]>([]);
  const [isLoading, setIsLoading] = useState(false);
  const [autoScroll, setAutoScroll] = useState(false);
  const [truncated, setTruncated] = useState(false);

  // The newest lines of the chosen log
  const fetchLogs = useCallback(async () => {
    try {
      setIsLoading(true);
      const res = await readLog(logType, { lines: 1000 });
      setEntries(res.entries);
      setTruncated(res.truncated);
    } catch (err: any) {
      toast({
        title: t("common.error"),
        description: err.message || t("common.unknownError"),
        variant: "destructive",
      });
    } finally {
      setIsLoading(false);
    }
  }, [logType, t, toast]);

  useEffect(() => {
    fetchLogs();
  }, [fetchLogs]);

  // Live tail: ask for what was written since the last line, every few seconds
  const lastAt = useRef(0);
  useEffect(() => {
    lastAt.current = entries.length ? Date.parse(entries[entries.length - 1].timestamp) : Date.now();
  }, [entries]);

  useEffect(() => {
    if (!isLiveTail) return;
    const interval = setInterval(async () => {
      try {
        const res = await readLog(logType, { lines: 500, sinceMs: lastAt.current });
        if (res.entries.length > 0) {
          setEntries((prev) => [...prev, ...res.entries.map((e, i) => ({ ...e, lineNumber: prev.length + i + 1 }))]);
        }
      } catch {
        // The next round tries again.
      }
    }, 3000);
    return () => clearInterval(interval);
  }, [isLiveTail, logType]);

  const handleDownloadLog = () => {
    if (entries.length === 0) return;
    const content = entries.map((e) => e.raw).join("\n");
    const blob = new Blob([content], { type: "text/plain;charset=utf-8" });
    const url = URL.createObjectURL(blob);
    const link = document.createElement("a");
    link.href = url;
    link.download = `${logType}-${new Date().toISOString().slice(0, 10)}.log`;
    document.body.appendChild(link);
    link.click();
    document.body.removeChild(link);
    URL.revokeObjectURL(url);

    toast({
      title: t("common.success"),
      description: t("logs.downloadStarted", { name: `${logType}.log` }),
    });
  };

  const loadedBytes = entries.reduce((sum, e) => sum + e.raw.length + 1, 0);
  const logSizeFormatted =
    loadedBytes > 1048576 ? `${(loadedBytes / 1048576).toFixed(1)} MB` : `${(loadedBytes / 1024).toFixed(1)} KB`;

  const errorCount = entries.filter(
    (e) => e.level === "error" || (e.status_code && e.status_code >= 500)
  ).length;

  return (
    <div className="p-6 space-y-4" data-testid="log-viewer-page">
      {/* Header */}
      <div className="flex flex-col sm:flex-row sm:items-center sm:justify-between gap-4">
        <div>
          <h1 className="text-2xl font-bold tracking-tight" data-testid="log-viewer-title">
            {t("logs.title")}
          </h1>
          <p className="text-sm text-muted-foreground">
            {t("logs.description")}
          </p>
        </div>

        <div className="flex flex-wrap items-center gap-2">
          {/* Live Tail Toggle */}
          <Button
            size="sm"
            variant={isLiveTail ? "default" : "outline"}
            onClick={() => {
              setIsLiveTail(!isLiveTail);
              setAutoScroll(!isLiveTail);
            }}
            className={`gap-1.5 text-xs ${isLiveTail ? "bg-green-600 hover:bg-green-700 text-white" : ""}`}
            data-testid="btn-toggle-live-mode"
          >
            {isLiveTail ? (
              <>
                <span className="relative flex h-2 w-2">
                  <span className="animate-ping absolute inline-flex h-full w-full rounded-full bg-white opacity-75"></span>
                  <span className="relative inline-flex rounded-full h-2 w-2 bg-white"></span>
                </span>
                <Pause className="w-3 h-3" />
                {t("logs.liveTailActive")}
              </>
            ) : (
              <>
                <Play className="w-3 h-3 text-green-500" />
                {t("logs.startLiveTail")}
              </>
            )}
          </Button>

          {/* Download Log */}
          <Button
            size="sm"
            variant="outline"
            onClick={handleDownloadLog}
            disabled={entries.length === 0}
            className="gap-1.5 text-xs"
            data-testid="btn-download-log"
          >
            <Download className="w-3.5 h-3.5" />
            {t("logs.download")}
          </Button>

          {/* Refresh */}
          <Button
            size="sm"
            variant="ghost"
            onClick={fetchLogs}
            disabled={isLoading}
            className="h-8 w-8 p-0"
            data-testid="btn-refresh-logs"
            title={t("common.refresh")}
          >
            <RefreshCw className={`w-3.5 h-3.5 ${isLoading ? "animate-spin" : ""}`} />
          </Button>
        </div>
      </div>

      {/* Filter and Type Bar */}
      <Card className="p-3">
        <div className="flex flex-col lg:flex-row lg:items-center justify-between gap-3">
          {/* Domain Picker & Log Type Tabs */}
          <div className="flex flex-wrap items-center gap-3">
            {/* Log Type Pill Tabs */}
            <div className="flex items-center rounded-lg bg-muted p-1 text-xs">
              <button
                type="button"
                onClick={() => setLogType("access")}
                className={`px-3 py-1 rounded-md font-medium transition-colors ${
                  logType === "access"
                    ? "bg-background text-foreground shadow-sm"
                    : "text-muted-foreground hover:text-foreground"
                }`}
                data-testid="tab-log-access"
              >
                {t("logs.tabAccess")}
              </button>
              <button
                type="button"
                onClick={() => setLogType("error")}
                className={`px-3 py-1 rounded-md font-medium transition-colors ${
                  logType === "error"
                    ? "bg-background text-foreground shadow-sm"
                    : "text-muted-foreground hover:text-foreground"
                }`}
                data-testid="tab-log-error"
              >
                {t("logs.tabError")}
              </button>
              <button
                type="button"
                onClick={() => setLogType("php")}
                className={`px-3 py-1 rounded-md font-medium transition-colors ${
                  logType === "php"
                    ? "bg-background text-foreground shadow-sm"
                    : "text-muted-foreground hover:text-foreground"
                }`}
                data-testid="tab-log-php"
              >
                {t("logs.tabPhp")}
              </button>
            </div>
          </div>

          {/* Search Box */}
          <div className="relative w-full lg:w-72">
            <Search className="w-3.5 h-3.5 absolute left-2.5 top-1/2 -translate-y-1/2 text-muted-foreground" />
            <Input
              placeholder={t("logs.searchPlaceholder")}
              value={searchQuery}
              onChange={(e) => setSearchQuery(e.target.value)}
              className="pl-8 text-xs h-8"
              data-testid="input-log-search"
            />
          </div>
        </div>
      </Card>

      {/* Status & Metrics Strip */}
      <div className="flex flex-wrap items-center justify-between text-xs text-muted-foreground px-1">
        <div className="flex items-center gap-4">
          <div>
            <span className="font-semibold text-foreground" data-testid="metric-loaded-lines">
              {entries.length.toLocaleString()}
            </span>{" "}
            {t("logs.linesLoaded")}
          </div>
          <div>
            <span className="font-semibold text-foreground" data-testid="metric-log-size">
              {logSizeFormatted}
            </span>{" "}
            {t("logs.fileSize")}
          </div>
          {errorCount > 0 && (
            <div className="text-destructive font-medium flex items-center gap-1">
              <AlertTriangle className="w-3 h-3" />
              <span data-testid="metric-error-count">{errorCount}</span> {t("logs.errorsDetected")}
            </div>
          )}
        </div>

        <div className="flex items-center gap-2">
          {isLiveTail ? (
            <Badge variant="outline" className="text-green-600 bg-green-500/10 border-green-400 gap-1 text-[11px]" data-testid="badge-live-tail-status">
              <span className="w-1.5 h-1.5 rounded-full bg-green-500 animate-pulse"></span>
              {t("logs.streamLive")}
            </Badge>
          ) : (
            <Badge variant="outline" className="text-muted-foreground text-[11px]" data-testid="badge-live-tail-status">
              {t("logs.streamPaused")}
            </Badge>
          )}

          {truncated && (
            <Badge variant="secondary" className="font-mono text-[11px]" data-testid="badge-log-truncated">
              {t("logs.onlyNewest", "Only the newest lines are shown")}
            </Badge>
          )}
        </div>
      </div>

      {/* Virtualized Terminal Log Display */}
      <div className="h-[580px] w-full">
        <VirtualizedLogList
          entries={entries}
          searchQuery={searchQuery}
          autoScrollBottom={autoScroll}
        />
      </div>
    </div>
  );
}
