import React from "react";
import { Card, CardContent, Badge } from "@builtpanel/ui";
import { useTranslation } from "@builtpanel/i18n";
import { Users, HardDrive, Activity, ShieldCheck, AlertCircle } from "lucide-react";
import { ResellerAccount, ResellerPackage } from "./types";

interface ResellerStatsGridProps {
  account: ResellerAccount;
  pkg?: ResellerPackage;
}

export function ResellerStatsGrid({ account, pkg }: ResellerStatsGridProps) {
  const { t } = useTranslation();

  const maxAccounts = pkg?.max_accounts || 50;
  const accountsPercent = Math.min(100, Math.round((account.accounts_count / maxAccounts) * 100));

  const maxDisk = pkg?.max_disk_gb || 200;
  const diskPercent = Math.min(100, Math.round((account.disk_used_gb / maxDisk) * 100));

  const maxBw = pkg?.max_bandwidth_gb || 2000;
  const bwPercent = Math.min(100, Math.round((account.bandwidth_used_gb / maxBw) * 100));

  return (
    <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-4 gap-4" data-testid="reseller-stats-grid">
      {/* 1. Accounts Quota */}
      <Card className="border shadow-xs">
        <CardContent className="p-4 space-y-2">
          <div className="flex items-center justify-between text-muted-foreground text-xs font-semibold">
            <span>{t("reseller.accountsQuota", "Tenant Accounts")}</span>
            <Users className="h-4 w-4 text-primary" />
          </div>
          <div className="flex items-baseline justify-between">
            <span className="text-2xl font-bold font-mono" data-testid="stat-accounts-count">
              {account.accounts_count}
            </span>
            <span className="text-xs text-muted-foreground font-mono">
              / {maxAccounts} max
            </span>
          </div>
          <div className="w-full bg-muted rounded-full h-1.5 overflow-hidden">
            <div
              className={`h-full rounded-full transition-all ${
                accountsPercent > 90 ? "bg-red-500" : accountsPercent > 70 ? "bg-amber-500" : "bg-primary"
              }`}
              style={{ width: `${accountsPercent}%` }}
            />
          </div>
        </CardContent>
      </Card>

      {/* 2. Disk Usage */}
      <Card className="border shadow-xs">
        <CardContent className="p-4 space-y-2">
          <div className="flex items-center justify-between text-muted-foreground text-xs font-semibold">
            <span>{t("reseller.diskAllocation", "Storage Quota")}</span>
            <HardDrive className="h-4 w-4 text-primary" />
          </div>
          <div className="flex items-baseline justify-between">
            <span className="text-2xl font-bold font-mono" data-testid="stat-disk-used">
              {account.disk_used_gb} GB
            </span>
            <span className="text-xs text-muted-foreground font-mono">
              / {maxDisk} GB
            </span>
          </div>
          <div className="w-full bg-muted rounded-full h-1.5 overflow-hidden">
            <div
              className={`h-full rounded-full transition-all ${
                diskPercent > 90 ? "bg-red-500" : diskPercent > 70 ? "bg-amber-500" : "bg-primary"
              }`}
              style={{ width: `${diskPercent}%` }}
            />
          </div>
        </CardContent>
      </Card>

      {/* 3. Bandwidth Usage */}
      <Card className="border shadow-xs">
        <CardContent className="p-4 space-y-2">
          <div className="flex items-center justify-between text-muted-foreground text-xs font-semibold">
            <span>{t("reseller.bandwidthAllocation", "Bandwidth")}</span>
            <Activity className="h-4 w-4 text-primary" />
          </div>
          <div className="flex items-baseline justify-between">
            <span className="text-2xl font-bold font-mono" data-testid="stat-bw-used">
              {account.bandwidth_used_gb} GB
            </span>
            <span className="text-xs text-muted-foreground font-mono">
              / {maxBw} GB
            </span>
          </div>
          <div className="w-full bg-muted rounded-full h-1.5 overflow-hidden">
            <div
              className={`h-full rounded-full transition-all ${
                bwPercent > 90 ? "bg-red-500" : bwPercent > 70 ? "bg-amber-500" : "bg-primary"
              }`}
              style={{ width: `${bwPercent}%` }}
            />
          </div>
        </CardContent>
      </Card>

      {/* 4. Package & Overselling */}
      <Card className="border shadow-xs">
        <CardContent className="p-4 space-y-2">
          <div className="flex items-center justify-between text-muted-foreground text-xs font-semibold">
            <span>{t("reseller.packageTier", "Reseller Tier")}</span>
            <ShieldCheck className="h-4 w-4 text-primary" />
          </div>
          <div className="flex items-center justify-between pt-1">
            <span className="font-bold text-sm truncate max-w-[120px]" data-testid="stat-package-name">
              {account.package_name}
            </span>
            <Badge variant={account.status === "active" ? "success" : "destructive"}>
              {account.status}
            </Badge>
          </div>
          <div className="flex items-center gap-1.5 text-[11px] text-muted-foreground pt-1">
            {pkg?.allow_overselling ? (
              <span className="text-emerald-600 dark:text-emerald-400 font-medium flex items-center gap-1">
                <ShieldCheck className="h-3 w-3" />
                {t("reseller.oversellingAllowed", "Overselling Enabled")}
              </span>
            ) : (
              <span className="text-muted-foreground flex items-center gap-1">
                <AlertCircle className="h-3 w-3" />
                {t("reseller.oversellingStrict", "Hard Quota Enforcement")}
              </span>
            )}
          </div>
        </CardContent>
      </Card>
    </div>
  );
}
