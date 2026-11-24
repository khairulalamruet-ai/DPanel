import React, { useState } from "react";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogFooter,
  DialogTitle,
  DialogDescription,
  Button,
  Input,
} from "@builtpanel/ui";
import { useTranslation } from "@builtpanel/i18n";
import { Server, Save, Network, Shield } from "lucide-react";
import { CreateDnsServerPayload, DnsServerRole, DnsSyncStrategy } from "./types";

export interface AddDnsServerDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onSave: (payload: CreateDnsServerPayload) => Promise<void>;
  isSaving?: boolean;
}

export function AddDnsServerDialog({
  open,
  onOpenChange,
  onSave,
  isSaving = false,
}: AddDnsServerDialogProps) {
  const { t } = useTranslation();

  const [name, setName] = useState("");
  const [hostname, setHostname] = useState("");
  const [ipAddress, setIpAddress] = useState("");
  const [port, setPort] = useState("53");
  const [apiUrl, setApiUrl] = useState("");
  const [apiKey, setApiKey] = useState("");
  const [role, setRole] = useState<DnsServerRole>("secondary");
  const [syncStrategy, setSyncStrategy] = useState<DnsSyncStrategy>("native_db");
  const [error, setError] = useState<string | null>(null);

  const handleReset = () => {
    setName("");
    setHostname("");
    setIpAddress("");
    setPort("53");
    setApiUrl("");
    setApiKey("");
    setRole("secondary");
    setSyncStrategy("native_db");
    setError(null);
  };

  const handleClose = () => {
    handleReset();
    onOpenChange(false);
  };

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!name.trim()) {
      setError(t("dns.cluster.dialog.nameRequired"));
      return;
    }
    if (!hostname.trim()) {
      setError(t("dns.cluster.dialog.hostnameRequired"));
      return;
    }
    if (!ipAddress.trim()) {
      setError(t("dns.cluster.dialog.ipRequired"));
      return;
    }

    try {
      await onSave({
        name: name.trim(),
        hostname: hostname.trim().toLowerCase(),
        ip_address: ipAddress.trim(),
        port: parseInt(port, 10) || 53,
        api_url: apiUrl.trim() || `http://${ipAddress.trim()}:8081`,
        api_key: apiKey.trim() || undefined,
        role,
        sync_strategy: syncStrategy,
      });
      handleClose();
    } catch (err: any) {
      setError(err?.message || "Failed to add DNS server");
    }
  };

  return (
    <Dialog open={open} onOpenChange={handleClose}>
      <DialogContent onClose={handleClose} className="max-w-xl p-6">
        <DialogHeader>
          <div className="flex items-center gap-2 text-primary mb-1">
            <Server className="h-5 w-5" />
            <DialogTitle>{t("dns.cluster.dialog.createTitle")}</DialogTitle>
          </div>
          <DialogDescription>{t("dns.cluster.dialog.createDesc")}</DialogDescription>
        </DialogHeader>

        <form onSubmit={handleSubmit} className="space-y-4 my-3">
          {error && (
            <div className="p-2.5 rounded-md bg-destructive/10 text-destructive text-xs">
              {error}
            </div>
          )}

          <div className="grid grid-cols-1 md:grid-cols-2 gap-3">
            <div className="space-y-1">
              <label htmlFor="dns-name" className="text-xs font-medium text-foreground flex items-center gap-1">
                <span>{t("dns.cluster.dialog.nameLabel")}</span>
                <span className="text-destructive font-bold">*</span>
              </label>
              <Input
                id="dns-name"
                data-testid="input-dns-name"
                value={name}
                onChange={(e) => {
                  setName(e.target.value);
                  if (error) setError(null);
                }}
                placeholder="e.g. US Secondary PowerDNS"
                disabled={isSaving}
                autoFocus
              />
            </div>

            <div className="space-y-1">
              <label htmlFor="dns-hostname" className="text-xs font-medium text-foreground flex items-center gap-1">
                <span>{t("dns.cluster.dialog.hostnameLabel")}</span>
                <span className="text-destructive font-bold">*</span>
              </label>
              <Input
                id="dns-hostname"
                data-testid="input-dns-hostname"
                value={hostname}
                onChange={(e) => {
                  setHostname(e.target.value);
                  if (error) setError(null);
                }}
                placeholder="e.g. ns2.builtpanel.net"
                disabled={isSaving}
              />
            </div>
          </div>

          <div className="grid grid-cols-1 md:grid-cols-2 gap-3">
            <div className="space-y-1">
              <label htmlFor="dns-ip" className="text-xs font-medium text-foreground flex items-center gap-1">
                <span>{t("dns.cluster.dialog.ipLabel")}</span>
                <span className="text-destructive font-bold">*</span>
              </label>
              <Input
                id="dns-ip"
                data-testid="input-dns-ip"
                value={ipAddress}
                onChange={(e) => {
                  setIpAddress(e.target.value);
                  if (error) setError(null);
                }}
                placeholder="198.51.100.25"
                disabled={isSaving}
              />
            </div>

            <div className="space-y-1">
              <label htmlFor="dns-port" className="text-xs font-medium text-foreground">
                {t("dns.cluster.dialog.portLabel")}
              </label>
              <Input
                id="dns-port"
                type="number"
                value={port}
                onChange={(e) => setPort(e.target.value)}
                placeholder="53"
                disabled={isSaving}
              />
            </div>
          </div>

          <div className="grid grid-cols-1 md:grid-cols-2 gap-3">
            <div className="space-y-1">
              <label htmlFor="dns-role" className="text-xs font-medium text-foreground">
                {t("dns.cluster.dialog.roleLabel")}
              </label>
              <select
                id="dns-role"
                data-testid="select-dns-role"
                value={role}
                onChange={(e) => setRole(e.target.value as DnsServerRole)}
                disabled={isSaving}
                className="flex h-9 w-full rounded-md border border-input bg-background px-3 py-1 text-xs shadow-sm transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring text-foreground"
              >
                <option value="secondary">{t("dns.cluster.role.secondary")}</option>
                <option value="replica">{t("dns.cluster.role.replica")}</option>
                <option value="primary">{t("dns.cluster.role.primary")}</option>
              </select>
            </div>

            <div className="space-y-1">
              <label htmlFor="dns-sync" className="text-xs font-medium text-foreground">
                {t("dns.cluster.dialog.syncStrategyLabel")}
              </label>
              <select
                id="dns-sync"
                data-testid="select-dns-sync"
                value={syncStrategy}
                onChange={(e) => setSyncStrategy(e.target.value as DnsSyncStrategy)}
                disabled={isSaving}
                className="flex h-9 w-full rounded-md border border-input bg-background px-3 py-1 text-xs shadow-sm transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring text-foreground"
              >
                <option value="native_db">{t("dns.cluster.strategy.nativeDb")}</option>
                <option value="axfr">{t("dns.cluster.strategy.axfr")}</option>
                <option value="api_push">{t("dns.cluster.strategy.apiPush")}</option>
              </select>
            </div>
          </div>

          <div className="grid grid-cols-1 md:grid-cols-2 gap-3">
            <div className="space-y-1">
              <label htmlFor="dns-api-url" className="text-xs font-medium text-foreground">
                {t("dns.cluster.dialog.apiUrlLabel")}
              </label>
              <Input
                id="dns-api-url"
                value={apiUrl}
                onChange={(e) => setApiUrl(e.target.value)}
                placeholder="http://198.51.100.25:8081"
                disabled={isSaving}
              />
            </div>

            <div className="space-y-1">
              <label htmlFor="dns-api-key" className="text-xs font-medium text-foreground">
                {t("dns.cluster.dialog.apiKeyLabel")}
              </label>
              <Input
                id="dns-api-key"
                type="password"
                value={apiKey}
                onChange={(e) => setApiKey(e.target.value)}
                placeholder="••••••••••••"
                disabled={isSaving}
              />
            </div>
          </div>

          <DialogFooter className="pt-4 border-t gap-2">
            <Button
              type="button"
              variant="outline"
              disabled={isSaving}
              onClick={handleClose}
            >
              {t("common.cancel")}
            </Button>
            <Button
              type="submit"
              disabled={isSaving}
              data-testid="btn-save-dns-server"
              className="gap-2"
            >
              <Save className="h-4 w-4" />
              {isSaving ? t("common.processing") : t("dns.cluster.dialog.saveBtn")}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
