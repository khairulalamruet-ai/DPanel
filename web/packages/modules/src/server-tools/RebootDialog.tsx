import React, { useState } from "react";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogFooter,
  Button,
  Input,
} from "@builtpanel/ui";
import { AlertTriangle, Lock, RefreshCw } from "lucide-react";
import { useTranslation } from "@builtpanel/i18n";

interface RebootDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  nodeHostname: string;
  /** What has to be typed to confirm: the backend wants the node's name as it is. */
  confirmWord?: string;
  onConfirm: (sudoPassword: string) => Promise<void>;
}

export function RebootDialog({
  open,
  onOpenChange,
  nodeHostname,
  confirmWord = "REBOOT",
  onConfirm,
}: RebootDialogProps) {
  const { t } = useTranslation();
  const [confirmText, setConfirmText] = useState("");
  const [sudoPassword, setSudoPassword] = useState("");
  const [isSubmitting, setIsSubmitting] = useState(false);

  React.useEffect(() => {
    if (open) {
      setConfirmText("");
      setSudoPassword("");
    }
  }, [open]);

  const isValid = confirmText.trim() === confirmWord && sudoPassword.trim().length > 0;

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!isValid) return;
    try {
      setIsSubmitting(true);
      await onConfirm(sudoPassword);
      onOpenChange(false);
    } finally {
      setIsSubmitting(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-md">
        <form onSubmit={handleSubmit} className="space-y-4">
          <DialogHeader>
            <DialogTitle className="flex items-center gap-2 text-destructive">
              <AlertTriangle className="h-5 w-5" />
              <span data-testid="reboot-dialog-title">{t("serverTools.rebootTitle")}</span>
            </DialogTitle>
          </DialogHeader>

          <div className="rounded-md border border-destructive/20 bg-destructive/10 p-3 text-sm text-destructive">
            <p className="font-semibold">{t("serverTools.rebootWarningHeading")}</p>
            <p className="mt-1 text-xs opacity-90">
              {t("serverTools.rebootWarningText", { hostname: nodeHostname })}
            </p>
          </div>

          <div className="space-y-3">
            <div>
              <label className="text-xs font-medium text-foreground">
                {t("serverTools.typeRebootConfirm")}
              </label>
              <Input
                data-testid="input-reboot-confirm"
                value={confirmText}
                onChange={(e) => setConfirmText(e.target.value)}
                placeholder={confirmWord}
                className="mt-1 font-mono text-sm uppercase"
                autoComplete="off"
              />
            </div>

            <div>
              <label className="flex items-center gap-1.5 text-xs font-medium text-foreground">
                <Lock className="h-3.5 w-3.5 text-amber-500" />
                <span>{t("serverTools.sudoPasswordLabel")}</span>
              </label>
              <Input
                data-testid="input-reboot-sudo"
                type="password"
                value={sudoPassword}
                onChange={(e) => setSudoPassword(e.target.value)}
                placeholder={t("serverTools.sudoPasswordPlaceholder")}
                className="mt-1 text-sm"
                autoComplete="current-password"
              />
            </div>
          </div>

          <div className="rounded border bg-muted/30 p-2 text-xs text-muted-foreground">
            {t("serverTools.auditNotice")}
          </div>

          <DialogFooter className="flex justify-end gap-2">
            <Button
              type="button"
              variant="outline"
              onClick={() => onOpenChange(false)}
              disabled={isSubmitting}
            >
              {t("common.cancel")}
            </Button>
            <Button
              type="submit"
              variant="destructive"
              disabled={!isValid || isSubmitting}
              data-testid="btn-confirm-reboot"
              className="gap-1.5"
            >
              {isSubmitting ? (
                <RefreshCw className="h-4 w-4 animate-spin" />
              ) : (
                <RefreshCw className="h-4 w-4" />
              )}
              {t("serverTools.executeReboot")}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
