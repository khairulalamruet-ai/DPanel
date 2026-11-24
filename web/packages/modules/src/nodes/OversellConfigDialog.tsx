import React, { useState, useEffect } from "react";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
  Button,
  Input,
  useToast,
} from "@builtpanel/ui";
import { useTranslation } from "@builtpanel/i18n";
import { apiFetch, getCsrfToken } from "@builtpanel/api";
import { Cpu, HardDrive, Layers, Server, Activity } from "lucide-react";
import type { NodeItem, NodeOversellConfig } from "./types";

interface OversellConfigDialogProps {
  node: NodeItem | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onSaved: () => void;
}

export function OversellConfigDialog({
  node,
  open,
  onOpenChange,
  onSaved,
}: OversellConfigDialogProps) {
  const { t } = useTranslation();
  const { toast } = useToast();

  const [cpuRatio, setCpuRatio] = useState<number>(2.0);
  const [ramRatio, setRamRatio] = useState<number>(1.25);
  const [diskRatio, setDiskRatio] = useState<number>(1.0);
  const [maxTenants, setMaxTenants] = useState<number>(50);
  const [isSubmitting, setIsSubmitting] = useState(false);

  useEffect(() => {
    if (node) {
      setCpuRatio(node.capacity?.oversell.cpu_ratio ?? 2.0);
      setRamRatio(node.capacity?.oversell.ram_ratio ?? 1.25);
      setDiskRatio(node.capacity?.oversell.disk_ratio ?? 1.0);
      setMaxTenants(node.capacity?.max_tenants ?? 50);
    }
  }, [node]);

  if (!node) return null;

  const totalCores = node.capabilities.cpu_cores || 1;
  const effectiveCores = Math.round(totalCores * cpuRatio * 10) / 10;

  const totalRamGb = Math.round((node.capabilities.ram_total_bytes / (1024 * 1024 * 1024)) * 10) / 10;
  const effectiveRamGb = Math.round(totalRamGb * ramRatio * 10) / 10;

  const totalDiskGb = Math.round((node.capabilities.disk_total_bytes / (1024 * 1024 * 1024)) * 10) / 10;
  const effectiveDiskGb = Math.round(totalDiskGb * diskRatio * 10) / 10;

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    try {
      setIsSubmitting(true);
      const oversellPayload: NodeOversellConfig = {
        cpu_ratio: cpuRatio,
        ram_ratio: ramRatio,
        disk_ratio: diskRatio,
      };

      // TODO(api:T16.01): update node oversell config and capacity model
      await apiFetch(`/api/v1/admin/nodes/${node.id}/capacity`, {
        method: "PUT",
        headers: {
          "Content-Type": "application/json",
          "X-CSRF-Token": getCsrfToken() || "",
        },
        body: JSON.stringify({
          oversell: oversellPayload,
          max_tenants: maxTenants,
        }),
      });

      toast({
        title: t("nodes.capacity.saveSuccess"),
        variant: "default",
      });
      onOpenChange(false);
      onSaved();
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
      <DialogContent className="max-w-lg">
        <form onSubmit={handleSubmit}>
          <DialogHeader>
            <div className="flex items-center gap-2">
              <Layers className="h-5 w-5 text-primary" />
              <DialogTitle>{t("nodes.capacity.oversellSettings")}</DialogTitle>
            </div>
            <DialogDescription className="text-xs">
              {node.name} ({node.hostname}) — {t("nodes.capacity.oversellDesc")}
            </DialogDescription>
          </DialogHeader>

          <div className="space-y-4 py-4 text-xs">
            {/* CPU Oversell */}
            <div className="p-3 border rounded-lg bg-muted/20 space-y-2">
              <div className="flex items-center justify-between">
                <label htmlFor="cpu-ratio-input" className="font-semibold flex items-center gap-1.5 text-foreground">
                  <Cpu className="h-4 w-4 text-primary" />
                  {t("nodes.capacity.cpuRatioLabel")}
                </label>
                <span className="font-mono font-bold text-primary">{cpuRatio.toFixed(1)}x</span>
              </div>
              <Input
                id="cpu-ratio-input"
                type="number"
                min="1.0"
                max="5.0"
                step="0.1"
                value={cpuRatio}
                onChange={(e) => setCpuRatio(parseFloat(e.target.value) || 1.0)}
                className="h-8 text-xs font-mono"
              />
              <p className="text-[11px] text-muted-foreground">
                Physical: {totalCores} cores → Effective Commitment: <strong className="text-foreground">{effectiveCores} cores</strong>
              </p>
            </div>

            {/* RAM Oversell */}
            <div className="p-3 border rounded-lg bg-muted/20 space-y-2">
              <div className="flex items-center justify-between">
                <label htmlFor="ram-ratio-input" className="font-semibold flex items-center gap-1.5 text-foreground">
                  <Activity className="h-4 w-4 text-primary" />
                  {t("nodes.capacity.ramRatioLabel")}
                </label>
                <span className="font-mono font-bold text-primary">{ramRatio.toFixed(2)}x</span>
              </div>
              <Input
                id="ram-ratio-input"
                type="number"
                min="1.0"
                max="2.5"
                step="0.05"
                value={ramRatio}
                onChange={(e) => setRamRatio(parseFloat(e.target.value) || 1.0)}
                className="h-8 text-xs font-mono"
              />
              <p className="text-[11px] text-muted-foreground">
                Physical: {totalRamGb} GB → Effective Commitment: <strong className="text-foreground">{effectiveRamGb} GB</strong>
              </p>
            </div>

            {/* Disk Oversell */}
            <div className="p-3 border rounded-lg bg-muted/20 space-y-2">
              <div className="flex items-center justify-between">
                <label htmlFor="disk-ratio-input" className="font-semibold flex items-center gap-1.5 text-foreground">
                  <HardDrive className="h-4 w-4 text-primary" />
                  {t("nodes.capacity.diskRatioLabel")}
                </label>
                <span className="font-mono font-bold text-primary">{diskRatio.toFixed(2)}x</span>
              </div>
              <Input
                id="disk-ratio-input"
                type="number"
                min="1.0"
                max="2.0"
                step="0.05"
                value={diskRatio}
                onChange={(e) => setDiskRatio(parseFloat(e.target.value) || 1.0)}
                className="h-8 text-xs font-mono"
              />
              <p className="text-[11px] text-muted-foreground">
                Physical: {totalDiskGb} GB → Effective Commitment: <strong className="text-foreground">{effectiveDiskGb} GB</strong>
              </p>
            </div>

            {/* Max Tenants */}
            <div className="p-3 border rounded-lg bg-muted/20 space-y-2">
              <div className="flex items-center justify-between">
                <label htmlFor="max-tenants-input" className="font-semibold flex items-center gap-1.5 text-foreground">
                  <Server className="h-4 w-4 text-primary" />
                  {t("nodes.capacity.maxTenantsLabel")}
                </label>
                <span className="font-mono font-bold text-primary">{maxTenants}</span>
              </div>
              <Input
                id="max-tenants-input"
                type="number"
                min="1"
                max="500"
                step="1"
                value={maxTenants}
                onChange={(e) => setMaxTenants(parseInt(e.target.value, 10) || 50)}
                className="h-8 text-xs font-mono"
              />
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
              type="submit"
              size="sm"
              disabled={isSubmitting}
              className="gap-1.5"
            >
              {isSubmitting ? t("nodes.wizard.generating") : t("common.save")}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
