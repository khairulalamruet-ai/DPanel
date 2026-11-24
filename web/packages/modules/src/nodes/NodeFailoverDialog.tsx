import React, { useState } from "react";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogFooter,
  DialogTitle,
  DialogDescription,
  Button,
  Badge,
  useToast,
} from "@builtpanel/ui";
import { useTranslation } from "@builtpanel/i18n";
import { apiFetch, getCsrfToken } from "@builtpanel/api";
import { AlertTriangle, ArrowRight, Server, ShieldCheck, RefreshCw } from "lucide-react";
import type { NodeItem } from "./types";

export interface NodeFailoverDialogProps {
  node: NodeItem | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  availableTargetNodes: NodeItem[];
  onFailoverComplete?: () => void;
}

export function NodeFailoverDialog({
  node,
  open,
  onOpenChange,
  availableTargetNodes,
  onFailoverComplete,
}: NodeFailoverDialogProps) {
  const { t } = useTranslation();
  const { toast } = useToast();

  const healthyTargets = availableTargetNodes.filter(
    (n) => n.id !== node?.id && n.status === "online"
  );

  const [selectedTarget, setSelectedTarget] = useState<string>("");
  const targetNodeId = selectedTarget || healthyTargets[0]?.id || "";
  const [isSubmitting, setIsSubmitting] = useState<boolean>(false);
  const [error, setError] = useState<string | null>(null);

  if (!node) return null;

  const handleFailover = async () => {
    if (!targetNodeId) {
      setError("Please select a target node");
      return;
    }

    try {
      setIsSubmitting(true);
      setError(null);

      await apiFetch<{ success: boolean; message: string; migrated_tenants: number }>(
        `/api/v1/admin/cluster/nodes/${node.id}/failover`,
        {
          method: "POST",
          headers: {
            "Content-Type": "application/json",
            "X-CSRF-Token": getCsrfToken() || "",
          },
          body: JSON.stringify({
            target_node_id: targetNodeId,
          }),
        }
      );

      toast({
        title: t("nodes.health.failoverSuccess"),
        variant: "default",
      });

      if (onFailoverComplete) {
        onFailoverComplete();
      }
      onOpenChange(false);
    } catch (err: any) {
      setError(err?.message || "Failed to execute failover");
    } finally {
      setIsSubmitting(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent onClose={() => onOpenChange(false)} className="max-w-lg">
        <DialogHeader>
          <div className="flex items-center gap-2 text-destructive mb-1">
            <AlertTriangle className="h-5 w-5" />
            <DialogTitle>{t("nodes.health.failoverModalTitle")}</DialogTitle>
          </div>
          <DialogDescription>
            {t("nodes.health.failoverModalDesc")}
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-4 my-3">
          {error && (
            <div className="p-2.5 rounded-md bg-destructive/10 text-destructive text-xs">
              {error}
            </div>
          )}

          {/* Node Migration Visualizer */}
          <div className="grid grid-cols-1 sm:grid-cols-2 gap-3 p-3 bg-muted/30 rounded-lg border">
            {/* Source Node */}
            <div className="space-y-1">
              <span className="text-[11px] font-semibold uppercase text-muted-foreground block">
                {t("nodes.health.sourceNode")}
              </span>
              <div className="flex items-center gap-2">
                <Server className="h-4 w-4 text-destructive" />
                <span className="text-xs font-bold font-mono text-foreground truncate">
                  {node.name}
                </span>
              </div>
              <Badge variant="destructive" className="text-[10px] mt-1">
                {node.status}
              </Badge>
            </div>

            {/* Target Node Selection */}
            <div className="space-y-1">
              <span className="text-[11px] font-semibold uppercase text-muted-foreground block">
                {t("nodes.health.targetNode")}
              </span>
              <select
                data-testid="select-failover-target-node"
                value={targetNodeId}
                onChange={(e) => setSelectedTarget(e.target.value)}
                disabled={isSubmitting}
                className="flex h-8 w-full rounded-md border border-input bg-background px-2 py-1 text-xs shadow-sm text-foreground"
              >
                {healthyTargets.map((target) => (
                  <option key={target.id} value={target.id}>
                    {target.name} ({target.ip_address})
                  </option>
                ))}
              </select>
              <Badge variant="outline" className="text-[10px] text-emerald-600 border-emerald-500/30 mt-1 flex items-center gap-1 w-fit">
                <ShieldCheck className="h-3 w-3" />
                Online & Ready
              </Badge>
            </div>
          </div>
        </div>

        <DialogFooter className="gap-2">
          <Button
            type="button"
            variant="outline"
            disabled={isSubmitting}
            onClick={() => onOpenChange(false)}
          >
            {t("common.cancel")}
          </Button>
          <Button
            type="button"
            variant="destructive"
            data-testid="btn-confirm-failover"
            disabled={isSubmitting || !targetNodeId}
            onClick={handleFailover}
            className="gap-1.5"
          >
            {isSubmitting ? (
              <RefreshCw className="h-4 w-4 animate-spin" />
            ) : (
              <ArrowRight className="h-4 w-4" />
            )}
            {t("nodes.health.executeFailover")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
