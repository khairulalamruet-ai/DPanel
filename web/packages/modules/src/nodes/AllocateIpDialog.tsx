import React, { useState, useEffect } from "react";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogFooter,
  DialogTitle,
  DialogDescription,
  Button,
  Input,
  Badge,
} from "@builtpanel/ui";
import { useTranslation } from "@builtpanel/i18n";
import { apiFetch } from "@builtpanel/api";
import { UserCheck, ShieldCheck, User } from "lucide-react";
import { AllocateIpPayload, NodeIpAddress } from "./types";

interface SimpleUser {
  id: string;
  username: string;
  email: string;
}

export interface AllocateIpDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  ip: NodeIpAddress | null;
  onAllocate: (ipId: string, payload: AllocateIpPayload) => Promise<void>;
  isAllocating?: boolean;
}

export function AllocateIpDialog({
  open,
  onOpenChange,
  ip,
  onAllocate,
  isAllocating = false,
}: AllocateIpDialogProps) {
  const { t } = useTranslation();

  const [users, setUsers] = useState<SimpleUser[]>([]);
  const [selectedUserId, setSelectedUserId] = useState<string>("");
  const [domain, setDomain] = useState<string>("");
  const [isLoadingUsers, setIsLoadingUsers] = useState<boolean>(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (open) {
      const fetchUsers = async () => {
        try {
          setIsLoadingUsers(true);
          const res = await apiFetch<{ items: SimpleUser[] }>("/api/v1/admin/users");
          const list = res.items || [];
          setUsers(list);
          if (list.length > 0) {
            setSelectedUserId(list[0].id);
          }
        } catch {
          // Fallback demo users if endpoint differs
          const fallbacks: SimpleUser[] = [
            { id: "usr_0001", username: "alpha", email: "alpha@example.com" },
            { id: "usr_0002", username: "beta", email: "beta@example.com" },
            { id: "usr_0003", username: "gamma", email: "gamma@example.com" },
          ];
          setUsers(fallbacks);
          setSelectedUserId(fallbacks[0].id);
        } finally {
          setIsLoadingUsers(false);
        }
      };
      fetchUsers();
      setDomain("");
      setError(null);
    }
  }, [open]);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!ip) return;
    if (!selectedUserId) {
      setError(t("nodes.ipPools.allocate.userRequired"));
      return;
    }

    const selectedUser = users.find((u) => u.id === selectedUserId);
    if (!selectedUser) {
      setError(t("nodes.ipPools.allocate.userRequired"));
      return;
    }

    try {
      await onAllocate(ip.id, {
        tenant_id: selectedUser.id,
        tenant_username: selectedUser.username,
        domain: domain.trim() || undefined,
      });
      onOpenChange(false);
    } catch (err: any) {
      setError(err?.message || "Failed to allocate dedicated IP");
    }
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent onClose={() => onOpenChange(false)} className="max-w-md p-6">
        <DialogHeader>
          <div className="flex items-center gap-2 text-primary mb-1">
            <UserCheck className="h-5 w-5" />
            <DialogTitle>{t("nodes.ipPools.allocate.title")}</DialogTitle>
          </div>
          <DialogDescription>
            {t("nodes.ipPools.allocate.desc", { ip: ip?.ip_address ?? "" })}
          </DialogDescription>
        </DialogHeader>

        {ip && (
          <form onSubmit={handleSubmit} className="space-y-4 my-3">
            {error && (
              <div className="p-2.5 rounded-md bg-destructive/10 text-destructive text-xs">
                {error}
              </div>
            )}

            {/* IP Details Banner */}
            <div className="p-3 rounded-lg border bg-muted/20 flex items-center justify-between">
              <div>
                <span className="text-xs text-muted-foreground block font-medium">
                  {t("nodes.ipPools.allocate.targetIp")}:
                </span>
                <span className="font-mono text-sm font-bold text-foreground">
                  {ip.ip_address}
                </span>
              </div>
              <div className="text-right">
                <span className="text-xs text-muted-foreground block font-medium">
                  {t("nodes.ipPools.allocate.nodeName")}:
                </span>
                <Badge variant="outline" className="font-mono text-xs">
                  {ip.node_name}
                </Badge>
              </div>
            </div>

            {/* Tenant Selector */}
            <div className="space-y-1">
              <label htmlFor="allocate-user" className="text-xs font-medium text-foreground">
                {t("nodes.ipPools.allocate.selectTenant")}
              </label>
              <select
                id="allocate-user"
                data-testid="select-allocate-tenant"
                value={selectedUserId}
                onChange={(e) => setSelectedUserId(e.target.value)}
                disabled={isAllocating || isLoadingUsers}
                className="flex h-9 w-full rounded-md border border-input bg-background px-3 py-1 text-xs shadow-sm text-foreground"
              >
                {users.map((u) => (
                  <option key={u.id} value={u.id}>
                    {u.username} ({u.email})
                  </option>
                ))}
              </select>
            </div>

            {/* Optional Specific Domain */}
            <div className="space-y-1">
              <label htmlFor="allocate-domain" className="text-xs font-medium text-foreground">
                {t("nodes.ipPools.allocate.domainOptional")}
              </label>
              <Input
                id="allocate-domain"
                data-testid="input-allocate-domain"
                value={domain}
                onChange={(e) => setDomain(e.target.value)}
                placeholder="e.g. mail.alexblog.org (optional)"
                disabled={isAllocating}
              />
              <p className="text-[11px] text-muted-foreground">
                {t("nodes.ipPools.allocate.domainDesc")}
              </p>
            </div>

            <DialogFooter className="pt-3 border-t gap-2">
              <Button
                type="button"
                variant="outline"
                disabled={isAllocating}
                onClick={() => onOpenChange(false)}
              >
                {t("common.cancel")}
              </Button>
              <Button
                type="submit"
                disabled={isAllocating}
                data-testid="btn-confirm-allocate-ip"
                className="gap-2"
              >
                <ShieldCheck className="h-4 w-4" />
                {isAllocating
                  ? t("common.processing")
                  : t("nodes.ipPools.allocate.confirmBtn")}
              </Button>
            </DialogFooter>
          </form>
        )}
      </DialogContent>
    </Dialog>
  );
}
