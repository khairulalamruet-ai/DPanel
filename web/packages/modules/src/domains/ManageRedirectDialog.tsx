import React, { useState, useEffect } from "react";
import {
  Button,
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogFooter,
  Input,
} from "@builtpanel/ui";
import { useTranslation } from "@builtpanel/i18n";
import { ArrowRightLeft } from "lucide-react";
import { ClientDomainItem, UpdateRedirectPayload } from "./types";

export interface ManageRedirectDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  domain: ClientDomainItem | null;
  onSubmit: (domainId: string, payload: UpdateRedirectPayload) => Promise<void>;
}

export function ManageRedirectDialog({
  open,
  onOpenChange,
  domain,
  onSubmit,
}: ManageRedirectDialogProps) {
  const { t } = useTranslation();
  const [enabled, setEnabled] = useState(false);
  const [targetUrl, setTargetUrl] = useState("");
  const [statusCode, setStatusCode] = useState<301 | 302>(301);
  const [error, setError] = useState("");
  const [isSubmitting, setIsSubmitting] = useState(false);

  useEffect(() => {
    if (domain) {
      if (domain.redirect && domain.redirect.enabled) {
        setEnabled(true);
        setTargetUrl(domain.redirect.target_url || "");
        setStatusCode(domain.redirect.status_code || 301);
      } else {
        setEnabled(false);
        setTargetUrl("");
        setStatusCode(301);
      }
      setError("");
    }
  }, [domain, open]);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!domain) return;

    if (enabled) {
      const trimmed = targetUrl.trim();
      if (!trimmed) {
        setError(t("domains.validation.redirectUrlRequired", "Target redirect URL is required"));
        return;
      }
      try {
        const parsed = new URL(trimmed);
        if (parsed.protocol !== "http:" && parsed.protocol !== "https:") {
          setError(t("domains.validation.redirectUrlProtocol", "URL must start with http:// or https://"));
          return;
        }
        if (parsed.hostname.toLowerCase() === domain.name.toLowerCase()) {
          setError(t("domains.validation.redirectUrlSelf", "Cannot redirect a domain to itself"));
          return;
        }
      } catch {
        setError(t("domains.validation.redirectUrlInvalid", "Please enter a valid URL"));
        return;
      }
    }

    try {
      setIsSubmitting(true);
      setError("");
      await onSubmit(domain.id, {
        enabled,
        target_url: enabled ? targetUrl.trim() : "",
        status_code: statusCode,
      });
      onOpenChange(false);
    } catch (err: any) {
      setError(err?.message || t("domains.errors.updateRedirectFailed", "Failed to update redirection"));
    } finally {
      setIsSubmitting(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <ArrowRightLeft className="h-5 w-5 text-primary" />
            {t("domains.redirect.title", "Manage Redirection")}
          </DialogTitle>
        </DialogHeader>

        <form onSubmit={handleSubmit} className="space-y-4 py-2">
          {error && (
            <div className="p-3 text-sm rounded bg-destructive/15 text-destructive border border-destructive/20">
              {error}
            </div>
          )}

          <div>
            <label className="text-sm font-medium text-muted-foreground block">
              {t("domains.domainName", "Domain")}
            </label>
            <p className="font-mono text-sm font-semibold mt-1">{domain?.name}</p>
          </div>

          <div className="flex items-center space-x-2 pt-1">
            <input
              type="checkbox"
              id="redirect-toggle"
              data-testid="checkbox-manage-redirect-toggle"
              checked={enabled}
              onChange={(e) => setEnabled(e.target.checked)}
              className="h-4 w-4 rounded border-input text-primary focus:ring-primary"
            />
            <label htmlFor="redirect-toggle" className="cursor-pointer font-medium text-sm">
              {t("domains.redirect.enable", "Redirect visitors to another URL")}
            </label>
          </div>

          {enabled && (
            <div className="space-y-3 pl-6 border-l-2 border-primary/30 ml-2">
              <div className="space-y-1.5">
                <label htmlFor="target-url-input" className="text-sm font-medium leading-none">
                  {t("domains.redirect.targetUrl", "Target URL")}
                </label>
                <Input
                  id="target-url-input"
                  data-testid="input-manage-redirect-url"
                  value={targetUrl}
                  onChange={(e) => setTargetUrl(e.target.value)}
                  placeholder="https://example.com"
                  className="font-mono text-sm"
                />
              </div>

              <div className="space-y-1.5">
                <label htmlFor="redirect-type" className="text-sm font-medium leading-none">
                  {t("domains.redirect.type", "Redirect Type")}
                </label>
                <select
                  id="redirect-type"
                  value={String(statusCode)}
                  onChange={(e) => setStatusCode(Number(e.target.value) as 301 | 302)}
                  className="flex h-9 w-full rounded-md border border-input bg-background px-3 py-1 text-sm shadow-sm transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                >
                  <option value="301">
                    301 — {t("domains.redirect.permanent", "Permanent (SEO friendly)")}
                  </option>
                  <option value="302">
                    302 — {t("domains.redirect.temporary", "Temporary")}
                  </option>
                </select>
              </div>
            </div>
          )}

          <DialogFooter className="pt-2">
            <Button
              type="button"
              variant="outline"
              onClick={() => onOpenChange(false)}
              disabled={isSubmitting}
            >
              {t("common.cancel", "Cancel")}
            </Button>
            <Button type="submit" disabled={isSubmitting} data-testid="btn-save-redirect">
              {isSubmitting
                ? t("common.saving", "Saving...")
                : t("common.save", "Save Changes")}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
