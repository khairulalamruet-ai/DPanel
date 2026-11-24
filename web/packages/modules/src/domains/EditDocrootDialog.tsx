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
import { Folder } from "lucide-react";
import { ClientDomainItem } from "./types";

export interface EditDocrootDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  domain: ClientDomainItem | null;
  onSubmit: (domainId: string, docroot: string) => Promise<void>;
}

export function EditDocrootDialog({
  open,
  onOpenChange,
  domain,
  onSubmit,
}: EditDocrootDialogProps) {
  const { t } = useTranslation();
  const [docroot, setDocroot] = useState("");
  const [error, setError] = useState("");
  const [isSubmitting, setIsSubmitting] = useState(false);

  useEffect(() => {
    if (domain) {
      setDocroot(domain.docroot || "");
      setError("");
    }
  }, [domain, open]);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!domain) return;

    const trimmed = docroot.trim();
    if (!trimmed) {
      setError(t("domains.validation.docrootRequired", "Document root path is required"));
      return;
    }
    if (!trimmed.startsWith("/")) {
      setError(t("domains.validation.docrootAbsolute", "Document root must start with /"));
      return;
    }

    try {
      setIsSubmitting(true);
      setError("");
      await onSubmit(domain.id, trimmed);
      onOpenChange(false);
    } catch (err: any) {
      setError(err?.message || t("domains.errors.updateDocrootFailed", "Failed to update document root"));
    } finally {
      setIsSubmitting(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <Folder className="h-5 w-5 text-primary" />
            {t("domains.editDocroot.title", "Edit Document Root")}
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

          <div className="space-y-1.5">
            <label htmlFor="docroot-input" className="text-sm font-medium leading-none">
              {t("domains.docroot", "Document Root")}
            </label>
            <Input
              id="docroot-input"
              data-testid="input-edit-docroot"
              value={docroot}
              onChange={(e) => setDocroot(e.target.value)}
              placeholder="/domains/example.com/public_html"
              className="font-mono text-sm"
            />
            <p className="text-xs text-muted-foreground">
              {t(
                "domains.editDocroot.help",
                "Relative to home directory. Files inside this folder are served publicly."
              )}
            </p>
          </div>

          <DialogFooter className="pt-2">
            <Button
              type="button"
              variant="outline"
              onClick={() => onOpenChange(false)}
              disabled={isSubmitting}
            >
              {t("common.cancel", "Cancel")}
            </Button>
            <Button type="submit" disabled={isSubmitting} data-testid="btn-save-docroot">
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
