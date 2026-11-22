import React, { useState, useMemo, useEffect } from "react";
import {
  Button,
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogFooter,
  Badge,
} from "@builtpanel/ui";
import { useTranslation } from "@builtpanel/i18n";
import { apiFetch } from "@builtpanel/api";
import { bulkCreateDomains } from "./adminApi";
import {
  Layers,
  CheckCircle2,
  AlertTriangle,
  RefreshCw,
  Sparkles,
  ExternalLink,
  ShieldCheck,
} from "lucide-react";
import { BulkCreateDomainsResponse } from "./types";

export interface BulkAddDomainsDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onSuccess: () => void;
}

export function BulkAddDomainsDialog({
  open,
  onOpenChange,
  onSuccess,
}: BulkAddDomainsDialogProps) {
  const { t } = useTranslation();

  const [tenantId, setTenantId] = useState("");
  const [owners, setOwners] = useState<{ id: string; username: string }[]>([]);
  useEffect(() => {
    apiFetch<{ items: { id: string; username: string; tenant?: unknown }[] }>("/api/v1/admin/users?limit=200")
      .then((res) => {
        const list = res.items.filter((u) => u.tenant).map((u) => ({ id: u.id, username: u.username }));
        setOwners(list);
        setTenantId((current) => current || list[0]?.id || "");
      })
      .catch(() => undefined);
  }, []);
    const [domainText, setDomainText] = useState("");
  const [enableSsl, setEnableSsl] = useState(true);

  const [isSubmitting, setIsSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Operation tracking
  const [activeOpId, setActiveOpId] = useState<string | null>(null);
  const [enqueuedResult, setEnqueuedResult] = useState<BulkCreateDomainsResponse | null>(null);


  // FQDN regex
  const fqdnRegex =
    /^(?:[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?\.)+[a-z0-9][a-z0-9-]{0,61}[a-z0-9]$/i;

  const { validDomains, invalidDomains } = useMemo(() => {
    const lines = domainText
      .split("\n")
      .map((l) => l.trim().toLowerCase())
      .filter((l) => l.length > 0);

    const valid: string[] = [];
    const invalid: string[] = [];

    for (const d of lines) {
      if (fqdnRegex.test(d)) {
        if (!valid.includes(d)) {
          valid.push(d);
        }
      } else {
        if (!invalid.includes(d)) {
          invalid.push(d);
        }
      }
    }

    return { validDomains: valid, invalidDomains: invalid };
  }, [domainText]);

  const handleFillSample = (count: number = 100) => {
    const samples: string[] = [];
    for (let i = 1; i <= count; i++) {
      samples.push(`bulk-site-${String(i).padStart(3, "0")}.example.com`);
    }
    setDomainText(samples.join("\n"));
    setError(null);
  };

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setError(null);

    if (validDomains.length === 0) {
      setError(
        t("adminDomains.bulk.noValidDomains", "Please enter at least one valid fully qualified domain name.")
      );
      return;
    }

    if (validDomains.length > 100) {
      setError(
        t("adminDomains.bulk.maxLimitError", "Maximum 100 domains can be queued in a single bulk operation.")
      );
      return;
    }

    try {
      setIsSubmitting(true);
      const res = await bulkCreateDomains(tenantId, validDomains);
      setEnqueuedResult({
        operation_id: "",
        total_requested: validDomains.length,
        enqueued_count: res.created.length,
        skipped_count: res.failed.length,
        skipped_domains: res.failed.map((f) => f.domain),
        status: "completed",
      });
      setActiveOpId("done");
      if (res.created.length > 0) onSuccess();
    } catch (err: any) {
      setError(err?.message || t("adminDomains.bulk.enqueueError", "Failed to enqueue bulk domain creation operation"));
    } finally {
      setIsSubmitting(false);
    }
  };

  const handleClose = () => {
    setActiveOpId(null);
    setEnqueuedResult(null);
    setDomainText("");
    setError(null);
    onOpenChange(false);
  };

  const isOpFinished = enqueuedResult !== null;
  const operation = enqueuedResult ? { status: enqueuedResult.enqueued_count > 0 ? "succeeded" : "failed" } : null;
  const isPolling = false;

  return (
    <Dialog open={open} onOpenChange={handleClose}>
      <DialogContent className="max-w-xl">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <Layers className="h-5 w-5 text-primary" />
            {t("adminDomains.bulk.title", "Bulk Add Domains (Queue)")}
          </DialogTitle>
        </DialogHeader>

        {activeOpId ? (
          /* Operation Progress View */
          <div className="py-4 space-y-4">
            <div className="p-4 rounded-xl border bg-muted/30 space-y-3">
              <div className="flex items-center justify-between">
                <div>
                  <p className="text-xs text-muted-foreground uppercase tracking-wider font-semibold">
                    {t("operations.idempotencyKey", "Operation ID")}
                  </p>
                  <p className="font-mono text-sm font-semibold">{activeOpId}</p>
                </div>
                <div>
                  {operation?.status === "succeeded" ? (
                    <Badge variant="default" className="bg-emerald-600 text-white gap-1" data-testid="bulk-op-succeeded-badge">
                      <CheckCircle2 className="h-3.5 w-3.5" />
                      {t("operations.status.succeeded", "Succeeded")}
                    </Badge>
                  ) : operation?.status === "failed" ? (
                    <Badge variant="destructive" className="gap-1">
                      <AlertTriangle className="h-3.5 w-3.5" />
                      {t("operations.status.failed", "Failed")}
                    </Badge>
                  ) : (
                    <Badge variant="secondary" className="gap-1 animate-pulse">
                      <RefreshCw className="h-3.5 w-3.5 animate-spin text-primary" />
                      {operation?.status || t("operations.status.pending", "Queued")}
                    </Badge>
                  )}
                </div>
              </div>

              {/* Progress text */}
              <div className="space-y-1">
                <div className="flex justify-between text-xs text-muted-foreground">
                  <span>
                    {t("adminDomains.bulk.provisioningCount", {
                      count: enqueuedResult?.enqueued_count || validDomains.length,
                    })}
                  </span>
                  <span>{operation?.status === "succeeded" ? "100%" : isPolling ? "50%" : "20%"}</span>
                </div>
                <div className="h-2 w-full bg-secondary rounded-full overflow-hidden">
                  <div
                    className={`h-full transition-all duration-500 ${
                      operation?.status === "succeeded"
                        ? "bg-emerald-500 w-full"
                        : "bg-primary w-1/2 animate-pulse"
                    }`}
                  />
                </div>
              </div>

              {operation?.status === "succeeded" && (
                <div className="p-3 bg-emerald-500/10 border border-emerald-500/20 text-emerald-800 dark:text-emerald-300 rounded-lg text-xs">
                  <p className="font-semibold">
                    {t(
                      "adminDomains.bulk.successNotice",
                      "Successfully registered {{count}} domains across virtual hosts and DNS zone templates.",
                      { count: enqueuedResult?.enqueued_count || validDomains.length }
                    )}
                  </p>
                </div>
              )}
            </div>

            <DialogFooter>
              <Button onClick={handleClose} disabled={!isOpFinished} data-testid="btn-done-bulk-domains">
                {t("common.done", "Done & Close")}
              </Button>
            </DialogFooter>
          </div>
        ) : (
          /* Bulk Input Form */
          <form onSubmit={handleSubmit} className="space-y-4 py-2 text-sm">
            {error && (
              <div className="p-3 text-xs bg-destructive/10 border border-destructive/20 text-destructive rounded-lg font-medium">
                {error}
              </div>
            )}

            <div className="grid grid-cols-1 sm:grid-cols-1 gap-3">
              {/* Target Tenant */}
              <div className="space-y-1.5">
                <label htmlFor="bulk-tenant" className="text-xs font-semibold uppercase text-muted-foreground">
                  {t("adminDomains.owner", "Target Tenant / Owner")}
                </label>
                <select
                  id="bulk-tenant"
                  value={tenantId}
                  onChange={(e) => setTenantId(e.target.value)}
                  className="flex h-9 w-full rounded-md border border-input bg-background px-3 py-1 text-sm shadow-sm transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                >
                  {owners.map((o) => (
                    <option key={o.id} value={o.id}>
                      {o.username}
                    </option>
                  ))}
                </select>
              </div>
            </div>

            {/* Domains Textarea */}
            <div className="space-y-1.5">
              <div className="flex items-center justify-between">
                <label htmlFor="bulk-domains-textarea" className="text-xs font-semibold uppercase text-muted-foreground">
                  {t("adminDomains.bulk.domainsListLabel", "Domain Names (1 per line, max 100)")}
                </label>
                <Button
                  type="button"
                  variant="ghost"
                  size="sm"
                  onClick={() => handleFillSample(100)}
                  className="h-7 text-xs text-primary gap-1"
                  data-testid="btn-fill-100-samples"
                >
                  <Sparkles className="h-3 w-3" />
                  {t("adminDomains.bulk.sample100", "Sample 100 domains")}
                </Button>
              </div>

              <textarea
                id="bulk-domains-textarea"
                data-testid="textarea-bulk-domains"
                rows={7}
                value={domainText}
                onChange={(e) => setDomainText(e.target.value)}
                placeholder="example1.com&#10;shop.example.com&#10;api.mysite.org"
                className="w-full rounded-md border border-input bg-background p-2.5 font-mono text-xs shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
              />

              {/* Parsed Counters */}
              <div className="flex items-center justify-between text-xs text-muted-foreground">
                <span className={validDomains.length > 100 ? "text-destructive font-semibold" : ""}>
                  {t("adminDomains.bulk.validCount", "Valid FQDNs:")}{" "}
                  <strong className="text-foreground">{validDomains.length}</strong> / 100
                </span>
                {invalidDomains.length > 0 && (
                  <span className="text-destructive">
                    {invalidDomains.length} {t("adminDomains.bulk.invalidSyntax", "invalid format")}
                  </span>
                )}
              </div>
            </div>

            {/* SSL Checkbox */}
            <div className="flex items-center gap-2 pt-1">
              <input
                type="checkbox"
                id="bulk-ssl-toggle"
                checked={enableSsl}
                onChange={(e) => setEnableSsl(e.target.checked)}
                className="rounded border-input text-primary focus:ring-primary h-4 w-4"
              />
              <label htmlFor="bulk-ssl-toggle" className="cursor-pointer text-xs flex items-center gap-1.5 font-medium">
                <ShieldCheck className="h-4 w-4 text-emerald-500" />
                {t("adminDomains.bulk.autoSsl", "Issue automatic Let's Encrypt SSL certificates")}
              </label>
            </div>

            <DialogFooter className="pt-2">
              <Button
                type="button"
                variant="outline"
                onClick={handleClose}
                disabled={isSubmitting}
              >
                {t("common.cancel", "Cancel")}
              </Button>
              <Button
                type="submit"
                disabled={isSubmitting || validDomains.length === 0 || validDomains.length > 100}
                data-testid="btn-submit-bulk-domains"
              >
                {isSubmitting
                  ? t("common.processing", "Processing...")
                  : t("adminDomains.bulk.submitBtn", { count: validDomains.length })}
              </Button>
            </DialogFooter>
          </form>
        )}
      </DialogContent>
    </Dialog>
  );
}
