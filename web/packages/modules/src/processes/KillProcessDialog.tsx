import React, { useState } from "react";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogFooter,
  Button,
  Badge,
  Input,
  useToast,
} from "@builtpanel/ui";
import { AlertTriangle, ShieldAlert, Terminal, XCircle, Power } from "lucide-react";
import { useTranslation } from "@builtpanel/i18n";
import { ProcessItem, ProcessSignal } from "./types";

interface KillProcessDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  process: ProcessItem | null;
  defaultSignal?: ProcessSignal;
  isSudoMode?: boolean;
  onConfirm: (pid: number, signal: ProcessSignal, sudoPass?: string) => Promise<void>;
}

export function KillProcessDialog({
  open,
  onOpenChange,
  process,
  defaultSignal = "SIGTERM",
  isSudoMode = false,
  onConfirm,
}: KillProcessDialogProps) {
  const { t } = useTranslation();
  const { toast } = useToast();

  const [signal, setSignal] = useState<ProcessSignal>(defaultSignal);
  const [sudoPassword, setSudoPassword] = useState("");
  const [isSubmitting, setIsSubmitting] = useState(false);

  React.useEffect(() => {
    if (open) {
      setSignal(defaultSignal);
      setSudoPassword("");
    }
  }, [open, defaultSignal]);

  if (!process) return null;

  const handleExecute = async (e: React.FormEvent) => {
    e.preventDefault();
    try {
      setIsSubmitting(true);
      await onConfirm(process.pid, signal, isSudoMode ? sudoPassword : undefined);
      onOpenChange(false);
      setSudoPassword("");
    } catch (err: any) {
      toast({
        title: t("common.error"),
        description: err.message || t("common.unknownError"),
        variant: "destructive",
      });
    } finally {
      setIsSubmitting(false);
    }
  };

  const isForce = signal === "SIGKILL";

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-[480px]">
        <DialogHeader>
          <div className="flex items-center gap-2">
            <div className={`p-2 rounded-lg ${isForce ? "bg-destructive/10 text-destructive" : "bg-amber-500/10 text-amber-500"}`}>
              {isForce ? <ShieldAlert className="w-5 h-5" /> : <AlertTriangle className="w-5 h-5" />}
            </div>
            <div>
              <DialogTitle data-testid="kill-process-dialog-title">
                {isForce ? t("processes.titleKillForce") : t("processes.titleKillGraceful")}
              </DialogTitle>
              <p className="text-xs text-muted-foreground mt-0.5">
                PID {process.pid} — {process.user}
              </p>
            </div>
          </div>
        </DialogHeader>

        <form onSubmit={handleExecute} className="space-y-4 py-2">
          {/* Target Process Info */}
          <div className="rounded-lg bg-zinc-950 p-3 text-xs font-mono border border-zinc-800 text-zinc-200 space-y-1">
            <div className="flex items-center justify-between text-[11px] text-zinc-400">
              <span>USER: {process.user}</span>
              <span>CPU: {process.cpu_percent}% | RSS: {process.mem_rss_mb} MB</span>
            </div>
            <div className="text-zinc-100 font-semibold break-all" data-testid="kill-process-command">
              $ {process.command}
            </div>
          </div>

          {/* Signal Radio Selection */}
          <div className="space-y-2">
            <label className="text-xs font-semibold uppercase tracking-wider text-muted-foreground">
              {t("processes.selectSignal")}
            </label>
            <div className="grid grid-cols-2 gap-2">
              <button
                type="button"
                onClick={() => setSignal("SIGTERM")}
                className={`p-3 rounded-lg border text-left transition-colors flex flex-col justify-between ${
                  signal === "SIGTERM"
                    ? "border-amber-500 bg-amber-500/10"
                    : "border-border hover:bg-muted"
                }`}
                data-testid="radio-signal-sigterm"
              >
                <div className="flex items-center justify-between w-full">
                  <span className="font-semibold text-xs text-foreground">SIGTERM (15)</span>
                  <Badge variant="outline" className="text-[10px] text-amber-600 border-amber-300">
                    {t("processes.graceful")}
                  </Badge>
                </div>
                <p className="text-[11px] text-muted-foreground mt-1.5 leading-snug">
                  {t("processes.sigtermDesc")}
                </p>
              </button>

              <button
                type="button"
                onClick={() => setSignal("SIGKILL")}
                className={`p-3 rounded-lg border text-left transition-colors flex flex-col justify-between ${
                  signal === "SIGKILL"
                    ? "border-destructive bg-destructive/10"
                    : "border-border hover:bg-muted"
                }`}
                data-testid="radio-signal-sigkill"
              >
                <div className="flex items-center justify-between w-full">
                  <span className="font-semibold text-xs text-destructive">SIGKILL (9)</span>
                  <Badge variant="destructive" className="text-[10px]">
                    {t("processes.force")}
                  </Badge>
                </div>
                <p className="text-[11px] text-muted-foreground mt-1.5 leading-snug">
                  {t("processes.sigkillDesc")}
                </p>
              </button>
            </div>
          </div>

          {/* Sudo Password Input if in sudo mode */}
          {isSudoMode && (
            <div className="space-y-1.5 pt-1">
              <label className="text-xs font-semibold text-foreground flex items-center gap-1.5">
                <ShieldAlert className="w-3.5 h-3.5 text-destructive" />
                {t("processes.sudoPasswordRequired")}
              </label>
              <Input
                type="password"
                required
                placeholder={t("processes.enterSudoPassword")}
                value={sudoPassword}
                onChange={(e) => setSudoPassword(e.target.value)}
                data-testid="input-sudo-password"
              />
            </div>
          )}

          <DialogFooter className="pt-2 border-t">
            <Button
              type="button"
              variant="outline"
              onClick={() => onOpenChange(false)}
              disabled={isSubmitting}
            >
              {t("common.cancel")}
            </Button>
            <Button
              type="submit"
              variant={isForce ? "destructive" : "default"}
              disabled={isSubmitting || (isSudoMode && !sudoPassword)}
              data-testid="btn-confirm-kill-process"
              className="gap-1.5"
            >
              <Power className="w-3.5 h-3.5" />
              {isSubmitting
                ? t("common.processing")
                : isForce
                ? t("processes.btnForceKill")
                : t("processes.btnTerminate")}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
