import React, { useState } from "react";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogFooter,
  DialogTitle,
  DialogDescription,
  Button,
  Input,
  useToast,
} from "@builtpanel/ui";
import { useTranslation } from "@builtpanel/i18n";
import { apiFetch, getCsrfToken } from "@builtpanel/api";
import { KeyRound, ShieldPlus, RefreshCw } from "lucide-react";
import type { IssueLicensePayload, LaLicenseItem, LaPlanTier } from "./types";

export interface IssueLicenseDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onLicenseIssued?: (license: LaLicenseItem) => void;
}

export function IssueLicenseDialog({
  open,
  onOpenChange,
  onLicenseIssued,
}: IssueLicenseDialogProps) {
  const { t } = useTranslation();
  const { toast } = useToast();

  const [customerName, setCustomerName] = useState<string>("");
  const [customerEmail, setCustomerEmail] = useState<string>("");
  const [company, setCompany] = useState<string>("");
  const [planTier, setPlanTier] = useState<LaPlanTier>("enterprise");
  const [maxNodes, setMaxNodes] = useState<number>(5);
  const [maxTenants, setMaxTenants] = useState<number>(100);
  const [validityDays, setValidityDays] = useState<number>(365);
  const [maxActivations, setMaxActivations] = useState<number>(1);

  const [isSubmitting, setIsSubmitting] = useState<boolean>(false);
  const [error, setError] = useState<string | null>(null);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!customerName.trim() || !customerEmail.trim()) {
      setError("Customer name and email are required");
      return;
    }

    try {
      setIsSubmitting(true);
      setError(null);

      const payload: IssueLicensePayload = {
        customer_name: customerName.trim(),
        customer_email: customerEmail.trim(),
        company: company.trim() || undefined,
        plan_tier: planTier,
        max_nodes: maxNodes,
        max_tenants: maxTenants,
        validity_days: validityDays,
        max_activations: maxActivations,
      };

      const res = await apiFetch<{ success: boolean; license: LaLicenseItem }>(
        "/api/v1/la/licenses",
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
        title: t("la.issue.successToast"),
        variant: "default",
      });

      if (onLicenseIssued && res.license) {
        onLicenseIssued(res.license);
      }
      onOpenChange(false);
      resetForm();
    } catch (err: any) {
      setError(err?.message || "Failed to issue license");
    } finally {
      setIsSubmitting(false);
    }
  };

  const resetForm = () => {
    setCustomerName("");
    setCustomerEmail("");
    setCompany("");
    setPlanTier("enterprise");
    setMaxNodes(5);
    setMaxTenants(100);
    setValidityDays(365);
    setMaxActivations(1);
    setError(null);
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent onClose={() => onOpenChange(false)} className="max-w-lg">
        <DialogHeader>
          <div className="flex items-center gap-2 text-primary mb-1">
            <KeyRound className="h-5 w-5" />
            <DialogTitle>{t("la.issue.title")}</DialogTitle>
          </div>
          <DialogDescription>{t("la.issue.desc")}</DialogDescription>
        </DialogHeader>

        <form onSubmit={handleSubmit} className="space-y-4 my-2 text-xs">
          {error && (
            <div className="p-2.5 rounded-md bg-destructive/10 text-destructive text-xs">
              {error}
            </div>
          )}

          {/* Customer Details */}
          <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
            <div className="space-y-1">
              <label htmlFor="la-cust-name" className="font-medium text-foreground">
                {t("la.issue.nameLabel")}
              </label>
              <Input
                id="la-cust-name"
                data-testid="input-customer-name"
                placeholder="Acme Hosting Ltd."
                value={customerName}
                onChange={(e) => setCustomerName(e.target.value)}
                disabled={isSubmitting}
                required
                className="h-8 text-xs"
              />
            </div>
            <div className="space-y-1">
              <label htmlFor="la-cust-email" className="font-medium text-foreground">
                {t("la.issue.emailLabel")}
              </label>
              <Input
                id="la-cust-email"
                type="email"
                data-testid="input-customer-email"
                placeholder="billing@acme.com"
                value={customerEmail}
                onChange={(e) => setCustomerEmail(e.target.value)}
                disabled={isSubmitting}
                required
                className="h-8 text-xs"
              />
            </div>
          </div>

          <div className="space-y-1">
            <label htmlFor="la-company" className="font-medium text-foreground">
              {t("la.issue.companyLabel")}
            </label>
            <Input
              id="la-company"
              placeholder="Acme Global Networks"
              value={company}
              onChange={(e) => setCompany(e.target.value)}
              disabled={isSubmitting}
              className="h-8 text-xs"
            />
          </div>

          {/* Plan & Quotas */}
          <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
            <div className="space-y-1">
              <label htmlFor="la-plan-tier" className="font-medium text-foreground">
                {t("la.issue.planTierLabel")}
              </label>
              <select
                id="la-plan-tier"
                data-testid="select-plan-tier"
                value={planTier}
                onChange={(e) => setPlanTier(e.target.value as LaPlanTier)}
                disabled={isSubmitting}
                className="flex h-8 w-full rounded-md border border-input bg-background px-2.5 py-1 text-xs shadow-sm text-foreground"
              >
                <option value="starter">{t("la.plan.starter")}</option>
                <option value="business">{t("la.plan.business")}</option>
                <option value="enterprise">{t("la.plan.enterprise")}</option>
                <option value="unlimited">{t("la.plan.unlimited")}</option>
              </select>
            </div>

            <div className="space-y-1">
              <label htmlFor="la-validity" className="font-medium text-foreground">
                {t("la.issue.validityLabel")}
              </label>
              <select
                id="la-validity"
                data-testid="select-validity"
                value={validityDays}
                onChange={(e) => setValidityDays(Number(e.target.value))}
                disabled={isSubmitting}
                className="flex h-8 w-full rounded-md border border-input bg-background px-2.5 py-1 text-xs shadow-sm text-foreground"
              >
                <option value={30}>{t("la.validity.30days")}</option>
                <option value={365}>{t("la.validity.1year")}</option>
                <option value={0}>{t("la.validity.lifetime")}</option>
              </select>
            </div>
          </div>

          <div className="grid grid-cols-3 gap-3">
            <div className="space-y-1">
              <label htmlFor="la-max-nodes" className="font-medium text-foreground">
                {t("la.issue.maxNodesLabel")}
              </label>
              <Input
                id="la-max-nodes"
                type="number"
                min={1}
                max={100}
                value={maxNodes}
                onChange={(e) => setMaxNodes(Number(e.target.value))}
                disabled={isSubmitting}
                className="h-8 text-xs"
              />
            </div>
            <div className="space-y-1">
              <label htmlFor="la-max-tenants" className="font-medium text-foreground">
                {t("la.issue.maxTenantsLabel")}
              </label>
              <Input
                id="la-max-tenants"
                type="number"
                min={1}
                max={5000}
                value={maxTenants}
                onChange={(e) => setMaxTenants(Number(e.target.value))}
                disabled={isSubmitting}
                className="h-8 text-xs"
              />
            </div>
            <div className="space-y-1">
              <label htmlFor="la-max-act" className="font-medium text-foreground">
                {t("la.issue.maxActivationsLabel")}
              </label>
              <Input
                id="la-max-act"
                type="number"
                min={1}
                max={10}
                value={maxActivations}
                onChange={(e) => setMaxActivations(Number(e.target.value))}
                disabled={isSubmitting}
                className="h-8 text-xs"
              />
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
              type="submit"
              data-testid="btn-confirm-issue-license"
              disabled={isSubmitting}
              className="gap-1.5"
            >
              {isSubmitting ? (
                <RefreshCw className="h-4 w-4 animate-spin" />
              ) : (
                <ShieldPlus className="h-4 w-4" />
              )}
              {t("la.issue.submitBtn")}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
