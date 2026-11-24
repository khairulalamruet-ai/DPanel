import React, { useState } from "react";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
  Button,
  Input,
} from "@builtpanel/ui";
import { useTranslation } from "@builtpanel/i18n";
import { KeyRound, ShieldCheck, AlertCircle } from "lucide-react";

interface ActivateLicenseDialogProps {
  open: boolean;
  onClose: () => void;
  onActivate: (licenseKey: string) => Promise<void>;
}

export function ActivateLicenseDialog({ open, onClose, onActivate }: ActivateLicenseDialogProps) {
  const { t } = useTranslation();

  const [licenseKey, setLicenseKey] = useState("");
  const [errorMsg, setErrorMsg] = useState("");
  const [isSubmitting, setIsSubmitting] = useState(false);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    const cleanKey = licenseKey.trim().toUpperCase();

    if (!cleanKey) {
      setErrorMsg(t("license.keyRequired", "Please enter a valid license key."));
      return;
    }

    if (!cleanKey.startsWith("DHP-")) {
      setErrorMsg(t("license.keyInvalidFormat", "License keys must start with 'DHP-' prefix."));
      return;
    }

    setErrorMsg("");
    setIsSubmitting(true);
    try {
      await onActivate(cleanKey);
      setLicenseKey("");
      onClose();
    } catch (err: any) {
      setErrorMsg(err.message || t("license.activationFailed", "Failed to activate license key."));
    } finally {
      setIsSubmitting(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={(val) => !val && onClose()}>
      <DialogContent data-testid="activate-license-dialog" className="sm:max-w-md">
        <DialogHeader>
          <div className="flex items-center gap-2">
            <div className="p-2 rounded-full bg-primary/10 text-primary">
              <KeyRound className="h-5 w-5" />
            </div>
            <div>
              <DialogTitle data-testid="activate-dialog-title">
                {t("license.activateModalTitle", "Activate License Key")}
              </DialogTitle>
              <DialogDescription className="text-xs">
                {t(
                  "license.activateModalSubtitle",
                  "Enter your issued Built Panel commercial license key to unlock features and node capacity."
                )}
              </DialogDescription>
            </div>
          </div>
        </DialogHeader>

        <form onSubmit={handleSubmit} className="space-y-4 py-2">
          {errorMsg && (
            <div
              className="p-3 bg-red-50 text-red-800 dark:bg-red-950/40 dark:text-red-300 rounded-lg text-xs flex items-center gap-2"
              data-testid="activate-error-message"
            >
              <AlertCircle className="h-4 w-4 shrink-0" />
              <span>{errorMsg}</span>
            </div>
          )}

          <div>
            <label className="text-xs font-semibold block mb-1.5" htmlFor="input-license-key">
              {t("license.licenseKeyInputLabel", "Commercial License Key")}
            </label>
            <Input
              id="input-license-key"
              type="text"
              placeholder="DHP-PRO-8492-9182-AA19"
              value={licenseKey}
              onChange={(e) => setLicenseKey(e.target.value)}
              className="font-mono text-xs uppercase tracking-wider"
              required
              data-testid="input-license-key"
              autoFocus
            />
            <p className="text-[11px] text-muted-foreground mt-1.5 flex items-center gap-1">
              <ShieldCheck className="h-3 w-3 text-emerald-500" />
              {t("license.activationNotice", "The key is exchanged with License Authority over TLS with proof-of-possession signature.")}
            </p>
          </div>

          <DialogFooter className="pt-2">
            <Button type="button" variant="outline" onClick={onClose} disabled={isSubmitting}>
              {t("common.cancel", "Cancel")}
            </Button>
            <Button type="submit" disabled={isSubmitting} data-testid="btn-submit-activate">
              {isSubmitting ? t("common.activating", "Activating...") : t("license.activateBtn", "Activate Key")}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
