import React, { useState } from "react";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
  Button,
  useToast,
} from "@builtpanel/ui";
import { setRemoteHosts } from "./api";
import { Network, Plus, Trash2, ShieldCheck, AlertCircle } from "lucide-react";
import type { DatabaseUserItem } from "./types";
import { useTranslation } from "@builtpanel/i18n";

interface RemoteAccessDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** The addresses are set per database user. */
  users: DatabaseUserItem[];
  onHostsUpdated: (newHosts: string[]) => void;
}

export function RemoteAccessDialog({
  open,
  onOpenChange,
  users,
  onHostsUpdated,
}: RemoteAccessDialogProps) {
  const { t } = useTranslation();
  const { toast } = useToast();

  const [userId, setUserId] = useState<string>(users[0]?.id ?? "");
  const selected = users.find((u) => u.id === userId) ?? users[0];
  const [hosts, setHosts] = useState<string[]>(selected?.remote_hosts ?? []);
  const [newHostInput, setNewHostInput] = useState("");
  const [isSubmitting, setIsSubmitting] = useState(false);
  const [inputError, setInputError] = useState<string | null>(null);

  React.useEffect(() => {
    if (!users.some((u) => u.id === userId)) setUserId(users[0]?.id ?? "");
  }, [users, userId]);

  React.useEffect(() => {
    setHosts(selected?.remote_hosts ?? []);
  }, [selected]);

  const handleAddHost = () => {
    const val = newHostInput.trim();
    if (!val) return;

    // Simple IP / CIDR validation
    const ipCidrRegex = /^(\d{1,3}\.){3}\d{1,3}(\/\d{1,2})?$/;
    if (!ipCidrRegex.test(val)) {
      setInputError(t("databases.remote.invalidIp", "Enter a valid IPv4 address or CIDR (e.g. 192.168.1.100 or 10.5.0.0/16)"));
      return;
    }

    if (hosts.includes(val)) {
      setInputError(t("databases.remote.duplicateIp", "This IP address or CIDR is already in the allowlist"));
      return;
    }

    setHosts([...hosts, val]);
    setNewHostInput("");
    setInputError(null);
  };

  const handleRemoveHost = (targetHost: string) => {
    setHosts(hosts.filter((h) => h !== targetHost));
  };

  const handleSave = async () => {
    try {
      setIsSubmitting(true);
      if (!selected) return;
      await setRemoteHosts(selected.id, hosts);

      toast({
        title: t("common.success"),
        description: t("databases.toast.remoteHostsUpdated", "Remote database access allowlist updated"),
        variant: "default",
      });

      onHostsUpdated(hosts);
      onOpenChange(false);
    } catch (err: any) {
      toast({
        title: t("common.error"),
        description: err?.message || t("databases.errors.remoteUpdateFailed", "Failed to update remote database hosts"),
        variant: "destructive",
      });
    } finally {
      setIsSubmitting(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-md" onClose={() => onOpenChange(false)}>
        <DialogHeader>
          <div className="flex items-center gap-3">
            <div className="rounded-full bg-primary/10 p-2.5 text-primary">
              <Network className="h-5 w-5" />
            </div>
            <div>
              <DialogTitle>{t("databases.remote.dialogTitle", "Remote Database Access (CIDR)")}</DialogTitle>
              <DialogDescription className="mt-0.5 text-xs">
                {t(
                  "databases.remote.desc",
                  "Configure external client IP addresses or CIDR blocks authorized to connect directly to your SQL databases."
                )}
              </DialogDescription>
            </div>
          </div>
        </DialogHeader>

        <div className="mt-4 space-y-4">
          {/* The database user the addresses belong to */}
          <div className="space-y-1.5">
            <label className="text-xs font-medium text-foreground">
              {t("databases.remote.user", "Database user")}
            </label>
            <select
              value={selected?.id ?? ""}
              onChange={(e) => setUserId(e.target.value)}
              className="w-full h-9 rounded-md border border-input bg-background px-2.5 text-xs font-mono"
              data-testid="select-remote-user"
            >
              {users.map((u) => (
                <option key={u.id} value={u.id}>
                  {u.username}
                </option>
              ))}
            </select>
          </div>

          {/* Add input */}
          <div className="space-y-1.5">
            <label className="text-xs font-medium text-foreground">
              {t("databases.remote.addLabel", "Add Authorized IP / CIDR")}
            </label>
            <div className="flex items-center gap-2">
              <input
                type="text"
                value={newHostInput}
                onChange={(e) => {
                  setNewHostInput(e.target.value);
                  setInputError(null);
                }}
                onKeyDown={(e) => {
                  if (e.key === "Enter") {
                    e.preventDefault();
                    handleAddHost();
                  }
                }}
                placeholder="203.0.113.50 or 198.51.100.0/24"
                className="flex-1 h-9 rounded-md border border-input bg-background px-3 text-xs font-mono text-foreground focus:outline-none focus:ring-1 focus:ring-ring"
                data-testid="input-remote-ip"
              />
              <Button
                type="button"
                size="sm"
                variant="secondary"
                onClick={handleAddHost}
                data-testid="btn-add-remote-ip"
              >
                <Plus className="h-3.5 w-3.5 mr-1" />
                {t("common.add")}
              </Button>
            </div>
            {inputError && (
              <p className="text-[11px] text-destructive flex items-center gap-1 mt-1">
                <AlertCircle className="h-3 w-3 shrink-0" />
                <span>{inputError}</span>
              </p>
            )}
          </div>

          {/* List of allowed IPs */}
          <div className="space-y-1.5">
            <label className="text-xs font-medium text-foreground">
              {t("databases.remote.currentAllowlist", "Current Allowed Remote Hosts")}
            </label>
            <div className="rounded-md border border-border bg-muted/20 max-h-48 overflow-y-auto divide-y divide-border">
              {hosts.length === 0 ? (
                <div className="p-4 text-center text-xs text-muted-foreground">
                  {t("databases.remote.empty", "No external IPs configured. Database is accessible only within the internal tenant container network.")}
                </div>
              ) : (
                hosts.map((host) => (
                  <div
                    key={host}
                    className="flex items-center justify-between p-2.5 text-xs font-mono"
                    data-testid={`remote-host-item-${host.replace(/[/.]/g, "-")}`}
                  >
                    <div className="flex items-center gap-2">
                      <ShieldCheck className="h-3.5 w-3.5 text-emerald-500" />
                      <span>{host}</span>
                    </div>
                    <Button
                      variant="ghost"
                      size="sm"
                      onClick={() => handleRemoveHost(host)}
                      className="h-7 w-7 p-0 text-destructive hover:bg-destructive/10"
                      title={t("common.delete")}
                      data-testid={`btn-remove-remote-host-${host.replace(/[/.]/g, "-")}`}
                    >
                      <Trash2 className="h-3.5 w-3.5" />
                    </Button>
                  </div>
                ))
              )}
            </div>
          </div>
        </div>

        <DialogFooter className="mt-6 border-t border-border pt-3">
          <Button
            type="button"
            variant="outline"
            size="sm"
            onClick={() => onOpenChange(false)}
            disabled={isSubmitting}
          >
            {t("common.cancel")}
          </Button>
          <Button
            size="sm"
            onClick={handleSave}
            disabled={isSubmitting || !selected}
            data-testid="btn-save-remote-hosts"
          >
            {isSubmitting ? t("common.saving") : t("common.save", "Save Allowed Hosts")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
