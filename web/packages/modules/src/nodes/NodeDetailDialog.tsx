import React from "react";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
  Button,
  Badge,
} from "@builtpanel/ui";
import { useTranslation, useFormatters } from "@builtpanel/i18n";
import { Server, Cpu, HardDrive, Shield, Activity, Fingerprint, Layers, Users } from "lucide-react";
import type { NodeItem } from "./types";
import { computeNodeSaturation, getSaturationColor } from "./capacity";

interface NodeDetailDialogProps {
  node: NodeItem | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}

export function NodeDetailDialog({ node, open, onOpenChange }: NodeDetailDialogProps) {
  const { t } = useTranslation();
  const { formatDate } = useFormatters();

  if (!node) return null;

  const ramUsedGb = (node.capabilities.ram_used_bytes / (1024 * 1024 * 1024)).toFixed(1);
  const ramTotalGb = (node.capabilities.ram_total_bytes / (1024 * 1024 * 1024)).toFixed(1);
  const diskUsedGb = (node.capabilities.disk_used_bytes / (1024 * 1024 * 1024)).toFixed(1);
  const diskTotalGb = (node.capabilities.disk_total_bytes / (1024 * 1024 * 1024)).toFixed(1);

  const saturationPct = computeNodeSaturation(node);
  const satColor = getSaturationColor(saturationPct);

  const cpuRatio = node.capacity?.oversell.cpu_ratio ?? 2.0;
  const ramRatio = node.capacity?.oversell.ram_ratio ?? 1.25;
  const diskRatio = node.capacity?.oversell.disk_ratio ?? 1.0;

  const allocatedCores = node.capacity?.allocated_cpu_cores ?? Math.round(((node.capabilities.cpu_usage_pct || 1) / 100) * (node.capabilities.cpu_cores || 1));
  const effectiveCores = Math.round((node.capabilities.cpu_cores || 1) * cpuRatio * 10) / 10;

  const allocatedRamGb = node.capacity
    ? (node.capacity.allocated_ram_bytes / (1024 * 1024 * 1024)).toFixed(1)
    : ramUsedGb;
  const effectiveRamGb = (parseFloat(ramTotalGb) * ramRatio).toFixed(1);

  const allocatedDiskGb = node.capacity
    ? (node.capacity.allocated_disk_bytes / (1024 * 1024 * 1024)).toFixed(1)
    : diskUsedGb;
  const effectiveDiskGb = (parseFloat(diskTotalGb) * diskRatio).toFixed(1);

  const tenantCount = node.capacity?.tenants_count ?? 0;
  const maxTenants = node.capacity?.max_tenants ?? 50;

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-2xl max-h-[85vh] overflow-y-auto">
        <DialogHeader>
          <div className="flex items-center gap-2">
            <Server className="h-5 w-5 text-primary" />
            <DialogTitle>{node.name}</DialogTitle>
          </div>
          <DialogDescription className="font-mono text-xs">
            {node.hostname} • {node.ip_address}
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-4 py-2">
          {/* Node Capacity & Saturation (T16.01) */}
          <div className="p-4 rounded-lg border bg-card space-y-3" data-testid="node-detail-capacity">
            <div className="flex items-center justify-between">
              <div className="flex items-center gap-2">
                <Layers className="h-4 w-4 text-primary" />
                <span className="font-semibold text-sm">{t("nodes.capacity.title")}</span>
              </div>
              <Badge variant={satColor.badgeVariant} className="font-mono text-xs font-semibold">
                {t("nodes.capacity.fullness")}: {saturationPct}%
              </Badge>
            </div>

            {/* Saturation Progress Bar */}
            <div className="space-y-1">
              <div className="w-full bg-muted rounded-full h-2.5 overflow-hidden">
                <div
                  className={`h-2.5 rounded-full transition-all ${satColor.bar}`}
                  style={{ width: `${saturationPct}%` }}
                />
              </div>
              <div className="flex justify-between text-[11px] text-muted-foreground font-mono">
                <span>0%</span>
                <span>{saturationPct}% {t("nodes.capacity.saturation")}</span>
                <span>100%</span>
              </div>
            </div>

            {/* Capacity & Oversell Breakdown Grid */}
            <div className="grid grid-cols-1 sm:grid-cols-3 gap-2.5 pt-1 text-xs">
              <div className="p-2.5 rounded border bg-muted/20 space-y-1">
                <div className="flex justify-between font-medium">
                  <span className="text-muted-foreground">CPU</span>
                  <span className="font-mono font-semibold text-primary">{cpuRatio.toFixed(1)}x</span>
                </div>
                <div className="font-mono text-[11px] text-foreground font-medium">
                  {allocatedCores} / {effectiveCores} cores
                </div>
                <div className="text-[10px] text-muted-foreground">
                  Physical: {node.capabilities.cpu_cores} cores • Load: {node.capabilities.cpu_usage_pct}%
                </div>
              </div>

              <div className="p-2.5 rounded border bg-muted/20 space-y-1">
                <div className="flex justify-between font-medium">
                  <span className="text-muted-foreground">RAM</span>
                  <span className="font-mono font-semibold text-primary">{ramRatio.toFixed(2)}x</span>
                </div>
                <div className="font-mono text-[11px] text-foreground font-medium">
                  {allocatedRamGb} / {effectiveRamGb} GB
                </div>
                <div className="text-[10px] text-muted-foreground">
                  Physical: {ramTotalGb} GB • Used: {ramUsedGb} GB
                </div>
              </div>

              <div className="p-2.5 rounded border bg-muted/20 space-y-1">
                <div className="flex justify-between font-medium">
                  <span className="text-muted-foreground">Disk</span>
                  <span className="font-mono font-semibold text-primary">{diskRatio.toFixed(2)}x</span>
                </div>
                <div className="font-mono text-[11px] text-foreground font-medium">
                  {allocatedDiskGb} / {effectiveDiskGb} GB
                </div>
                <div className="text-[10px] text-muted-foreground">
                  Physical: {diskTotalGb} GB • Used: {diskUsedGb} GB
                </div>
              </div>
            </div>

            {/* Tenant Density */}
            <div className="flex items-center justify-between text-xs pt-1 border-t text-muted-foreground">
              <div className="flex items-center gap-1.5">
                <Users className="h-3.5 w-3.5 text-primary" />
                <span>{t("nodes.capacity.tenants")}:</span>
                <strong className="text-foreground font-mono">{tenantCount} / {maxTenants}</strong>
              </div>
              <div className="text-[11px]">
                {node.status === "draining" ? (
                  <span className="text-amber-500 font-medium">● {t("nodes.status.draining")}</span>
                ) : (
                  <span className="text-emerald-500 font-medium">● {t("nodes.status.online")}</span>
                )}
              </div>
            </div>
          </div>
          {/* Hardware & Resources Grid */}
          <div className="grid grid-cols-1 sm:grid-cols-2 gap-3 text-xs">
            <div className="p-3 rounded-lg border bg-muted/30 space-y-1">
              <div className="flex items-center gap-1.5 text-muted-foreground font-semibold">
                <Cpu className="h-4 w-4 text-primary" />
                {t("nodes.details.cpuModel")}
              </div>
              <p className="font-medium text-foreground">{node.capabilities.cpu_model}</p>
              <p className="text-muted-foreground">
                {node.capabilities.cpu_cores} Cores • {node.capabilities.cpu_usage_pct}% load
              </p>
            </div>

            <div className="p-3 rounded-lg border bg-muted/30 space-y-1">
              <div className="flex items-center gap-1.5 text-muted-foreground font-semibold">
                <Activity className="h-4 w-4 text-primary" />
                {t("nodes.table.ram")}
              </div>
              <p className="font-medium text-foreground">
                {ramUsedGb} GB / {ramTotalGb} GB
              </p>
              <p className="text-muted-foreground">
                {Math.round((node.capabilities.ram_used_bytes / node.capabilities.ram_total_bytes) * 100)}% utilization
              </p>
            </div>

            <div className="p-3 rounded-lg border bg-muted/30 space-y-1">
              <div className="flex items-center gap-1.5 text-muted-foreground font-semibold">
                <HardDrive className="h-4 w-4 text-primary" />
                {t("nodes.table.disk")}
              </div>
              <p className="font-medium text-foreground">
                {diskUsedGb} GB / {diskTotalGb} GB
              </p>
              <p className="text-muted-foreground">
                {Math.round((node.capabilities.disk_used_bytes / node.capabilities.disk_total_bytes) * 100)}% utilization
              </p>
            </div>

            <div className="p-3 rounded-lg border bg-muted/30 space-y-1">
              <div className="flex items-center gap-1.5 text-muted-foreground font-semibold">
                <Server className="h-4 w-4 text-primary" />
                {t("nodes.details.docker")}
              </div>
              <p className="font-medium text-foreground">{node.capabilities.docker_version}</p>
              <p className="text-muted-foreground font-mono text-[11px]">{node.capabilities.kernel}</p>
            </div>
          </div>

          {/* Security Features */}
          <div className="p-3.5 rounded-lg border bg-muted/30 space-y-2 text-xs">
            <div className="flex items-center gap-1.5 text-muted-foreground font-semibold">
              <Shield className="h-4 w-4 text-emerald-500" />
              {t("nodes.details.securityFeatures")}
            </div>
            <div className="flex flex-wrap gap-1.5">
              {node.capabilities.security_features.map((feature) => (
                <Badge key={feature} variant="outline" className="text-[11px] font-mono bg-background">
                  {feature}
                </Badge>
              ))}
            </div>
          </div>

          {/* mTLS Identity & Fingerprint */}
          <div className="p-3.5 rounded-lg border bg-muted/30 space-y-2 text-xs">
            <div className="flex items-center gap-1.5 text-muted-foreground font-semibold">
              <Fingerprint className="h-4 w-4 text-primary" />
              {t("nodes.details.fingerprint")}
            </div>
            <p className="font-mono text-[11px] break-all bg-muted/60 p-2 rounded border">
              {node.fingerprint}
            </p>
          </div>

          {/* Metadata Row */}
          <div className="grid grid-cols-2 gap-2 text-xs text-muted-foreground pt-1">
            <div>
              <span className="font-medium">{t("nodes.details.enrolledAt")}: </span>
              <span>{formatDate(new Date(node.enrolled_at))}</span>
            </div>
            <div>
              <span className="font-medium">{t("nodes.table.lastSeen")}: </span>
              <span>{formatDate(new Date(node.last_seen_at))}</span>
            </div>
          </div>
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            {t("nodes.details.close")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
