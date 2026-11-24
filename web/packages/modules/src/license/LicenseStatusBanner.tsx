import React from "react";
import { useTranslation } from "@builtpanel/i18n";
import { Badge, Button } from "@builtpanel/ui";
import {
  CheckCircle2,
  AlertTriangle,
  XCircle,
  Clock,
  ShieldCheck,
  ShieldAlert,
  RotateCw,
  KeyRound,
  LifeBuoy,
} from "lucide-react";
import { LicenseState } from "./types";

interface LicenseStatusBannerProps {
  license: LicenseState;
  onRevalidate: () => void;
  onOpenActivate: () => void;
  onOpenRecovery: () => void;
  revalidating: boolean;
}

export function LicenseStatusBanner({
  license,
  onRevalidate,
  onOpenActivate,
  onOpenRecovery,
  revalidating,
}: LicenseStatusBannerProps) {
  const { t } = useTranslation();

  if (license.status === "active") {
    return (
      <div
        className="p-4 rounded-xl border border-emerald-500/30 bg-emerald-500/10 flex flex-col sm:flex-row items-start sm:items-center justify-between gap-4"
        data-testid="license-banner-active"
      >
        <div className="flex items-start sm:items-center gap-3">
          <div className="p-2.5 rounded-lg bg-emerald-500/20 text-emerald-600 dark:text-emerald-400 shrink-0">
            <ShieldCheck className="h-6 w-6" />
          </div>
          <div className="space-y-0.5">
            <div className="flex items-center gap-2">
              <span className="font-semibold text-sm text-foreground">
                {t("license.activeTitle", "License Verified & Cryptographically Signed")}
              </span>
              <Badge variant="success" className="text-[11px] font-mono">
                {t("license.statusActive", "ACTIVE")}
              </Badge>
              <Badge variant="outline" className="text-[10px] font-mono border-emerald-500/40 text-emerald-600 dark:text-emerald-400">
                Ed25519 Valid
              </Badge>
            </div>
            <p className="text-xs text-muted-foreground">
              {t(
                "license.activeSubtitle",
                "Entitlement verified by License Authority. All licensed features and node limits are operational."
              )}
            </p>
          </div>
        </div>

        <div className="flex items-center gap-2 shrink-0 self-end sm:self-center">
          <Button
            variant="outline"
            size="sm"
            onClick={onRevalidate}
            disabled={revalidating}
            className="text-xs gap-1.5 h-8"
            data-testid="btn-revalidate-license"
          >
            <RotateCw className={`h-3.5 w-3.5 ${revalidating ? "animate-spin" : ""}`} />
            {t("license.verifyNow", "Revalidate")}
          </Button>
        </div>
      </div>
    );
  }

  if (license.status === "grace_period") {
    return (
      <div
        className="p-4 rounded-xl border border-amber-500/40 bg-amber-500/10 flex flex-col sm:flex-row items-start sm:items-center justify-between gap-4"
        data-testid="license-banner-grace"
      >
        <div className="flex items-start sm:items-center gap-3">
          <div className="p-2.5 rounded-lg bg-amber-500/20 text-amber-600 dark:text-amber-400 shrink-0">
            <Clock className="h-6 w-6" />
          </div>
          <div className="space-y-0.5">
            <div className="flex items-center gap-2">
              <span className="font-semibold text-sm text-amber-700 dark:text-amber-300">
                {t("license.graceTitle", "Offline Grace Period Active")}
              </span>
              <Badge variant="outline" className="border-amber-500 text-amber-700 dark:text-amber-300 font-mono text-[11px]">
                {license.offline_grace_hours_remaining ?? 72}h Remaining
              </Badge>
            </div>
            <p className="text-xs text-muted-foreground">
              {t(
                "license.graceSubtitle",
                "License Authority could not be reached on the last heartbeat. The panel remains fully functional for the remainder of the 72-hour grace period."
              )}
            </p>
          </div>
        </div>

        <div className="flex items-center gap-2 shrink-0 self-end sm:self-center">
          <Button
            variant="outline"
            size="sm"
            onClick={onRevalidate}
            disabled={revalidating}
            className="text-xs gap-1.5 h-8 border-amber-500/50 hover:bg-amber-500/20"
            data-testid="btn-revalidate-grace"
          >
            <RotateCw className={`h-3.5 w-3.5 ${revalidating ? "animate-spin" : ""}`} />
            {t("license.retryHeartbeat", "Retry Connection")}
          </Button>
        </div>
      </div>
    );
  }

  // Restricted or Expired / Suspended mode
  return (
    <div
      className="p-4 rounded-xl border border-destructive/50 bg-destructive/10 flex flex-col gap-3"
      data-testid="license-banner-restricted"
    >
      <div className="flex items-start gap-3">
        <div className="p-2.5 rounded-lg bg-destructive/20 text-destructive shrink-0">
          <ShieldAlert className="h-6 w-6" />
        </div>
        <div className="space-y-1 flex-1">
          <div className="flex items-center gap-2">
            <span className="font-bold text-sm text-destructive">
              {t("license.restrictedTitle", "Restricted Management Mode Active")}
            </span>
            <Badge variant="destructive" className="font-mono text-[11px] uppercase">
              {license.status}
            </Badge>
          </div>
          <p className="text-xs text-foreground/90">
            {license.restricted_reason ||
              t(
                "license.restrictedReasonDefault",
                "License expired or failed validation. Management mutations are temporarily restricted to read-only."
              )}
          </p>
          <div className="p-2.5 rounded-lg bg-background/80 border text-[11px] text-muted-foreground flex items-center gap-2 mt-2">
            <CheckCircle2 className="h-4 w-4 text-emerald-500 shrink-0" />
            <span>
              {t(
                "license.restrictedSafetyNotice",
                "Safety Invariant: Websites, Databases, DNS, and Mail services remain 100% online and operational."
              )}
            </span>
          </div>
        </div>
      </div>

      <div className="flex items-center justify-end gap-2 pt-1 border-t border-destructive/20">
        <Button
          variant="outline"
          size="sm"
          onClick={onOpenRecovery}
          className="text-xs gap-1.5 h-8 border-destructive/40 hover:bg-destructive/10"
          data-testid="btn-open-recovery"
        >
          <LifeBuoy className="h-3.5 w-3.5" />
          {t("license.requestRecoveryBtn", "Emergency Recovery Request")}
        </Button>

        <Button
          size="sm"
          onClick={onOpenActivate}
          className="text-xs gap-1.5 h-8"
          data-testid="btn-open-activate-banner"
        >
          <KeyRound className="h-3.5 w-3.5" />
          {t("license.enterKeyBtn", "Enter License Key")}
        </Button>
      </div>
    </div>
  );
}
