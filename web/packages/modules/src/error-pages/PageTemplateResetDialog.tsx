import React from "react";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
  Button,
} from "@builtpanel/ui";
import { AlertTriangle } from "lucide-react";
import { useTranslation } from "@builtpanel/i18n";

interface PageTemplateResetDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  templateTitle: string;
  onConfirm: () => void;
  isLoading: boolean;
}

export function PageTemplateResetDialog({
  open,
  onOpenChange,
  templateTitle,
  onConfirm,
  isLoading,
}: PageTemplateResetDialogProps) {
  const { t } = useTranslation();

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent onClose={() => onOpenChange(false)}>
        <DialogHeader>
          <div className="flex items-center gap-3">
            <div className="rounded-full bg-destructive/10 p-2 text-destructive">
              <AlertTriangle className="h-5 w-5" />
            </div>
            <DialogTitle>{t("errorPages.dialog.resetTitle")}</DialogTitle>
          </div>
          <DialogDescription className="mt-2 text-sm text-muted-foreground">
            {t("errorPages.dialog.resetDescription", { template: templateTitle })}
          </DialogDescription>
        </DialogHeader>

        <DialogFooter className="mt-6 flex justify-end gap-3">
          <Button
            variant="outline"
            onClick={() => onOpenChange(false)}
            disabled={isLoading}
            data-testid="btn-cancel-reset"
          >
            {t("common.cancel")}
          </Button>
          <Button
            variant="destructive"
            onClick={onConfirm}
            disabled={isLoading}
            data-testid="btn-confirm-reset"
          >
            {isLoading ? t("common.resetting") : t("errorPages.actions.confirmReset")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
