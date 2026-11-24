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
import { Ban, Plus } from "lucide-react";
import { useTranslation } from "@builtpanel/i18n";

interface AddGlobalBanDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onConfirm: (ip: string, reason: string, durationHours: number) => Promise<void>;
}

export function AddGlobalBanDialog({ open, onOpenChange, onConfirm }: AddGlobalBanDialogProps) {
  const { t } = useTranslation();
  const [ip, setIp] = useState("");
  const [isSubmitting, setIsSubmitting] = useState(false);

  React.useEffect(() => {
    if (open) {
      setIp("");
    }
  }, [open]);

  const isValid = ip.trim().length > 6;

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!isValid) return;
    try {
      setIsSubmitting(true);
      await onConfirm(ip.trim(), "", 0);
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
            <DialogTitle className="flex items-center gap-2 text-base font-semibold text-destructive">
              <Ban className="h-5 w-5" />
              <span data-testid="add-global-ban-title">{t("firewall.addGlobalBanTitle")}</span>
            </DialogTitle>
          </DialogHeader>

          <div className="space-y-3">
            <div>
              <label className="text-xs font-medium text-foreground">
                {t("firewall.ipAddressLabel")}
              </label>
              <Input
                data-testid="input-global-ban-ip"
                value={ip}
                onChange={(e) => setIp(e.target.value)}
                placeholder="e.g. 198.51.100.99"
                className="mt-1 font-mono text-xs"
                autoComplete="off"
              />
            </div>

          </div>

          <DialogFooter className="flex justify-end gap-2 pt-2">
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
              data-testid="btn-confirm-add-global-ban"
              className="gap-1.5"
            >
              <Ban className="h-3.5 w-3.5" />
              {t("firewall.btnSubmitBan")}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
