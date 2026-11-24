import React, { useState } from "react";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
  Button,
  Badge,
  useToast,
} from "@builtpanel/ui";
import { useTranslation } from "@builtpanel/i18n";
import { apiFetch, getCsrfToken } from "@builtpanel/api";
import { Cpu, Server, Sparkles, CheckCircle2, XCircle, ArrowRight, ShieldCheck, AlertTriangle } from "lucide-react";
import type { NodeItem } from "./types";
import { selectPlacementNode, type PlacementPolicy } from "./placement";
import type { NodeOption } from "../user-wizard/types";

interface SchedulerConfigDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  nodes: NodeItem[];
  onPolicyUpdated?: () => void;
}

export function SchedulerConfigDialog({
  open,
  onOpenChange,
  nodes,
  onPolicyUpdated,
}: SchedulerConfigDialogProps) {
  const { t } = useTranslation();
  const { toast } = useToast();

  const [policy, setPolicy] = useState<PlacementPolicy>("least-loaded");
  const [isSubmitting, setIsSubmitting] = useState(false);

  // Convert NodeItem to NodeOption for placement simulation
  const nodeOptions: NodeOption[] = nodes.map((n) => ({
    id: n.id,
    name: n.name,
    region: n.hostname,
    cpu_usage_pct: n.capabilities?.cpu_usage_pct ?? 0,
    memory_usage_pct: n.capabilities?.ram_total_bytes
      ? Math.round((n.capabilities.ram_used_bytes / n.capabilities.ram_total_bytes) * 100)
      : 0,
    status: (n.status === "online" ? "healthy" : n.status === "draining" ? "draining" : "offline") as NodeOption["status"],
  }));

  const simulation = selectPlacementNode(nodeOptions, policy);

  const handleSavePolicy = async () => {
    try {
      setIsSubmitting(true);
      // TODO(api:T16.02): cluster placement scheduler configuration endpoint
      await apiFetch("/api/v1/admin/cluster/scheduler", {
        method: "PUT",
        headers: {
          "Content-Type": "application/json",
          "X-CSRF-Token": getCsrfToken() || "",
        },
        body: JSON.stringify({
          policy,
          exclude_draining: true,
          fallback_policy: "round-robin",
        }),
      });

      toast({
        title: t("nodes.scheduler.saveSuccess"),
        variant: "default",
      });
      onOpenChange(false);
      onPolicyUpdated?.();
    } catch {
      toast({
        title: t("common.error"),
        variant: "destructive",
      });
    } finally {
      setIsSubmitting(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-2xl max-h-[85vh] overflow-y-auto">
        <DialogHeader>
          <div className="flex items-center gap-2">
            <Cpu className="h-5 w-5 text-primary" />
            <DialogTitle>{t("nodes.scheduler.title")}</DialogTitle>
          </div>
          <DialogDescription className="text-xs">
            {t("nodes.scheduler.desc")}
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-5 py-3 text-xs">
          {/* Policy Selection */}
          <div className="space-y-2">
            <label className="font-semibold text-foreground flex items-center gap-1.5">
              <Sparkles className="h-4 w-4 text-primary" />
              {t("nodes.scheduler.policyLabel")}
            </label>
            <div className="grid grid-cols-1 sm:grid-cols-2 gap-2.5">
              {/* Least-Loaded */}
              <div
                onClick={() => setPolicy("least-loaded")}
                className={`p-3 rounded-lg border cursor-pointer transition-all ${
                  policy === "least-loaded"
                    ? "border-primary bg-primary/5 ring-1 ring-primary/30"
                    : "border-border bg-card hover:bg-muted/30"
                }`}
              >
                <div className="flex items-center justify-between">
                  <span className="font-semibold text-foreground">
                    {t("nodes.scheduler.leastLoadedTitle")}
                  </span>
                  <Badge variant="default" className="text-[10px]">
                    Recommended
                  </Badge>
                </div>
                <p className="text-[11px] text-muted-foreground mt-1">
                  {t("nodes.scheduler.leastLoadedDesc")}
                </p>
              </div>

              {/* Round-Robin */}
              <div
                onClick={() => setPolicy("round-robin")}
                className={`p-3 rounded-lg border cursor-pointer transition-all ${
                  policy === "round-robin"
                    ? "border-primary bg-primary/5 ring-1 ring-primary/30"
                    : "border-border bg-card hover:bg-muted/30"
                }`}
              >
                <span className="font-semibold text-foreground">
                  {t("nodes.scheduler.roundRobinTitle")}
                </span>
                <p className="text-[11px] text-muted-foreground mt-1">
                  {t("nodes.scheduler.roundRobinDesc")}
                </p>
              </div>
            </div>
          </div>

          {/* Drainage Invariant Notice (DoD T16.02) */}
          <div className="p-3 rounded-lg border border-emerald-500/30 bg-emerald-500/5 space-y-1">
            <div className="flex items-center gap-1.5 text-emerald-600 dark:text-emerald-400 font-semibold text-xs">
              <ShieldCheck className="h-4 w-4" />
              {t("nodes.scheduler.drainInvariantTitle")}
            </div>
            <p className="text-[11px] text-muted-foreground">
              {t("nodes.scheduler.drainInvariantDesc")}
            </p>
          </div>

          {/* Live Placement Simulation (DoD T16.02) */}
          <div className="p-4 rounded-lg border bg-muted/20 space-y-3" data-testid="placement-simulation-card">
            <div className="flex items-center justify-between">
              <div className="flex items-center gap-1.5 font-semibold text-foreground">
                <Server className="h-4 w-4 text-primary" />
                <span>{t("nodes.scheduler.simulationTitle")}</span>
              </div>
              <Badge variant="outline" className="text-[10px] font-mono">
                {policy}
              </Badge>
            </div>

            {simulation.selectedNode ? (
              <div className="p-3 rounded border bg-background flex flex-col sm:flex-row sm:items-center justify-between gap-2">
                <div>
                  <span className="text-[11px] text-muted-foreground block">
                    {t("nodes.scheduler.nextTarget")}:
                  </span>
                  <span className="font-mono font-bold text-foreground text-sm flex items-center gap-1.5">
                    <CheckCircle2 className="h-4 w-4 text-emerald-500" />
                    {simulation.selectedNode.name}
                  </span>
                </div>
                <div className="text-right text-[11px] text-muted-foreground">
                  <p>{simulation.reason}</p>
                </div>
              </div>
            ) : (
              <div className="p-3 rounded border bg-destructive/10 text-destructive text-xs">
                {simulation.reason}
              </div>
            )}

            {/* Excluded Draining Nodes breakdown */}
            {simulation.excludedNodes.length > 0 && (
              <div className="space-y-1.5 pt-1">
                <span className="text-[11px] font-medium text-muted-foreground flex items-center gap-1">
                  <AlertTriangle className="h-3.5 w-3.5 text-amber-500" />
                  {t("nodes.scheduler.excludedNodesTitle")} ({simulation.excludedNodes.length}):
                </span>
                <div className="flex flex-wrap gap-1.5">
                  {simulation.excludedNodes.map((n) => (
                    <Badge
                      key={n.id}
                      variant="outline"
                      className="text-[10px] text-amber-500 border-amber-500 gap-1 font-mono"
                    >
                      <XCircle className="h-3 w-3" />
                      {n.name} ({n.status})
                    </Badge>
                  ))}
                </div>
              </div>
            )}
          </div>
        </div>

        <DialogFooter className="gap-2">
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
            type="button"
            size="sm"
            onClick={handleSavePolicy}
            disabled={isSubmitting}
            className="gap-1.5"
            data-testid="save-scheduler-policy-btn"
          >
            {isSubmitting ? t("common.loading") : t("common.save")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
