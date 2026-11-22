import React, { useState, useEffect } from "react";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogFooter,
  Button,
  Input,
  useToast,
} from "@builtpanel/ui";
import { changeFtpQuota } from "./api";
import { HardDrive } from "lucide-react";
import { useTranslation } from "@builtpanel/i18n";
import { FtpAccount } from "./types";

interface ChangeFtpQuotaDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  account: FtpAccount | null;
  onChanged: () => void;
}

export function ChangeFtpQuotaDialog({
  open,
  onOpenChange,
  account,
  onChanged,
}: ChangeFtpQuotaDialogProps) {
  const { t } = useTranslation();
  const { toast } = useToast();

  const [quotaUnlimited, setQuotaUnlimited] = useState(true);
  const [quotaMb, setQuotaMb] = useState<number>(500);
  const [isSubmitting, setIsSubmitting] = useState(false);

  useEffect(() => {
    if (open && account) {
      if (account.quota_bytes === -1) {
        setQuotaUnlimited(true);
        setQuotaMb(500);
      } else {
        setQuotaUnlimited(false);
        setQuotaMb(Math.round(account.quota_bytes / (1024 * 1024)));
      }
    }
  }, [open, account]);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!account) return;

    try {
      setIsSubmitting(true);
      await changeFtpQuota(account.id, quotaUnlimited ? null : Number(quotaMb));

      toast({
        title: t("common.success"),
        description: t("ftp.quotaUpdatedSuccess"),
      });

      onOpenChange(false);
      onChanged();
    } catch (err: any) {
      toast({
        title: t("common.error"),
        description: err.message || t("common.unknownError"),
        variant: "destructive",
      });
    } finally {
      setIsSubmitting(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-[400px]">
        <DialogHeader>
          <div className="flex items-center gap-2">
            <div className="p-2 rounded-lg bg-primary/10 text-primary">
              <HardDrive className="w-5 h-5" />
            </div>
            <DialogTitle data-testid="change-ftp-quota-title">
              {t("ftp.changeQuota")}
            </DialogTitle>
          </div>
        </DialogHeader>

        <form onSubmit={handleSubmit} className="space-y-4 py-2">
          <div className="space-y-1">
            <span className="text-xs text-muted-foreground">{t("ftp.username")}</span>
            <p className="font-mono text-sm font-semibold truncate">
              {account?.username}
            </p>
          </div>

          <div className="space-y-2">
            <label className="text-sm font-medium">
              {t("ftp.quota")}
            </label>
            <div className="flex gap-4">
              <label className="flex items-center gap-2 text-sm cursor-pointer">
                <input
                  type="radio"
                  name="edit-quota"
                  checked={quotaUnlimited}
                  onChange={() => setQuotaUnlimited(true)}
                  data-testid="edit-radio-quota-unlimited"
                />
                <span>{t("ftp.unlimited")}</span>
              </label>
              <label className="flex items-center gap-2 text-sm cursor-pointer">
                <input
                  type="radio"
                  name="edit-quota"
                  checked={!quotaUnlimited}
                  onChange={() => setQuotaUnlimited(false)}
                  data-testid="edit-radio-quota-limited"
                />
                <span>{t("ftp.customQuota")}</span>
              </label>
            </div>

            {!quotaUnlimited && (
              <div className="flex items-center gap-2 pt-1">
                <Input
                  type="number"
                  min="1"
                  value={quotaMb}
                  onChange={(e) => setQuotaMb(Number(e.target.value))}
                  className="w-32 font-mono"
                  data-testid="edit-input-ftp-quota-mb"
                />
                <span className="text-sm text-muted-foreground">MB</span>
              </div>
            )}
          </div>

          <DialogFooter className="pt-2">
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
              disabled={isSubmitting}
              data-testid="btn-submit-change-quota"
            >
              {isSubmitting ? t("common.submitting") : t("common.save")}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
