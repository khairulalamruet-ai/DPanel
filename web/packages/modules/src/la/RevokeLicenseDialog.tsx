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
import { AlertOctagon, RefreshCw } from "lucide-react";
import type { LaLicenseItem, RevokeLicensePayload } from "./types";

export interface RevokeLicenseDialogProps {
  license: LaLicenseItem | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onLicenseRevoked?: (revokedLicense: LaLicenseItem) => void;
}

export function RevokeLicenseDialog({
  license,
  open,
  onOpenChange,
  onLicenseRevoked,
}: RevokeLicenseDialogProps) {
  const { t } = useTranslation();
  const { toast } = useToast();

  const [reason, setReason] = useState<string>("Terms of service violation");
  const [isSubmitting, setIsSubmitting] = useState<boolean>(false);
  const [error, setError] = useState<string | null>(null);

  if (!license) return null;

  const handleRevoke = async () => {
    try {
      setIsSubmitting(true);
      setError(null);

      const payload: RevokeLicensePayload = {
        reason: reason.trim() || "Administrative revocation",
      };

      const res = await apiFetch<{ success: boolean; license: LaLicenseItem }>(
        `/api/v1/la/licenses/${license.id}/revoke`,
        {
          method: "POST",
          headers: {
            "Content-Type": "application/json",
            "X-CSRF-Token": getCsrfToken() || "",
          },
          body: JSON.stringify(payload),
        }
      );

      toast({
        title: t("la.revoke.successToast"),
        variant: "destructive",
      });

      if (onLicenseRevoked && res.license) {
        onLicenseRevoked(res.license);
      }
      onOpenChange(false);
    } catch (err: any) {
      setError(err?.message || "Failed to revoke license");
    } finally {
      setIsSubmitting(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent onClose={() => onOpenChange(false)} className="max-w-md">
        <DialogHeader>
          <div className="flex items-center gap-2 text-destructive mb-1">
            <AlertOctagon className="h-5 w-5" />
            <DialogTitle>{t("la.revoke.title")}</DialogTitle>
          </div>
          <DialogDescription>{t("la.revoke.desc")}</DialogDescription>
        </DialogHeader>

        <div className="space-y-3 my-2 text-xs">
          {error && (
            <div className="p-2.5 rounded-md bg-destructive/10 text-destructive text-xs">
              {error}
            </div>
          )}

          {/* License Summary Box */}
          <div className="p-3 bg-muted/30 border rounded-lg space-y-1.5 font-mono text-[11px]">
            <div className="flex justify-between">
              <span className="text-muted-foreground">Key:</span>
              <span className="font-bold text-foreground">{license.license_key}</span>
            </div>
            <div className="flex justify-between">
              <span className="text-muted-foreground">Customer:</span>
              <span className="text-foreground">{license.customer_name}</span>
            </div>
            <div className="flex justify-between">
              <span className="text-muted-foreground">Plan Tier:</span>
              <Badge variant="outline" className="text-[10px] uppercase font-sans">
                {license.plan_tier}
              </Badge>
            </div>
          </div>

          {/* Reason Selector */}
          <div className="space-y-1">
            <label htmlFor="revoke-reason" className="font-medium text-foreground">
              {t("la.revoke.reasonLabel")}
            </label>
            <select
              id="revoke-reason"
              data-testid="select-revoke-reason"
              value={reason}
              onChange={(e) => setReason(e.target.value)}
              disabled={isSubmitting}
              className="flex h-8 w-full rounded-md border border-input bg-background px-2.5 py-1 text-xs shadow-sm text-foreground"
            >
              <option value="Terms of service violation">
                {t("la.revoke.reasons.tos")}
              </option>
              <option value="Payment default or chargeback">
                {t("la.revoke.reasons.payment")}
              </option>
              <option value="Customer requested cancellation">
                {t("la.revoke.reasons.cancellation")}
              </option>
              <option value="Cryptographic key compromise">
                {t("la.revoke.reasons.compromise")}
              </option>
            </select>
          </div>
        </div>

        <DialogFooter className="pt-2 border-t gap-2">
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
            data-testid="btn-confirm-revoke-license"
            disabled={isSubmitting}
            onClick={handleRevoke}
            className="gap-1.5"
          >
            {isSubmitting ? (
              <RefreshCw className="h-4 w-4 animate-spin" />
            ) : (
              <AlertOctagon className="h-4 w-4" />
            )}
            {t("la.revoke.submitBtn")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
