import React, { useState, useEffect, useCallback } from "react";
import { useTranslation } from "@builtpanel/i18n";
import { getLicense, refreshLicense, activateLicense, requestRecovery } from "./api";
import { useToast, Button } from "@builtpanel/ui";
import { KeyRound, RefreshCw, LifeBuoy, AlertCircle } from "lucide-react";
import {
  LicenseState,
  RecoveryRequestPayload,
  RecoveryRequestResponse,
} from "./types";
import { LicenseStatusBanner } from "./LicenseStatusBanner";
import { LicenseDetailsCard } from "./LicenseDetailsCard";
import { LicenseEntitlementsCard } from "./LicenseEntitlementsCard";
import { ActivateLicenseDialog } from "./ActivateLicenseDialog";
import { RecoveryRequestDialog } from "./RecoveryRequestDialog";

export function LicensePage() {
  const { t } = useTranslation();
  const { toast } = useToast();

  const [license, setLicense] = useState<LicenseState | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  const [isError, setIsError] = useState(false);
  const [revalidating, setRevalidating] = useState(false);

  const [activateOpen, setActivateOpen] = useState(false);
  const [recoveryOpen, setRecoveryOpen] = useState(false);

  const loadLicense = useCallback(async () => {
    try {
      setIsLoading(true);
      setIsError(false);
      const res = await getLicense();
      setLicense(res);
    } catch {
      setIsError(true);
    } finally {
      setIsLoading(false);
    }
  }, []);

  useEffect(() => {
    loadLicense();
  }, [loadLicense]);

  // Actions
  const handleRevalidate = async () => {
    try {
      setRevalidating(true);
      const updated = await refreshLicense();
      setLicense(updated);
      toast({
        title: t("license.validatedTitle", "License Validated"),
        description: t("license.validatedDesc", "Entitlement verified with License Authority. Ed25519 signature valid."),
        variant: "success",
      });
    } catch (err: any) {
      toast({
        title: t("license.validateFailedTitle", "Validation Failed"),
        description: err.message || t("license.validateFailedDesc", "Could not verify license status."),
        variant: "destructive",
      });
    } finally {
      setRevalidating(false);
    }
  };

  const handleActivate = async (licenseKey: string) => {
    const activated = await activateLicense(licenseKey);
    setLicense(activated);
    toast({
      title: t("license.activatedTitle", "License Activated"),
      description: t("license.activatedDesc", "License {{key}} successfully verified.", {
        key: activated.license_key,
      }),
      variant: "success",
    });
  };

  const handleRecoverySubmit = async (
    payload: RecoveryRequestPayload
  ): Promise<RecoveryRequestResponse> => {
    return requestRecovery(payload);
  };

  return (
    <div className="space-y-6" data-testid="license-page">
      {/* Page Header */}
      <div className="flex flex-col sm:flex-row sm:items-center sm:justify-between gap-4">
        <div>
          <h1 className="text-2xl font-bold tracking-tight" data-testid="license-page-title">
            {t("license.pageTitle", "License & Authority Verification")}
          </h1>
          <p className="text-muted-foreground text-sm">
            {t(
              "license.pageSubtitle",
              "Manage Built Panel commercial subscription, cryptographic entitlement, and anti-tamper recovery."
            )}
          </p>
        </div>

        <div className="flex items-center gap-2">
          <Button
            onClick={() => setActivateOpen(true)}
            className="gap-2 shrink-0"
            data-testid="btn-open-activate-dialog"
          >
            <KeyRound className="h-4 w-4" />
            {t("license.enterKeyBtn", "Enter License Key")}
          </Button>

          <Button
            variant="outline"
            size="sm"
            onClick={loadLicense}
            disabled={isLoading || revalidating}
            className="gap-2 shrink-0"
            data-testid="btn-refresh-license-page"
          >
            <RefreshCw className={`h-4 w-4 ${isLoading ? "animate-spin" : ""}`} />
            {t("common.refresh", "Refresh")}
          </Button>
        </div>
      </div>

      {isError && (
        <div className="p-4 rounded-lg bg-red-50 text-red-800 dark:bg-red-950/40 dark:text-red-300 border border-red-200 flex items-center gap-3">
          <AlertCircle className="h-5 w-5 shrink-0" />
          <div className="flex-1 text-sm">
            {t("license.loadError", "Failed to retrieve license entitlement status.")}
          </div>
          <Button variant="outline" size="sm" onClick={loadLicense}>
            {t("common.retry", "Retry")}
          </Button>
        </div>
      )}

      {license && (
        <>
          {/* Status Alert Banner */}
          <LicenseStatusBanner
            license={license}
            onRevalidate={handleRevalidate}
            onOpenActivate={() => setActivateOpen(true)}
            onOpenRecovery={() => setRecoveryOpen(true)}
            revalidating={revalidating}
          />

          {/* Identity & Cryptographic Metadata */}
          <LicenseDetailsCard
            license={license}
            onOpenActivate={() => setActivateOpen(true)}
            onOpenRecovery={() => setRecoveryOpen(true)}
            onRevalidate={handleRevalidate}
            revalidating={revalidating}
          />

          {/* Capacity Limits & Feature Gates */}
          <LicenseEntitlementsCard
            features={license.features}
            usage={license.usage}
          />
        </>
      )}

      {/* Modals */}
      <ActivateLicenseDialog
        open={activateOpen}
        onClose={() => setActivateOpen(false)}
        onActivate={handleActivate}
      />

      <RecoveryRequestDialog
        open={recoveryOpen}
        onClose={() => setRecoveryOpen(false)}
        onSubmit={handleRecoverySubmit}
      />
    </div>
  );
}
