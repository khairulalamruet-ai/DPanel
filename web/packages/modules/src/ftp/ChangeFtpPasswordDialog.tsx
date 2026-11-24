import React, { useState } from "react";
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
import { changeFtpPassword } from "./api";
import { Key, RefreshCw } from "lucide-react";
import { useTranslation } from "@builtpanel/i18n";
import { FtpAccount } from "./types";

interface ChangeFtpPasswordDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  account: FtpAccount | null;
  onChanged: () => void;
}

export function ChangeFtpPasswordDialog({
  open,
  onOpenChange,
  account,
  onChanged,
}: ChangeFtpPasswordDialogProps) {
  const { t } = useTranslation();
  const { toast } = useToast();

  const [password, setPassword] = useState("");
  const [isSubmitting, setIsSubmitting] = useState(false);

  const generatePassword = () => {
    const chars = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789!@#$%^&*";
    let generated = "";
    for (let i = 0; i < 16; i++) {
      generated += chars.charAt(Math.floor(Math.random() * chars.length));
    }
    setPassword(generated);
  };

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!account || !password) return;

    try {
      setIsSubmitting(true);
      await changeFtpPassword(account.id, password);

      toast({
        title: t("common.success"),
        description: t("ftp.passwordChangedSuccess"),
      });

      setPassword("");
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
              <Key className="w-5 h-5" />
            </div>
            <DialogTitle data-testid="change-ftp-password-title">
              {t("ftp.changePassword")}
            </DialogTitle>
          </div>
        </DialogHeader>

        <form onSubmit={handleSubmit} className="space-y-4 py-2">
          <div className="space-y-1">
            <span className="text-xs text-muted-foreground">{t("ftp.username")}</span>
            <p className="font-mono text-sm font-semibold truncate" data-testid="ftp-pw-target-user">
              {account?.username}
            </p>
          </div>

          <div className="space-y-1.5">
            <div className="flex items-center justify-between">
              <label className="text-sm font-medium">
                {t("ftp.newPassword")}
              </label>
              <button
                type="button"
                onClick={generatePassword}
                className="text-xs text-primary hover:underline flex items-center gap-1"
                data-testid="btn-generate-change-pw"
              >
                <RefreshCw className="w-3 h-3" />
                <span>{t("ftp.generatePassword")}</span>
              </button>
            </div>
            <Input
              type="text"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              placeholder="••••••••••••"
              className="font-mono text-xs"
              autoFocus
              data-testid="input-new-ftp-password"
            />
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
              disabled={!password || isSubmitting}
              data-testid="btn-submit-change-ftp-pw"
            >
              {isSubmitting ? t("common.submitting") : t("common.save")}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
