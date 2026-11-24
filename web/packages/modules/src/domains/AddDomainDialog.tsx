import React, { useState } from "react";
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
import { Globe, ShieldCheck } from "lucide-react";
import { DomainType, CreateDomainPayload } from "./types";

export interface AddDomainDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onSubmit: (payload: CreateDomainPayload) => Promise<void>;
  primaryDomain?: string;
  isLimitReached?: boolean;
}

export function AddDomainDialog({
  open,
  onOpenChange,
  onSubmit,
  isLimitReached,
}: AddDomainDialogProps) {
  const { t } = useTranslation();

  const [name, setName] = useState("");
  const [type, setType] = useState<DomainType>("addon");
  const [docroot, setDocroot] = useState("");
  const [isRedirectEnabled, setIsRedirectEnabled] = useState(false);
  const [targetUrl, setTargetUrl] = useState("");
  const [statusCode, setStatusCode] = useState<301 | 302>(301);
  const [enableSsl, setEnableSsl] = useState(true);

  const [error, setError] = useState("");
  const [isSubmitting, setIsSubmitting] = useState(false);

  // Auto-generate docroot when name changes
  const handleNameChange = (val: string) => {
    const clean = val.trim().toLowerCase();
    setName(clean);
    if (clean && !isRedirectEnabled) {
      setDocroot(`/domains/${clean}/public_html`);
    } else if (!clean) {
      setDocroot("");
    }
  };

  const validate = (): boolean => {
    setError("");

    if (!name.trim()) {
      setError(t("domains.validation.nameRequired", "Domain name is required"));
      return false;
    }

    // FQDN syntax check: hostname letters/digits/hyphens separated by dots
    const fqdnRegex =
      /^(?:[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?\.)+[a-z0-9][a-z0-9-]{0,61}[a-z0-9]$/i;
    if (!fqdnRegex.test(name.trim())) {
      setError(
        t(
          "domains.validation.invalidFqdn",
          "Invalid domain format. Enter a valid fully qualified domain name (e.g. example.com or app.example.com)"
        )
      );
      return false;
    }

    if (isRedirectEnabled) {
      if (!targetUrl.trim()) {
        setError(
          t(
            "domains.validation.redirectUrlRequired",
            "Target redirect URL is required when redirection is enabled"
          )
        );
        return false;
      }
      try {
        const url = new URL(targetUrl.trim());
        if (url.protocol !== "http:" && url.protocol !== "https:") {
          setError(
            t(
              "domains.validation.redirectUrlProtocol",
              "Redirect URL must start with http:// or https://"
            )
          );
          return false;
        }
      } catch {
        setError(
          t("domains.validation.redirectUrlInvalid", "Please enter a valid target URL")
        );
        return false;
      }
    } else {
      if (!docroot.trim()) {
        setError(
          t("domains.validation.docrootRequired", "Document root directory is required")
        );
        return false;
      }
      if (!docroot.startsWith("/")) {
        setError(
          t(
            "domains.validation.docrootAbsolute",
            "Document root path must be absolute (start with /)"
          )
        );
        return false;
      }
    }

    return true;
  };

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!validate()) return;

    try {
      setIsSubmitting(true);
      await onSubmit({
        name: name.trim().toLowerCase(),
        type,
        docroot: isRedirectEnabled ? undefined : docroot.trim(),
        redirect: isRedirectEnabled
          ? {
              enabled: true,
              target_url: targetUrl.trim(),
              status_code: statusCode,
            }
          : undefined,
        enable_ssl: enableSsl,
      });

      // Reset form
      setName("");
      setType("addon");
      setDocroot("");
      setIsRedirectEnabled(false);
      setTargetUrl("");
      setStatusCode(301);
      setError("");
      onOpenChange(false);
    } catch (err: any) {
      setError(err?.message || t("domains.toast.createError", "Failed to create domain"));
    } finally {
      setIsSubmitting(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-lg">
        <form onSubmit={handleSubmit}>
          <DialogHeader>
            <DialogTitle className="flex items-center gap-2">
              <Globe className="h-5 w-5 text-primary" />
              {t("domains.dialog.createTitle", "Add New Domain")}
            </DialogTitle>
          </DialogHeader>

          <div className="py-4 space-y-4 text-sm">
            {isLimitReached && (
              <div className="p-3 text-xs bg-amber-500/10 border border-amber-500/30 text-amber-800 dark:text-amber-300 rounded-lg">
                {t("domains.limitReachedWarning", "You have reached your domain quota limit.")}
              </div>
            )}

            {error && (
              <div className="p-3 text-xs bg-destructive/10 border border-destructive/20 text-destructive rounded-lg font-medium">
                {error}
              </div>
            )}

            {/* Domain Name */}
            <div className="space-y-1.5">
              <label htmlFor="domain-name" className="text-sm font-medium leading-none">
                {t("domains.dialog.nameLabel", "Domain Name")}
              </label>
              <Input
                id="domain-name"
                data-testid="input-domain-name"
                placeholder={t("domains.dialog.namePlaceholder", "e.g. mynewsite.com or blog.mysite.com")}
                value={name}
                onChange={(e) => handleNameChange(e.target.value)}
                disabled={isSubmitting || isLimitReached}
                autoFocus
              />
              <p className="text-[11px] text-muted-foreground">
                {t("domains.dialog.nameHelp", "Enter the fully-qualified domain name (FQDN).")}
              </p>
            </div>

            {/* Domain Type */}
            <div className="space-y-1.5">
              <label htmlFor="domain-type" className="text-sm font-medium leading-none">
                {t("domains.dialog.typeLabel", "Domain Type")}
              </label>
              <select
                id="domain-type"
                data-testid="select-domain-type"
                value={type}
                onChange={(e) => setType(e.target.value as DomainType)}
                disabled={isSubmitting || isLimitReached}
                className="flex h-9 w-full rounded-md border border-input bg-background px-3 py-1 text-sm shadow-sm transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50"
              >
                <option value="addon">{t("domains.types.addon", "Addon Domain (Independent website)")}</option>
                <option value="subdomain">{t("domains.types.subdomain", "Subdomain (e.g. sub.example.com)")}</option>
                <option value="alias">{t("domains.types.alias", "Domain Alias (Points to primary domain)")}</option>
              </select>
              <p className="text-[11px] text-muted-foreground">
                {type === "addon"
                  ? t("domains.types.addonHelp", "Separate website with its own files and document root.")
                  : type === "subdomain"
                  ? t("domains.types.subdomainHelp", "A branch of your existing domain.")
                  : t("domains.types.aliasHelp", "Parks this domain on top of an existing website.")}
              </p>
            </div>

            {/* Redirection Toggle */}
            <div className="pt-2 border-t space-y-3">
              <div className="flex items-center gap-2">
                <input
                  type="checkbox"
                  id="enable-redirect"
                  data-testid="checkbox-enable-redirect"
                  className="rounded border-input text-primary focus:ring-primary"
                  checked={isRedirectEnabled}
                  onChange={(e) => setIsRedirectEnabled(e.target.checked)}
                  disabled={isSubmitting || isLimitReached}
                />
                <label htmlFor="enable-redirect" className="cursor-pointer font-medium text-sm">
                  {t("domains.dialog.redirectToggle", "Redirect this domain to another URL")}
                </label>
              </div>

              {isRedirectEnabled ? (
                <div className="space-y-3 pl-6 border-l-2 border-primary/30">
                  <div className="space-y-1">
                    <label htmlFor="target-url" className="text-sm font-medium leading-none">
                      {t("domains.dialog.targetUrlLabel", "Redirect Destination (Target URL)")}
                    </label>
                    <Input
                      id="target-url"
                      data-testid="input-target-url"
                      placeholder="https://destination.com"
                      value={targetUrl}
                      onChange={(e) => setTargetUrl(e.target.value)}
                      disabled={isSubmitting}
                    />
                  </div>

                  <div className="space-y-1">
                    <label htmlFor="status-code" className="text-sm font-medium leading-none">
                      {t("domains.dialog.statusCodeLabel", "Redirect Type")}
                    </label>
                    <select
                      id="status-code"
                      value={String(statusCode)}
                      onChange={(e) => setStatusCode(Number(e.target.value) as 301 | 302)}
                      disabled={isSubmitting}
                      className="flex h-9 w-full rounded-md border border-input bg-background px-3 py-1 text-sm shadow-sm transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50"
                    >
                      <option value="301">301 — {t("domains.redirects.permanent301", "Permanent (SEO friendly)")}</option>
                      <option value="302">302 — {t("domains.redirects.temporary302", "Temporary")}</option>
                    </select>
                  </div>
                </div>
              ) : (
                /* Document Root (when not redirecting) */
                <div className="space-y-1.5">
                  <label htmlFor="docroot" className="text-sm font-medium leading-none">
                    {t("domains.dialog.docrootLabel", "Document Root Path")}
                  </label>
                  <Input
                    id="docroot"
                    data-testid="input-docroot"
                    placeholder="/domains/example.com/public_html"
                    value={docroot}
                    onChange={(e) => setDocroot(e.target.value)}
                    disabled={isSubmitting || isLimitReached}
                  />
                  <p className="text-[11px] text-muted-foreground">
                    {t("domains.dialog.docrootHelp", "Directory where website files will be located.")}
                  </p>
                </div>
              )}
            </div>

            {/* Auto SSL Notice */}
            <div className="p-3 bg-muted/40 rounded-lg flex items-start gap-2.5 text-xs text-muted-foreground">
              <ShieldCheck className="h-4 w-4 text-emerald-500 shrink-0 mt-0.5" />
              <div>
                <span className="font-semibold text-foreground block">
                  {t("domains.dialog.autoSslTitle", "Free Automatic SSL Included")}
                </span>
                {t(
                  "domains.dialog.autoSslDesc",
                  "Let's Encrypt SSL certificates are issued and renewed automatically once DNS resolves."
                )}
              </div>
            </div>
          </div>

          <DialogFooter className="gap-2 sm:gap-0">
            <Button
              type="button"
              variant="outline"
              onClick={() => onOpenChange(false)}
              disabled={isSubmitting}
            >
              {t("common.cancel", "Cancel")}
            </Button>
            <Button
              type="submit"
              data-testid="btn-submit-domain"
              disabled={isSubmitting || isLimitReached || !name.trim()}
            >
              {isSubmitting ? t("common.saving", "Saving...") : t("domains.dialog.createSubmit", "Add Domain")}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
