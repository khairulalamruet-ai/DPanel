import React, { useState, useEffect, useRef } from "react";
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
  useToast,
} from "@builtpanel/ui";
import {
  ArrowRightLeft,
  Server,
  Lock,
  Play,
  CheckCircle2,
  AlertCircle,
  Loader2,
  Terminal,
  ShieldCheck,
  RotateCcw,
} from "lucide-react";
import { useTranslation } from "@builtpanel/i18n";
import { MigrationRequest, MigrationStatus } from "../types";
import { startMigration, getMigrationStatus } from "../api";
import { SudoConfirmModal } from "../SudoConfirmModal";

interface ServerMigrateTabProps {
  nodeId: string;
  canManage: boolean;
  onAuditActionTriggered?: () => void;
}

export function ServerMigrateTab({
  nodeId,
  canManage,
  onAuditActionTriggered,
}: ServerMigrateTabProps) {
  const { t } = useTranslation();
  const { toast } = useToast();

  // Migration target form
  const [remoteHost, setRemoteHost] = useState("");
  const [remotePort, setRemotePort] = useState(22);
  const [remoteUser, setRemoteUser] = useState("root");
  // Important Rule: The remote password lives ONLY in the form field.
  // Never put it in browser storage, URLs, logs or error reports. Clear the field after submit.
  const [remotePassword, setRemotePassword] = useState("");
  const [transferAccounts, setTransferAccounts] = useState(true);
  const [transferConfigs, setTransferConfigs] = useState(true);
  const [transferDatabases, setTransferDatabases] = useState(true);

  // Sudo confirmation modal
  const [isSudoModalOpen, setIsSudoModalOpen] = useState(false);
  const [isSubmitting, setIsSubmitting] = useState(false);

  // Progress tracking
  const [status, setStatus] = useState<MigrationStatus>({
    id: "mig-idle",
    status: "idle",
    progress: 0,
    current_step: "Ready",
    steps: [],
    logs: [],
  });

  const canStart =
    remoteHost.trim().length > 0 &&
    remoteUser.trim().length > 0 &&
    remotePassword.trim().length > 0 &&
    canManage;

  const fetchStatus = async () => {
    if (!nodeId) return;
    try {
      const res = await getMigrationStatus(nodeId);
      setStatus(res);
    } catch {
      // ignore
    }
  };

  useEffect(() => {
    void fetchStatus();
  }, [nodeId]);

  // Polling during running migration
  useEffect(() => {
    let timer: NodeJS.Timeout;
    if (status.status === "running" || status.status === "connecting" || status.status === "transferring") {
      timer = setInterval(fetchStatus, 3000);
    }
    return () => clearInterval(timer);
  }, [status.status]);

  const pendingPasswordRef = useRef<string>("");

  const handleOpenSudo = () => {
    pendingPasswordRef.current = remotePassword;
    // Rule: Remote password lives only in form field; clear field immediately upon submission/initiation
    setRemotePassword("");
    setIsSudoModalOpen(true);
  };

  const handleStartMigration = async (sudoPassword: string) => {
    const passwordToSend = pendingPasswordRef.current || remotePassword;
    pendingPasswordRef.current = "";
    setRemotePassword("");

    const payload: MigrationRequest = {
      remote_host: remoteHost.trim(),
      port: remotePort,
      user: remoteUser.trim(),
      password: passwordToSend,
      transfer_accounts: transferAccounts,
      transfer_configs: transferConfigs,
      transfer_databases: transferDatabases,
    };

    try {
      setIsSubmitting(true);
      await startMigration(nodeId, payload, sudoPassword);
      toast({
        title: t("common.success"),
        description: t("system.migrate.initiatedSuccess", "Server migration process started over encrypted SSH."),
      });
      await fetchStatus();
      onAuditActionTriggered?.();
    } catch (err: any) {
      toast({
        title: t("common.error"),
        description: err.message || t("common.unknownError"),
        variant: "destructive",
      });
      throw err;
    } finally {
      setIsSubmitting(false);
    }
  };

  return (
    <div className="space-y-6">
      {/* 1. Migration Setup Form */}
      <Card data-testid="card-server-migrate">
        <CardHeader>
          <div className="flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
            <div>
              <CardTitle className="text-base font-semibold flex items-center gap-2">
                <ArrowRightLeft className="h-4 w-4 text-primary" />
                <span data-testid="title-server-migrate">{t("system.migrate.title", "Server-to-Server Full Migration")}</span>
              </CardTitle>
              <CardDescription className="text-xs text-muted-foreground">
                {t(
                  "system.migrate.description",
                  "Transfer accounts, databases, configs and system state to another server instance over a secured SSH connection",
                )}
              </CardDescription>
            </div>
            <Badge
              variant={status.status === "completed" ? "default" : status.status === "running" ? "outline" : "secondary"}
              className="font-mono text-xs capitalize"
              data-testid="badge-migration-status"
            >
              {status.status}
            </Badge>
          </div>
        </CardHeader>

        <CardContent className="space-y-4">
          {/* Security Notice about password handling */}
          <div className="rounded-lg border border-primary/20 bg-primary/5 p-3 text-xs text-foreground flex items-center gap-2.5">
            <ShieldCheck className="h-4 w-4 text-primary shrink-0" />
            <span>
              {t(
                "system.migrate.securityNotice",
                "Strict Security Enforced: The remote password lives exclusively in temporary memory during handover. It is never stored in browser storage, URLs, log files, or audit reports.",
              )}
            </span>
          </div>

          {/* Remote connection fields */}
          <div className="grid grid-cols-1 gap-4 sm:grid-cols-3">
            <div className="space-y-1.5 sm:col-span-2">
              <label className="text-xs font-medium text-foreground">
                {t("system.migrate.remoteHost", "Remote Destination Host / IP")}
              </label>
              <Input
                data-testid="input-migrate-host"
                placeholder="203.0.113.10 or target.example.com"
                value={remoteHost}
                onChange={(e) => setRemoteHost(e.target.value)}
                className="font-mono text-xs"
                disabled={status.status === "running"}
              />
            </div>

            <div className="space-y-1.5">
              <label className="text-xs font-medium text-foreground">
                {t("system.migrate.remotePort", "SSH Port")}
              </label>
              <Input
                data-testid="input-migrate-port"
                type="number"
                min={1}
                max={65535}
                value={remotePort}
                onChange={(e) => setRemotePort(Number(e.target.value) || 22)}
                className="font-mono text-xs"
                disabled={status.status === "running"}
              />
            </div>
          </div>

          <div className="grid grid-cols-1 gap-4 sm:grid-cols-2">
            <div className="space-y-1.5">
              <label className="text-xs font-medium text-foreground">
                {t("system.migrate.remoteUser", "Remote SSH Username")}
              </label>
              <Input
                data-testid="input-migrate-user"
                value={remoteUser}
                onChange={(e) => setRemoteUser(e.target.value)}
                placeholder="root"
                className="font-mono text-xs"
                disabled={status.status === "running"}
              />
            </div>

            <div className="space-y-1.5">
              <label className="flex items-center gap-1.5 text-xs font-medium text-foreground">
                <Lock className="h-3 w-3 text-amber-500" />
                <span>{t("system.migrate.remotePassword", "Remote Password")}</span>
              </label>
              <Input
                data-testid="input-migrate-password"
                type="password"
                value={remotePassword}
                onChange={(e) => setRemotePassword(e.target.value)}
                placeholder={t("system.migrate.remotePasswordPlaceholder", "Password (cleared after submission)")}
                className="text-xs"
                autoComplete="new-password"
                disabled={status.status === "running"}
              />
            </div>
          </div>

          {/* Scope checkboxes */}
          <div className="space-y-2 pt-2 border-t">
            <label className="text-xs font-medium text-foreground">
              {t("system.migrate.scopeHeading", "Migration Scope Components")}
            </label>
            <div className="flex flex-wrap gap-4 text-xs">
              <label className="flex items-center gap-2 cursor-pointer">
                <input
                  type="checkbox"
                  data-testid="check-transfer-accounts"
                  checked={transferAccounts}
                  onChange={(e) => setTransferAccounts(e.target.checked)}
                  className="rounded border"
                  disabled={status.status === "running"}
                />
                <span>{t("system.migrate.scopeAccounts", "Tenant Accounts & Web Roots (/srv/tenants)")}</span>
              </label>

              <label className="flex items-center gap-2 cursor-pointer">
                <input
                  type="checkbox"
                  data-testid="check-transfer-databases"
                  checked={transferDatabases}
                  onChange={(e) => setTransferDatabases(e.target.checked)}
                  className="rounded border"
                  disabled={status.status === "running"}
                />
                <span>{t("system.migrate.scopeDatabases", "Database Engines & Schemas (MySQL / PostgreSQL)")}</span>
              </label>

              <label className="flex items-center gap-2 cursor-pointer">
                <input
                  type="checkbox"
                  data-testid="check-transfer-configs"
                  checked={transferConfigs}
                  onChange={(e) => setTransferConfigs(e.target.checked)}
                  className="rounded border"
                  disabled={status.status === "running"}
                />
                <span>{t("system.migrate.scopeConfigs", "Web & DNS Policies, SSL Certificates")}</span>
              </label>
            </div>
          </div>
        </CardContent>

        <CardFooter className="flex items-center justify-between border-t px-6 py-3">
          <span className="text-[11px] text-muted-foreground">
            {t("system.migrate.confirmBeforeStart", "Initiating migration requires administrator sudo verification.")}
          </span>

          <Button
            size="sm"
            disabled={!canStart || status.status === "running"}
            data-testid="btn-start-migration"
            onClick={handleOpenSudo}
            className="gap-1.5 text-xs"
          >
            {status.status === "running" ? (
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
            ) : (
              <Play className="h-3.5 w-3.5" />
            )}
            <span>{t("system.migrate.startBtn", "Start Migration")}</span>
          </Button>
        </CardFooter>
      </Card>

      {/* 2. Live Progress & Execution Logs */}
      {(status.status !== "idle" || status.logs.length > 0) && (
        <Card data-testid="card-migration-progress">
          <CardHeader>
            <div className="flex items-center justify-between">
              <CardTitle className="text-base font-semibold flex items-center gap-2">
                <Terminal className="h-4 w-4 text-primary" />
                <span>{t("system.migrate.progressTitle", "Migration Progress & Streaming Logs")}</span>
              </CardTitle>
              <Badge variant="outline" className="font-mono text-xs">
                {status.progress}%
              </Badge>
            </div>
          </CardHeader>

          <CardContent className="space-y-4">
            {/* Progress Bar */}
            <div className="space-y-1">
              <div className="flex justify-between text-xs text-muted-foreground">
                <span className="font-medium text-foreground">{status.current_step}</span>
                <span>{status.progress}%</span>
              </div>
              <div className="h-2 w-full overflow-hidden rounded-full bg-muted">
                <div
                  className="h-full bg-primary transition-all duration-300"
                  style={{ width: `${status.progress}%` }}
                />
              </div>
            </div>

            {/* Checklist steps */}
            {status.steps && status.steps.length > 0 && (
              <div className="grid grid-cols-1 gap-2 sm:grid-cols-2 pt-2">
                {status.steps.map((st) => (
                  <div
                    key={st.id}
                    className="flex items-center gap-2 rounded border bg-muted/20 p-2 text-xs"
                  >
                    {st.status === "completed" && <CheckCircle2 className="h-3.5 w-3.5 text-emerald-500" />}
                    {st.status === "in_progress" && <Loader2 className="h-3.5 w-3.5 text-primary animate-spin" />}
                    {st.status === "pending" && <div className="h-3.5 w-3.5 rounded-full border" />}
                    {st.status === "failed" && <AlertCircle className="h-3.5 w-3.5 text-destructive" />}
                    <span className={st.status === "completed" ? "line-through text-muted-foreground" : "font-medium"}>
                      {st.name}
                    </span>
                  </div>
                ))}
              </div>
            )}

            {/* Terminal log box */}
            <div className="space-y-1.5 pt-2">
              <label className="text-xs font-medium text-foreground">
                {t("system.migrate.liveLogOutput", "Live Execution Output:")}
              </label>
              <div
                data-testid="migration-log-box"
                className="max-h-56 overflow-y-auto rounded-md border bg-zinc-950 p-3 font-mono text-xs text-zinc-200 space-y-1"
              >
                {status.logs.map((log, idx) => (
                  <div key={idx} className="leading-relaxed">
                    {log}
                  </div>
                ))}
              </div>
            </div>
          </CardContent>
        </Card>
      )}

      {/* Sudo Modal */}
      <SudoConfirmModal
        open={isSudoModalOpen}
        onOpenChange={setIsSudoModalOpen}
        title={t("system.migrate.sudoConfirmTitle", "Confirm Host Migration Handover")}
        description={t(
          "system.migrate.sudoConfirmDesc",
          "You are about to start a comprehensive system migration to a remote host. Please enter your administrator password to authenticate and initiate transfer.",
        )}
        confirmLabel={t("system.migrate.startBtn", "Start Migration")}
        onConfirm={handleStartMigration}
      />
    </div>
  );
}
