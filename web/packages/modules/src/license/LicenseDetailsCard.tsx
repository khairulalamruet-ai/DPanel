import React, { useState } from "react";
import { useTranslation } from "@builtpanel/i18n";
import { Card, CardHeader, CardTitle, CardDescription, CardContent, Button, Badge } from "@builtpanel/ui";
import {
  KeyRound,
  Copy,
  Check,
  Server,
  Calendar,
  Clock,
  Shield,
  LifeBuoy,
  RotateCw,
  Eye,
  EyeOff,
  Building,
  Hash,
} from "lucide-react";
import { LicenseState } from "./types";

interface LicenseDetailsCardProps {
  license: LicenseState;
  onOpenActivate: () => void;
  onOpenRecovery: () => void;
  onRevalidate: () => void;
  revalidating: boolean;
}

export function LicenseDetailsCard({
  license,
  onOpenActivate,
  onOpenRecovery,
  onRevalidate,
  revalidating,
}: LicenseDetailsCardProps) {
  const { t } = useTranslation();
  const [showKey, setShowKey] = useState(false);
  const [copiedKey, setCopiedKey] = useState(false);
  const [copiedInstallId, setCopiedInstallId] = useState(false);

  const handleCopyKey = () => {
    navigator.clipboard.writeText(license.license_key);
    setCopiedKey(true);
    setTimeout(() => setCopiedKey(false), 2000);
  };

  const handleCopyInstallId = () => {
    navigator.clipboard.writeText(license.installation_id);
    setCopiedInstallId(true);
    setTimeout(() => setCopiedInstallId(false), 2000);
  };

  const maskedKey = (license.license_key || "").replace(
    /^(DHP-[A-Z0-9]+-)(.+)(-[A-Z0-9]+)$/,
    (_, p1, p2, p3) => `${p1}${"•".repeat(p2.length)}${p3}`
  );

  return (
    <Card className="border shadow-xs" data-testid="license-details-card">
      <CardHeader className="flex flex-col sm:flex-row sm:items-center justify-between pb-3 gap-2">
        <div>
          <CardTitle className="text-base font-bold flex items-center gap-2" data-testid="license-card-title">
            <KeyRound className="h-4 w-4 text-primary" />
            <span>{t("license.detailsTitle", "License & Installation Identity")}</span>
          </CardTitle>
          <CardDescription className="text-xs">
            {t(
              "license.detailsSubtitle",
              "Cryptographic identity, signing key verification, and entitlement metadata."
            )}
          </CardDescription>
        </div>

        <div className="flex items-center gap-2">
          <Button
            variant="outline"
            size="sm"
            onClick={onRevalidate}
            disabled={revalidating}
            className="text-xs gap-1.5 h-8"
            data-testid="btn-revalidate-card"
          >
            <RotateCw className={`h-3.5 w-3.5 ${revalidating ? "animate-spin" : ""}`} />
            <span>{t("license.revalidateBtn", "Revalidate")}</span>
          </Button>

          <Button
            variant="outline"
            size="sm"
            onClick={onOpenRecovery}
            className="text-xs gap-1.5 h-8"
            data-testid="btn-recovery-card"
          >
            <LifeBuoy className="h-3.5 w-3.5" />
            <span>{t("license.recoveryBtn", "Recovery")}</span>
          </Button>

          <Button
            size="sm"
            onClick={onOpenActivate}
            className="text-xs gap-1.5 h-8"
            data-testid="btn-change-license-key"
          >
            <KeyRound className="h-3.5 w-3.5" />
            <span>{t("license.changeKeyBtn", "Change Key")}</span>
          </Button>
        </div>
      </CardHeader>

      <CardContent className="space-y-4 pt-1">
        {/* Keys & Identity Row */}
        <div className="grid grid-cols-1 md:grid-cols-2 gap-3">
          {/* License Key */}
          {license.license_key && (
          <div className="p-3 rounded-lg border bg-muted/20 space-y-1.5">
            <div className="flex items-center justify-between">
              <span className="text-xs font-semibold text-muted-foreground flex items-center gap-1.5">
                <KeyRound className="h-3.5 w-3.5 text-primary" />
                {t("license.licenseKeyLabel", "License Key")}
              </span>
              <div className="flex items-center gap-1">
                <button
                  type="button"
                  onClick={() => setShowKey(!showKey)}
                  className="p-1 text-muted-foreground hover:text-foreground text-xs"
                  title={showKey ? "Hide key" : "Show key"}
                  data-testid="btn-toggle-show-key"
                >
                  {showKey ? <EyeOff className="h-3.5 w-3.5" /> : <Eye className="h-3.5 w-3.5" />}
                </button>
                <button
                  type="button"
                  onClick={handleCopyKey}
                  className="p-1 text-muted-foreground hover:text-foreground text-xs"
                  title="Copy license key"
                  data-testid="btn-copy-license-key"
                >
                  {copiedKey ? <Check className="h-3.5 w-3.5 text-emerald-500" /> : <Copy className="h-3.5 w-3.5" />}
                </button>
              </div>
            </div>
            <div className="font-mono text-xs font-bold truncate select-all" data-testid="text-license-key">
              {showKey ? license.license_key : maskedKey}
            </div>
          </div>
          )}

          {/* Installation ID */}
          <div className="p-3 rounded-lg border bg-muted/20 space-y-1.5">
            <div className="flex items-center justify-between">
              <span className="text-xs font-semibold text-muted-foreground flex items-center gap-1.5">
                <Hash className="h-3.5 w-3.5 text-primary" />
                {t("license.installationIdLabel", "Installation Identity (UUID)")}
              </span>
              <button
                type="button"
                onClick={handleCopyInstallId}
                className="p-1 text-muted-foreground hover:text-foreground text-xs"
                title="Copy Installation ID"
                data-testid="btn-copy-installation-id"
              >
                {copiedInstallId ? <Check className="h-3.5 w-3.5 text-emerald-500" /> : <Copy className="h-3.5 w-3.5" />}
              </button>
            </div>
            <div className="font-mono text-xs font-bold truncate select-all" data-testid="text-installation-id">
              {license.installation_id}
            </div>
          </div>
        </div>

        {/* Metadata Details Grid */}
        <div className="grid grid-cols-2 sm:grid-cols-4 gap-3 text-xs pt-1">
          <div className="space-y-1">
            <span className="text-muted-foreground block text-[11px] font-medium">
              {t("license.productTierLabel", "Product & Tier")}
            </span>
            <div className="font-semibold flex items-center gap-1.5">
              <span data-testid="text-product-name">{license.product_name}</span>
              <Badge variant="outline" className="text-[10px] font-mono uppercase" data-testid="badge-plan-tier">
                {license.plan_tier}
              </Badge>
            </div>
          </div>

          <div className="space-y-1">
            <span className="text-muted-foreground block text-[11px] font-medium">
              {t("license.customerLabel", "Licensed Organization")}
            </span>
            <div className="font-semibold truncate" data-testid="text-customer-name">
              {license.customer_name}
            </div>
            <div className="text-[11px] text-muted-foreground truncate font-mono">
              {license.customer_email}
            </div>
          </div>

          <div className="space-y-1">
            <span className="text-muted-foreground block text-[11px] font-medium">
              {t("license.validityPeriodLabel", "Validity Period")}
            </span>
            <div className="font-mono text-[11px]">
              <span className="text-muted-foreground">Issued: </span>
              <span data-testid="text-issued-at">{license.issued_at.slice(0, 10)}</span>
            </div>
            <div className="font-mono text-[11px]">
              <span className="text-muted-foreground">Expires: </span>
              <span className="font-bold text-primary" data-testid="text-expires-at">
                {license.expires_at.slice(0, 10)}
              </span>
            </div>
          </div>

          <div className="space-y-1">
            <span className="text-muted-foreground block text-[11px] font-medium">
              {t("license.cryptoLabel", "Signing Key (Ed25519)")}
            </span>
            <div className="font-mono text-[11px] truncate" data-testid="text-kid">
              kid: {license.signing_key_id}
            </div>
            <div className="text-[11px] text-emerald-600 dark:text-emerald-400 font-semibold flex items-center gap-1">
              <Shield className="h-3 w-3" />
              <span>Signature Verified</span>
            </div>
          </div>
        </div>

        {/* Heartbeat & Sync Row */}
        <div className="p-2.5 rounded-lg border bg-muted/10 flex flex-col sm:flex-row sm:items-center justify-between text-xs text-muted-foreground gap-2">
          <div className="flex items-center gap-4">
            <span className="flex items-center gap-1.5">
              <Clock className="h-3.5 w-3.5 text-primary" />
              <span>{t("license.lastHeartbeat", "Last Heartbeat")}:</span>
              <span className="font-mono font-medium text-foreground" data-testid="text-last-heartbeat">
                {new Date(license.last_heartbeat_at).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}
              </span>
            </span>

            <span className="flex items-center gap-1.5">
              <span>{t("license.nextRefresh", "Next Entitlement Refresh")}:</span>
              <span className="font-mono font-medium text-foreground" data-testid="text-next-refresh">
                {new Date(license.next_refresh_at).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}
              </span>
            </span>
          </div>

          <span className="text-[11px] italic">
            {t("license.heartbeatIntervalNote", "Heartbeat every 6h • 72h offline grace")}
          </span>
        </div>
      </CardContent>
    </Card>
  );
}
