import React, { useState } from "react";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
  Button,
  Input,
} from "@builtpanel/ui";
import { useTranslation } from "@builtpanel/i18n";
import { ArrowRightLeft, Server, Shield, CheckCircle2 } from "lucide-react";
import { MigrationSourceType, RemoteMigrationPayload } from "./types";

interface NewTransferWizardDialogProps {
  open: boolean;
  onClose: () => void;
  onSubmit: (payload: RemoteMigrationPayload) => Promise<void>;
}

export function NewTransferWizardDialog({
  open,
  onClose,
  onSubmit,
}: NewTransferWizardDialogProps) {
  const { t } = useTranslation();

  const [sourceType, setSourceType] = useState<MigrationSourceType>("cpanel");
  const [remoteHost, setRemoteHost] = useState("");
  const [remotePort, setRemotePort] = useState(22);
  const [remoteUser, setRemoteUser] = useState("root");
  const [authMethod, setAuthMethod] = useState<"password" | "ssh_key">("password");
  const [password, setPassword] = useState("");
  const [sshKey, setSshKey] = useState("");
  const [targetDomain, setTargetDomain] = useState("");
  const [tenantUser, setTenantUser] = useState("");
  const [dryRun, setDryRun] = useState(false);

  const [isSubmitting, setIsSubmitting] = useState(false);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setErrorMsg(null);

    if (!remoteHost.trim()) {
      setErrorMsg("Please specify the source server hostname or IP address.");
      return;
    }
    if (!targetDomain.trim()) {
      setErrorMsg("Please specify the primary target domain.");
      return;
    }
    if (!tenantUser.trim()) {
      setErrorMsg("Please specify the destination tenant username.");
      return;
    }

    setIsSubmitting(true);
    try {
      await onSubmit({
        source_type: sourceType,
        remote_host: remoteHost.trim(),
        remote_ssh_port: Number(remotePort) || 22,
        remote_user: remoteUser.trim(),
        auth_method: authMethod,
        password: authMethod === "password" ? password : undefined,
        ssh_key: authMethod === "ssh_key" ? sshKey : undefined,
        target_domain: targetDomain.trim().toLowerCase(),
        tenant_username: tenantUser.trim().toLowerCase(),
        dry_run: dryRun,
      });
      onClose();
    } catch (err: any) {
      setErrorMsg(err.message || "Failed to initiate migration transfer.");
    } finally {
      setIsSubmitting(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={(val) => !val && onClose()}>
      <DialogContent data-testid="new-transfer-dialog" className="sm:max-w-lg max-h-[90vh] overflow-y-auto">
        <DialogHeader>
          <div className="flex items-center gap-2">
            <div className="p-2 rounded-full bg-primary/10 text-primary">
              <ArrowRightLeft className="h-5 w-5" />
            </div>
            <div>
              <DialogTitle data-testid="new-transfer-title">
                {t("migration.newTransferTitle", "Remote Server Migration")}
              </DialogTitle>
              <DialogDescription className="text-xs">
                {t("migration.newTransferSubtitle", "Import cPanel, CyberPanel, or OpenPanel accounts automatically via SSH.")}
              </DialogDescription>
            </div>
          </div>
        </DialogHeader>

        <form onSubmit={handleSubmit} className="space-y-4 py-2">
          {errorMsg && (
            <div className="p-3 bg-red-50 text-red-700 dark:bg-red-950/40 dark:text-red-300 rounded-lg text-xs">
              {errorMsg}
            </div>
          )}

          {/* 1. Source Server Platform */}
          <div>
            <label className="text-xs font-semibold block mb-1.5">
              {t("migration.sourcePlatformLabel", "Source Control Panel Platform")}
            </label>
            <div className="grid grid-cols-2 gap-2">
              <button
                type="button"
                onClick={() => setSourceType("cpanel")}
                className={`p-2.5 rounded-lg border text-left text-xs transition-colors flex items-center justify-between ${
                  sourceType === "cpanel"
                    ? "border-primary bg-primary/5 font-semibold text-primary"
                    : "border-border hover:bg-muted/50"
                }`}
                data-testid="source-type-cpanel"
              >
                <span>cPanel (cpmove)</span>
                {sourceType === "cpanel" && <CheckCircle2 className="h-3.5 w-3.5 text-primary" />}
              </button>

              <button
                type="button"
                onClick={() => setSourceType("cyberpanel")}
                className={`p-2.5 rounded-lg border text-left text-xs transition-colors flex items-center justify-between ${
                  sourceType === "cyberpanel"
                    ? "border-primary bg-primary/5 font-semibold text-primary"
                    : "border-border hover:bg-muted/50"
                }`}
                data-testid="source-type-cyberpanel"
              >
                <span>CyberPanel</span>
                {sourceType === "cyberpanel" && <CheckCircle2 className="h-3.5 w-3.5 text-primary" />}
              </button>

              <button
                type="button"
                onClick={() => setSourceType("openpanel")}
                className={`p-2.5 rounded-lg border text-left text-xs transition-colors flex items-center justify-between ${
                  sourceType === "openpanel"
                    ? "border-primary bg-primary/5 font-semibold text-primary"
                    : "border-border hover:bg-muted/50"
                }`}
                data-testid="source-type-openpanel"
              >
                <span>OpenPanel</span>
                {sourceType === "openpanel" && <CheckCircle2 className="h-3.5 w-3.5 text-primary" />}
              </button>

              <button
                type="button"
                onClick={() => setSourceType("builtpanel_archive")}
                className={`p-2.5 rounded-lg border text-left text-xs transition-colors flex items-center justify-between ${
                  sourceType === "builtpanel_archive"
                    ? "border-primary bg-primary/5 font-semibold text-primary"
                    : "border-border hover:bg-muted/50"
                }`}
                data-testid="source-type-archive"
              >
                <span>Built Panel Backup</span>
                {sourceType === "builtpanel_archive" && <CheckCircle2 className="h-3.5 w-3.5 text-primary" />}
              </button>
            </div>
          </div>

          {/* 2. Remote Connection Details */}
          <div className="space-y-3 pt-1 border-t">
            <span className="text-xs font-semibold text-muted-foreground flex items-center gap-1">
              <Server className="h-3.5 w-3.5" />
              {t("migration.remoteConnectionTitle", "Remote SSH Connection")}
            </span>

            <div className="grid grid-cols-3 gap-3">
              <div className="col-span-2">
                <label className="text-xs font-semibold block mb-1">
                  {t("migration.hostLabel", "Remote IP / Hostname")}
                </label>
                <Input
                  value={remoteHost}
                  onChange={(e) => setRemoteHost(e.target.value)}
                  placeholder="192.168.1.100"
                  required
                  data-testid="input-migration-host"
                />
              </div>

              <div>
                <label className="text-xs font-semibold block mb-1">
                  {t("migration.portLabel", "SSH Port")}
                </label>
                <Input
                  type="number"
                  value={remotePort}
                  onChange={(e) => setRemotePort(Number(e.target.value))}
                  required
                  data-testid="input-migration-port"
                />
              </div>
            </div>

            <div className="grid grid-cols-2 gap-3">
              <div>
                <label className="text-xs font-semibold block mb-1">
                  {t("migration.sshUserLabel", "SSH Username")}
                </label>
                <Input
                  value={remoteUser}
                  onChange={(e) => setRemoteUser(e.target.value)}
                  placeholder="root"
                  required
                  data-testid="input-migration-ssh-user"
                />
              </div>

              <div>
                <label className="text-xs font-semibold block mb-1">
                  {t("migration.authMethodLabel", "Auth Method")}
                </label>
                <select
                  value={authMethod}
                  onChange={(e) => setAuthMethod(e.target.value as any)}
                  className="w-full border rounded-md px-3 py-2 text-xs bg-background h-9"
                  data-testid="select-migration-auth"
                >
                  <option value="password">Password</option>
                  <option value="ssh_key">SSH Private Key</option>
                </select>
              </div>
            </div>

            {authMethod === "password" ? (
              <div>
                <label className="text-xs font-semibold block mb-1">
                  {t("migration.passwordLabel", "Remote Root / User Password")}
                </label>
                <Input
                  type="password"
                  value={password}
                  onChange={(e) => setPassword(e.target.value)}
                  placeholder="••••••••••••"
                  required
                  data-testid="input-migration-password"
                />
                <p className="text-[11px] text-muted-foreground mt-1 flex items-center gap-1">
                  <Shield className="h-3 w-3 text-emerald-500" />
                  {t("migration.credentialNotice", "Credentials are used in memory only and never stored in the database.")}
                </p>
              </div>
            ) : (
              <div>
                <label className="text-xs font-semibold block mb-1">
                  {t("migration.sshKeyLabel", "SSH Private Key")}
                </label>
                <textarea
                  value={sshKey}
                  onChange={(e) => setSshKey(e.target.value)}
                  rows={3}
                  className="w-full font-mono text-xs p-2 border rounded-md bg-background"
                  placeholder="-----BEGIN OPENSSH PRIVATE KEY-----"
                  required
                  data-testid="input-migration-ssh-key"
                />
              </div>
            )}
          </div>

          {/* 3. Destination Tenant Mapping */}
          <div className="space-y-3 pt-1 border-t">
            <span className="text-xs font-semibold text-muted-foreground">
              {t("migration.targetMappingTitle", "Destination Tenant Mapping")}
            </span>

            <div className="grid grid-cols-2 gap-3">
              <div>
                <label className="text-xs font-semibold block mb-1">
                  {t("migration.targetDomainLabel", "Primary Domain")}
                </label>
                <Input
                  value={targetDomain}
                  onChange={(e) => setTargetDomain(e.target.value)}
                  placeholder="myclientdomain.com"
                  required
                  data-testid="input-migration-target-domain"
                />
              </div>

              <div>
                <label className="text-xs font-semibold block mb-1">
                  {t("migration.tenantUserLabel", "Tenant Username")}
                </label>
                <Input
                  value={tenantUser}
                  onChange={(e) => setTenantUser(e.target.value)}
                  placeholder="myclient"
                  required
                  data-testid="input-migration-tenant-user"
                />
              </div>
            </div>

            <div className="flex items-center gap-2 pt-1">
              <input
                type="checkbox"
                id="dryrun-checkbox"
                checked={dryRun}
                onChange={(e) => setDryRun(e.target.checked)}
                className="rounded border-gray-300 text-primary focus:ring-primary h-4 w-4"
                data-testid="checkbox-migration-dryrun"
              />
              <label htmlFor="dryrun-checkbox" className="text-xs font-medium cursor-pointer">
                {t("migration.dryRunLabel", "Dry Run Only (Validate backup and conflict check without restoring)")}
              </label>
            </div>
          </div>

          <DialogFooter className="pt-2">
            <Button type="button" variant="outline" onClick={onClose} disabled={isSubmitting}>
              {t("common.cancel", "Cancel")}
            </Button>
            <Button type="submit" disabled={isSubmitting} data-testid="btn-start-transfer">
              {isSubmitting ? t("common.initiating", "Connecting...") : t("migration.startTransferBtn", "Start Migration")}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
