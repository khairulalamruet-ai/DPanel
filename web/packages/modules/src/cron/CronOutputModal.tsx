import React, { useState } from "react";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogFooter,
  Button,
  Badge,
  useToast,
} from "@builtpanel/ui";
import { Terminal, Copy, Check, Play, CheckCircle2, AlertTriangle, Clock } from "lucide-react";
import { useTranslation } from "@builtpanel/i18n";
import { CronJob } from "./types";

interface CronOutputModalProps {
  job: CronJob | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onRunNow?: (job: CronJob) => Promise<void>;
}

export function CronOutputModal({
  job,
  open,
  onOpenChange,
  onRunNow,
}: CronOutputModalProps) {
  const { t } = useTranslation();
  const { toast } = useToast();
  const [copied, setCopied] = useState(false);
  const [isRunning, setIsRunning] = useState(false);

  if (!job) return null;

  const handleCopy = () => {
    if (!job.last_output) return;
    navigator.clipboard.writeText(job.last_output);
    setCopied(true);
    toast({
      title: t("common.success"),
      description: t("cron.outputCopied"),
    });
    setTimeout(() => setCopied(false), 2000);
  };

  const handleRun = async () => {
    if (!onRunNow) return;
    try {
      setIsRunning(true);
      await onRunNow(job);
    } finally {
      setIsRunning(false);
    }
  };

  const isSuccess = job.last_status === "success" || job.last_exit_code === 0;
  const isFailed = job.last_status === "failed" || (job.last_exit_code !== null && job.last_exit_code > 0);

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-[650px] max-h-[85vh] flex flex-col">
        <DialogHeader className="pb-3 border-b">
          <div className="flex items-center justify-between gap-3 pr-6">
            <div className="flex items-center gap-2">
              <div className="p-2 rounded-lg bg-primary/10 text-primary">
                <Terminal className="w-5 h-5" />
              </div>
              <div>
                <DialogTitle data-testid="cron-output-modal-title">
                  {job.name}
                </DialogTitle>
                <p className="text-xs text-muted-foreground font-mono mt-0.5 truncate max-w-[420px]">
                  $ {job.command}
                </p>
              </div>
            </div>

            <div>
              {job.last_status === "running" ? (
                <Badge variant="outline" className="gap-1 animate-pulse" data-testid="output-badge-status">
                  <Clock className="w-3 h-3" />
                  {t("cron.running")}
                </Badge>
              ) : isSuccess ? (
                <Badge variant="outline" className="text-green-600 border-green-300 dark:border-green-800 bg-green-500/10 gap-1" data-testid="output-badge-status">
                  <CheckCircle2 className="w-3 h-3" />
                  {t("cron.statusSuccess")} (0)
                </Badge>
              ) : isFailed ? (
                <Badge variant="destructive" className="gap-1" data-testid="output-badge-status">
                  <AlertTriangle className="w-3 h-3" />
                  {t("cron.statusFailed")} ({job.last_exit_code ?? 1})
                </Badge>
              ) : (
                <Badge variant="outline" className="text-muted-foreground gap-1" data-testid="output-badge-status">
                  <Clock className="w-3 h-3" />
                  {t("cron.statusIdle")}
                </Badge>
              )}
            </div>
          </div>
        </DialogHeader>

        {/* Execution Metadata */}
        <div className="py-2 grid grid-cols-2 sm:grid-cols-3 gap-2 text-xs text-muted-foreground bg-muted/40 p-2.5 rounded-md">
          <div>
            <span className="font-medium text-foreground">{t("cron.lastRun")}: </span>
            {job.last_run_at ? new Date(job.last_run_at).toLocaleString() : t("cron.neverRun")}
          </div>
          <div>
            <span className="font-medium text-foreground">{t("cron.duration")}: </span>
            {job.last_duration_ms != null ? `${job.last_duration_ms} ms` : "—"}
          </div>
          <div>
            <span className="font-medium text-foreground">{t("cron.timeout")}: </span>
            {job.timeout_seconds}s
          </div>
        </div>

        {/* Console / Output display */}
        <div className="flex-1 min-h-[220px] max-h-[360px] overflow-auto rounded-lg bg-zinc-950 p-3.5 text-zinc-100 font-mono text-xs relative border border-zinc-800">
          <div className="flex items-center justify-between pb-2 border-b border-zinc-800/80 mb-2">
            <span className="text-[11px] text-zinc-400 font-sans">{t("cron.outputTerminal")}</span>
            <button
              onClick={handleCopy}
              className="flex items-center gap-1 text-[11px] text-zinc-400 hover:text-zinc-200 transition-colors"
              title="Copy output"
              data-testid="btn-copy-cron-output"
            >
              {copied ? <Check className="w-3.5 h-3.5 text-green-400" /> : <Copy className="w-3.5 h-3.5" />}
              {copied ? t("common.copied") : t("common.copy")}
            </button>
          </div>

          <pre
            className="whitespace-pre-wrap break-all leading-relaxed"
            data-testid="cron-output-content"
          >
            {job.last_output || t("cron.noOutputAvailable")}
          </pre>
        </div>

        <DialogFooter className="pt-3 border-t flex flex-row items-center justify-between sm:justify-between">
          <Button
            type="button"
            variant="outline"
            size="sm"
            onClick={() => onOpenChange(false)}
            data-testid="btn-close-cron-output"
          >
            {t("common.close")}
          </Button>

          {onRunNow && (
            <Button
              type="button"
              size="sm"
              disabled={isRunning || job.last_status === "running"}
              onClick={handleRun}
              className="gap-1.5"
              data-testid="btn-run-from-output-modal"
            >
              <Play className={`w-3.5 h-3.5 ${isRunning ? "animate-spin" : ""}`} />
              {isRunning ? t("cron.running") : t("cron.runNow")}
            </Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
