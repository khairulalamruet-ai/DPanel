import React, { useState, useEffect } from "react";
import {
  Card,
  CardHeader,
  CardTitle,
  CardDescription,
  CardContent,
  CardFooter,
  Button,
  Input,
  Badge,
} from "@builtpanel/ui";
import {
  Power,
  AlertTriangle,
  RotateCcw,
  CheckCircle2,
  Clock,
  Loader2,
  Lock,
  Radio,
  Server,
  Activity,
} from "lucide-react";
import { useTranslation } from "@builtpanel/i18n";
import { RebootMode } from "../types";
import { reboot, checkNodeOnline } from "../api";

interface ServerRebootTabProps {
  nodeId: string;
  nodeName: string;
  nodeHostname: string;
  canManage: boolean;
  onAuditActionTriggered?: () => void;
}

export function ServerRebootTab({
  nodeId,
  nodeName,
  nodeHostname,
  canManage,
  onAuditActionTriggered,
}: ServerRebootTabProps) {
  const { t } = useTranslation();

  const [rebootMode, setRebootMode] = useState<RebootMode>("graceful");
  const [confirmName, setConfirmName] = useState("");
  const [sudoPassword, setSudoPassword] = useState("");
  const [isSubmitting, setIsSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Status after reboot initiated
  const [rebootState, setRebootState] = useState<"idle" | "rebooting" | "online">("idle");
  const [secondsElapsed, setSecondsElapsed] = useState(0);

  const isConfirmed = confirmName.trim() === nodeName && sudoPassword.trim().length > 0;

  // Polling when rebooting
  useEffect(() => {
    let timer: NodeJS.Timeout;
    let pollInterval: NodeJS.Timeout;

    if (rebootState === "rebooting") {
      timer = setInterval(() => {
        setSecondsElapsed((sec) => sec + 1);
      }, 1000);

      // Start polling node after 5 seconds
      pollInterval = setInterval(async () => {
        if (secondsElapsed > 4) {
          const isOnline = await checkNodeOnline(nodeId);
          if (isOnline) {
            setRebootState("online");
          }
        }
      }, 3000);
    }

    return () => {
      clearInterval(timer);
      clearInterval(pollInterval);
    };
  }, [rebootState, secondsElapsed, nodeId]);

  const handleInitiateReboot = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!isConfirmed || !canManage) return;

    try {
      setIsSubmitting(true);
      setError(null);
      await reboot(nodeId, nodeName, sudoPassword, rebootMode);
      setRebootState("rebooting");
      setSecondsElapsed(0);
      setConfirmName("");
      setSudoPassword("");
      onAuditActionTriggered?.();
    } catch (err: any) {
      setError(err?.message || t("common.error", "Reboot failed. Please verify credentials."));
    } finally {
      setIsSubmitting(false);
    }
  };

  const handleReset = () => {
    setRebootState("idle");
    setSecondsElapsed(0);
  };

  return (
    <div className="space-y-6">
      {/* If reboot is active or completed */}
      {rebootState !== "idle" ? (
        <Card
          className={`border-2 ${
            rebootState === "online" ? "border-emerald-500/50 bg-emerald-500/5" : "border-amber-500/50 bg-amber-500/5"
          }`}
          data-testid="reboot-progress-card"
        >
          <CardContent className="py-12 flex flex-col items-center justify-center text-center space-y-4">
            {rebootState === "rebooting" ? (
              <>
                <div className="relative">
                  <div className="h-16 w-16 rounded-full border-4 border-amber-500/30 border-t-amber-500 animate-spin" />
                  <Activity className="h-6 w-6 text-amber-500 absolute inset-0 m-auto" />
                </div>
                <div className="space-y-1">
                  <h3 className="text-lg font-bold text-foreground" data-testid="text-server-coming-back">
                    {t("system.reboot.comingBackHeading", "Server is coming back online...")}
                  </h3>
                  <p className="text-xs text-muted-foreground max-w-md">
                    {t(
                      "system.reboot.comingBackDesc",
                      "Reboot command issued. We are polling the node heartbeat ping. Connections will restore automatically.",
                    )}
                  </p>
                </div>
                <Badge variant="outline" className="font-mono text-xs gap-1 border-amber-500/30 text-amber-600 dark:text-amber-400">
                  <Clock className="h-3 w-3" />
                  <span>{secondsElapsed}s elapsed</span>
                </Badge>
              </>
            ) : (
              <>
                <div className="h-16 w-16 rounded-full bg-emerald-500/10 flex items-center justify-center text-emerald-600 dark:text-emerald-400">
                  <CheckCircle2 className="h-8 w-8" />
                </div>
                <div className="space-y-1">
                  <h3 className="text-lg font-bold text-foreground" data-testid="text-server-reconnected">
                    {t("system.reboot.onlineHeading", "Server Successfully Reconnected")}
                  </h3>
                  <p className="text-xs text-muted-foreground max-w-md">
                    {t(
                      "system.reboot.onlineDesc",
                      "The server finished rebooting, the node agent resumed communication, and all core services are responsive.",
                    )}
                  </p>
                </div>
                <Button size="sm" onClick={handleReset} data-testid="btn-reboot-done" className="gap-1.5 text-xs">
                  <CheckCircle2 className="h-3.5 w-3.5" />
                  <span>{t("common.done", "Return to Controls")}</span>
                </Button>
              </>
            )}
          </CardContent>
        </Card>
      ) : (
        /* Standard Reboot Configuration & Execution */
        <Card className="border-destructive/30" data-testid="card-server-reboot">
          <CardHeader>
            <div className="flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
              <div>
                <CardTitle className="text-base font-semibold flex items-center gap-2 text-destructive">
                  <Power className="h-4 w-4" />
                  <span data-testid="title-reboot-server">{t("serverTools.rebootSectionTitle", "Server Reboot & Power Cycle")}</span>
                </CardTitle>
                <CardDescription className="text-xs text-muted-foreground">
                  {t(
                    "serverTools.rebootSectionDescription",
                    "Gracefully flush pending I/O buffers, notify active container tenants, and reboot the operating system",
                  )}
                </CardDescription>
              </div>
              <Badge variant="outline" className="text-xs font-mono border-destructive/20 text-destructive">
                {nodeHostname}
              </Badge>
            </div>
          </CardHeader>

          <form onSubmit={handleInitiateReboot}>
            <CardContent className="space-y-6">
              {/* Warning Alert */}
              <div className="rounded-lg border border-destructive/30 bg-destructive/10 p-4 text-xs text-destructive space-y-1.5">
                <div className="flex items-center gap-2 font-semibold text-sm">
                  <AlertTriangle className="h-4 w-4" />
                  <span>{t("serverTools.rebootWarningHeading", "Critical System Action")}</span>
                </div>
                <p className="leading-relaxed opacity-95">
                  {t(
                    "serverTools.rebootWarningText",
                    `Rebooting node ${nodeHostname} will temporarily disrupt hosted sites and active network connections. All services will restart automatically after boot.`,
                    { hostname: nodeHostname },
                  )}
                </p>
              </div>

              {/* Mode Selection */}
              <div className="space-y-2">
                <label className="text-xs font-medium text-foreground">
                  {t("system.reboot.selectMode", "Select Reboot Mechanism")}
                </label>
                <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
                  <div
                    onClick={() => setRebootMode("graceful")}
                    data-testid="option-reboot-graceful"
                    className={`cursor-pointer rounded-lg border p-3 transition-colors ${
                      rebootMode === "graceful"
                        ? "border-primary bg-primary/5 ring-1 ring-primary"
                        : "border-border hover:bg-muted/50"
                    }`}
                  >
                    <div className="flex items-center justify-between">
                      <span className="font-semibold text-xs text-foreground">
                        {t("system.reboot.modeGraceful", "Graceful Reboot (Recommended)")}
                      </span>
                      {rebootMode === "graceful" && <CheckCircle2 className="h-4 w-4 text-primary" />}
                    </div>
                    <p className="mt-1 text-[11px] text-muted-foreground">
                      {t("system.reboot.modeGracefulDesc", "Sends SIGTERM to processes, syncs disk blocks, and orders systemd to reboot cleanly.")}
                    </p>
                  </div>

                  <div
                    onClick={() => setRebootMode("hard")}
                    data-testid="option-reboot-hard"
                    className={`cursor-pointer rounded-lg border p-3 transition-colors ${
                      rebootMode === "hard"
                        ? "border-destructive bg-destructive/5 ring-1 ring-destructive"
                        : "border-border hover:bg-muted/50"
                    }`}
                  >
                    <div className="flex items-center justify-between">
                      <span className="font-semibold text-xs text-destructive">
                        {t("system.reboot.modeHard", "Hard Reboot (Immediate Force)")}
                      </span>
                      {rebootMode === "hard" && <CheckCircle2 className="h-4 w-4 text-destructive" />}
                    </div>
                    <p className="mt-1 text-[11px] text-muted-foreground">
                      {t("system.reboot.modeHardDesc", "Immediate forced reboot without waiting for hung processes. Use only if server is frozen.")}
                    </p>
                  </div>
                </div>
              </div>

              {/* Two-step Confirmation */}
              <div className="rounded-lg border bg-muted/20 p-4 space-y-4">
                <div className="space-y-1.5">
                  <label className="text-xs font-medium text-foreground">
                    {t("system.reboot.typeConfirmLabel", `To confirm, please type node name '${nodeName}':`)}
                  </label>
                  <Input
                    data-testid="input-reboot-confirm"
                    value={confirmName}
                    onChange={(e) => setConfirmName(e.target.value)}
                    placeholder={nodeName}
                    className="font-mono text-xs uppercase"
                    autoComplete="off"
                  />
                </div>

                <div className="space-y-1.5">
                  <label className="flex items-center gap-1.5 text-xs font-medium text-foreground">
                    <Lock className="h-3.5 w-3.5 text-amber-500" />
                    <span>{t("serverTools.sudoPasswordLabel", "Admin Sudo Password")}</span>
                  </label>
                  <Input
                    data-testid="input-reboot-sudo"
                    type="password"
                    value={sudoPassword}
                    onChange={(e) => setSudoPassword(e.target.value)}
                    placeholder={t("serverTools.sudoPasswordPlaceholder", "Enter admin sudo password")}
                    className="text-xs"
                    autoComplete="current-password"
                  />
                </div>

                {error && (
                  <p className="text-xs text-destructive font-medium" data-testid="reboot-error">
                    {error}
                  </p>
                )}
              </div>
            </CardContent>

            <CardFooter className="flex items-center justify-between border-t px-6 py-3">
              <span className="text-[11px] text-muted-foreground">
                {t("serverTools.auditNotice", "This operation is privileged and will be recorded in audit logs.")}
              </span>

              <Button
                type="submit"
                variant="destructive"
                size="sm"
                disabled={!isConfirmed || !canManage || isSubmitting}
                data-testid="btn-execute-reboot"
                className="gap-1.5 text-xs"
              >
                {isSubmitting ? (
                  <Loader2 className="h-3.5 w-3.5 animate-spin" />
                ) : (
                  <Power className="h-3.5 w-3.5" />
                )}
                <span>{t("serverTools.executeReboot", "Confirm & Reboot")}</span>
              </Button>
            </CardFooter>
          </form>
        </Card>
      )}
    </div>
  );
}
