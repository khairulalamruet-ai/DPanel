import React, { useState } from "react";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogFooter,
  DialogTitle,
  DialogDescription,
  Button,
  Input,
} from "@builtpanel/ui";
import { useTranslation } from "@builtpanel/i18n";
import { KeyRound, Check, RefreshCw } from "lucide-react";

export interface ResetPasswordDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  username: string;
  onConfirm: (password: string) => Promise<void>;
  isSubmitting?: boolean;
}

export function generatePassword(): string {
  const chars = "abcdefghjkmnpqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ23456789!@#$%^&*";
  let pass = "";
  for (let i = 0; i < 16; i++) {
    pass += chars.charAt(Math.floor(Math.random() * chars.length));
  }
  return pass;
}

export function ResetPasswordDialog({
  open,
  onOpenChange,
  username,
  onConfirm,
  isSubmitting = false,
}: ResetPasswordDialogProps) {
  const { t } = useTranslation();
  const [newPassword, setNewPassword] = useState(generatePassword());
  const [error, setError] = useState<string | null>(null);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!newPassword || newPassword.length < 8) {
      setError(t("userDetail.dialog.passwordMinLength"));
      return;
    }

    try {
      setError(null);
      await onConfirm(newPassword);
      onOpenChange(false);
    } catch (err: any) {
      setError(err?.message || "Failed to reset password");
    }
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent onClose={() => onOpenChange(false)} className="max-w-md p-6">
        <DialogHeader>
          <div className="flex items-center gap-2 text-primary mb-1">
            <KeyRound className="h-5 w-5" />
            <DialogTitle>{t("userDetail.dialog.resetPasswordTitle", { username })}</DialogTitle>
          </div>
          <DialogDescription>
            {t("userDetail.dialog.resetPasswordDesc", { username })}
          </DialogDescription>
        </DialogHeader>

        <form noValidate onSubmit={handleSubmit} className="space-y-4 my-2">
          <div className="space-y-1.5">
            <div className="flex items-center justify-between">
              <label htmlFor="new-password" className="text-sm font-medium text-foreground">
                {t("userDetail.dialog.newPasswordLabel")}
              </label>
              <button
                type="button"
                onClick={() => setNewPassword(generatePassword())}
                className="text-xs text-primary hover:underline flex items-center gap-1 font-medium"
              >
                <RefreshCw className="h-3 w-3" />
                {t("userDetail.dialog.generateNew")}
              </button>
            </div>
            <Input
              id="new-password"
              type="text"
              placeholder="Enter new password"
              value={newPassword}
              disabled={isSubmitting}
              onChange={(e) => setNewPassword(e.target.value)}
              className="font-mono text-sm"
            />
            {error && <p className="text-xs text-destructive">{error}</p>}
          </div>

          <DialogFooter className="pt-4 border-t">
            <Button
              type="button"
              variant="outline"
              disabled={isSubmitting}
              onClick={() => onOpenChange(false)}
            >
              {t("common.cancel")}
            </Button>
            <Button type="submit" disabled={isSubmitting} className="gap-2">
              <Check className="h-4 w-4" />
              {isSubmitting ? t("common.saving") : t("userDetail.dialog.updatePassword")}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
