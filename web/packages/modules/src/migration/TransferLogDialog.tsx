import React, { useState, useEffect, useRef } from "react";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
  Button,
  Badge,
} from "@builtpanel/ui";
import { useTranslation } from "@builtpanel/i18n";
import {
  Terminal,
  RotateCw,
  Copy,
  Check,
  Server,
  Globe,
  HardDrive,
  Clock,
  StopCircle,
} from "lucide-react";
import { TransferJob } from "./types";
import { TransferStepTimeline } from "./TransferStepTimeline";

interface TransferLogDialogProps {
  job: TransferJob | null;
  open: boolean;
  onClose: () => void;
  onRetry?: (id: string) => Promise<void>;
  onCancel?: (id: string) => Promise<void>;
}

export function TransferLogDialog({
  job,
  open,
  onClose,
  onRetry,
  onCancel,
}: TransferLogDialogProps) {
  const { t } = useTranslation();
  const logContainerRef = useRef<HTMLDivElement>(null);
  const [copied, setCopied] = useState(false);
  const [actionLoading, setActionLoading] = useState(false);

  useEffect(() => {
    if (logContainerRef.current) {
      logContainerRef.current.scrollTop = logContainerRef.current.scrollHeight;
    }
  }, [job?.logs]);

  if (!job) return null;

  const handleCopyLogs = () => {
    if (!job.logs) return;
    navigator.clipboard.writeText(job.logs.join("\n"));
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  };

  const handleRetry = async () => {
    if (!onRetry) return;
    setActionLoading(true);
    try {
      await onRetry(job.id);
    } finally {
      setActionLoading(false);
    }
  };

  const handleCancel = async () => {
    if (!onCancel) return;
    setActionLoading(true);
    try {
      await onCancel(job.id);
    } finally {
      setActionLoading(false);
    }
  };

  const formatSize = (bytes?: number) => {
    if (!bytes) return "—";
    return `${Math.round((bytes / (1024 * 1024)) * 10) / 10} MB`;
  };

  return (
    <Dialog open={open} onOpenChange={(val) => !val && onClose()}>
      <DialogContent data-testid="transfer-log-dialog" className="sm:max-w-3xl max-h-[90vh] flex flex-col">
        <DialogHeader className="pb-3 border-b">
          <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-2">
            <div className="flex items-center gap-2">
              <div className="p-2 rounded-full bg-primary/10 text-primary">
                <Terminal className="h-5 w-5" />
              </div>
              <div>
                <DialogTitle data-testid="transfer-dialog-title" className="text-base font-bold flex items-center gap-2">
                  <span>{t("migration.transferDetailTitle", "Transfer & Migration Progress")}</span>
                  <span className="font-mono text-xs text-muted-foreground">({job.id})</span>
                </DialogTitle>
                <DialogDescription className="text-xs">
                  {t("migration.transferDetailSubtitle", "Live migration state, execution steps, and service restoration logs.")}
                </DialogDescription>
              </div>
            </div>

            <div className="flex items-center gap-2">
              <Badge
                variant={
                  job.status === "completed"
                    ? "success"
                    : job.status === "running"
                    ? "default"
                    : job.status === "failed"
                    ? "destructive"
                    : "secondary"
                }
                data-testid="transfer-status-badge"
              >
                {job.status.toUpperCase()}
              </Badge>
              <Badge variant="outline" className="font-mono text-xs uppercase">
                {job.source_type}
              </Badge>
            </div>
          </div>

          {/* Progress Bar */}
          <div className="pt-3 space-y-1">
            <div className="flex items-center justify-between text-xs font-mono">
              <span className="text-muted-foreground font-semibold">{job.current_step}</span>
              <span className="font-bold">{job.progress_percent}%</span>
            </div>
            <div className="w-full bg-muted rounded-full h-2 overflow-hidden">
              <div
                className={`h-full transition-all duration-300 ${
                  job.status === "failed"
                    ? "bg-destructive"
                    : job.status === "completed"
                    ? "bg-emerald-500"
                    : "bg-primary"
                }`}
                style={{ width: `${job.progress_percent}%` }}
              />
            </div>
          </div>
        </DialogHeader>

        {/* Metadata Bar */}
        <div className="grid grid-cols-2 sm:grid-cols-4 gap-2 py-3 px-1 text-xs border-b bg-muted/20">
          <div className="flex items-center gap-1.5 truncate">
            <Server className="h-3.5 w-3.5 text-muted-foreground shrink-0" />
            <span className="text-muted-foreground">{t("migration.sourceHost", "Source")}:</span>
            <span className="font-mono font-semibold truncate">{job.source_host || "Direct Archive"}</span>
          </div>

          <div className="flex items-center gap-1.5 truncate">
            <Globe className="h-3.5 w-3.5 text-muted-foreground shrink-0" />
            <span className="text-muted-foreground">{t("migration.targetDomain", "Domain")}:</span>
            <span className="font-mono font-semibold truncate">{job.target_domain}</span>
          </div>

          <div className="flex items-center gap-1.5">
            <HardDrive className="h-3.5 w-3.5 text-muted-foreground shrink-0" />
            <span className="text-muted-foreground">{t("migration.archiveSize", "Size")}:</span>
            <span className="font-mono font-semibold">{formatSize(job.size_bytes)}</span>
          </div>

          <div className="flex items-center gap-1.5">
            <Clock className="h-3.5 w-3.5 text-muted-foreground shrink-0" />
            <span className="text-muted-foreground">{t("migration.startedAt", "Started")}:</span>
            <span className="font-mono">{new Date(job.created_at).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}</span>
          </div>
        </div>

        {/* Two-column layout: Steps on left, Live Log on right */}
        <div className="grid grid-cols-1 md:grid-cols-5 gap-4 py-3 flex-1 overflow-hidden min-h-[300px]">
          {/* Step Pipeline (2 cols) */}
          <div className="md:col-span-2 overflow-y-auto pr-2 border-r border-border">
            <h4 className="text-xs font-semibold text-muted-foreground mb-3 uppercase tracking-wider">
              {t("migration.executionSteps", "Migration Pipeline")}
            </h4>
            <TransferStepTimeline steps={job.steps} />
          </div>

          {/* Monospace Live Log Terminal (3 cols) */}
          <div className="md:col-span-3 flex flex-col h-full overflow-hidden">
            <div className="flex items-center justify-between pb-2">
              <h4 className="text-xs font-semibold text-muted-foreground uppercase tracking-wider flex items-center gap-1.5">
                <Terminal className="h-3.5 w-3.5" />
                {t("migration.transferLogs", "Transfer & Restore Log")}
              </h4>
              <button
                type="button"
                onClick={handleCopyLogs}
                className="text-[11px] text-muted-foreground hover:text-foreground flex items-center gap-1"
                data-testid="btn-copy-transfer-logs"
              >
                {copied ? <Check className="h-3 w-3 text-emerald-500" /> : <Copy className="h-3 w-3" />}
                {copied ? t("common.copied", "Copied") : t("common.copy", "Copy Log")}
              </button>
            </div>

            <div
              ref={logContainerRef}
              className="flex-1 bg-black text-emerald-400 p-3 rounded-lg font-mono text-xs overflow-y-auto border border-zinc-800 space-y-1 select-text"
              data-testid="transfer-log-console"
            >
              {job.logs && job.logs.length > 0 ? (
                job.logs.map((logLine, idx) => (
                  <div key={idx} className="leading-relaxed">
                    <span className="text-zinc-600 mr-2">[{idx + 1}]</span>
                    {logLine.startsWith("ERROR") || logLine.startsWith("FAILED") ? (
                      <span className="text-red-400">{logLine}</span>
                    ) : logLine.startsWith("SUCCESS") || logLine.startsWith("RESTORED") ? (
                      <span className="text-emerald-300 font-bold">{logLine}</span>
                    ) : (
                      <span className="text-zinc-300">{logLine}</span>
                    )}
                  </div>
                ))
              ) : (
                <div className="text-zinc-500 italic">
                  {t("migration.waitingForLogs", "Waiting for remote agent output...")}
                </div>
              )}
            </div>
          </div>
        </div>

        {/* Footer with Controls */}
        <DialogFooter className="pt-3 border-t flex items-center justify-between sm:justify-between w-full">
          <div>
            {job.status === "running" && onCancel && (
              <Button
                variant="outline"
                size="sm"
                onClick={handleCancel}
                disabled={actionLoading}
                className="text-destructive hover:bg-destructive/10 gap-1.5 text-xs"
                data-testid="btn-cancel-transfer"
              >
                <StopCircle className="h-3.5 w-3.5" />
                {t("migration.cancelTransferBtn", "Abort Transfer")}
              </Button>
            )}

            {job.status === "failed" && onRetry && (
              <Button
                variant="outline"
                size="sm"
                onClick={handleRetry}
                disabled={actionLoading}
                className="gap-1.5 text-xs"
                data-testid="btn-retry-transfer"
              >
                <RotateCw className="h-3.5 w-3.5" />
                {t("migration.retryTransferBtn", "Retry Migration")}
              </Button>
            )}
          </div>

          <Button type="button" onClick={onClose} data-testid="btn-close-log-dialog">
            {t("common.close", "Close")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
