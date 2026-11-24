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
import { ShieldAlert, Lock, Loader2 } from "lucide-react";
import { useTranslation } from "@builtpanel/i18n";

interface SudoConfirmModalProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  title: string;
  description: string;
  confirmLabel?: string;
  variant?: "default" | "destructive";
  onConfirm: (password: string) => Promise<void>;
}

export function SudoConfirmModal({
  open,
  onOpenChange,
  title,
  description,
  confirmLabel,
  variant = "default",
  onConfirm,
}: SudoConfirmModalProps) {
  const { t } = useTranslation();
  const [password, setPassword] = useState("");
  const [isSubmitting, setIsSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  React.useEffect(() => {
    if (open) {
      setPassword("");
      setError(null);
      setIsSubmitting(false);
    }
  }, [open]);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!password.trim()) return;
    try {
      setIsSubmitting(true);
      setError(null);
      await onConfirm(password);
      onOpenChange(false);
    } catch (err: any) {
      setError(err?.message || t("common.error", "Verification failed. Please check password."));
    } finally {
      setIsSubmitting(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-md">
        <form onSubmit={handleSubmit} className="space-y-4">
          <DialogHeader>
            <DialogTitle className="flex items-center gap-2 text-base font-semibold">
              <ShieldAlert className={`h-5 w-5 ${variant === "destructive" ? "text-destructive" : "text-amber-500"}`} />
              <span data-testid="sudo-modal-title">{title}</span>
            </DialogTitle>
            <DialogDescription className="text-xs text-muted-foreground pt-1">
              {description}
            </DialogDescription>
          </DialogHeader>

          <div className="space-y-2">
            <label className="flex items-center gap-1.5 text-xs font-medium text-foreground">
              <Lock className="h-3.5 w-3.5 text-muted-foreground" />
              <span>{t("serverTools.sudoPasswordLabel", "Admin Sudo Password")}</span>
            </label>
            <Input
              data-testid="input-sudo-modal-password"
              type="password"
              autoFocus
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              placeholder={t("serverTools.sudoPasswordPlaceholder", "Enter administrator password")}
              className="text-sm"
              autoComplete="current-password"
            />
            {error && (
              <p className="text-xs text-destructive font-medium" data-testid="sudo-modal-error">
                {error}
              </p>
            )}
          </div>

          <DialogFooter className="gap-2 sm:gap-0">
            <Button
              type="button"
              variant="outline"
              size="sm"
              onClick={() => onOpenChange(false)}
              disabled={isSubmitting}
            >
              {t("common.cancel", "Cancel")}
            </Button>
            <Button
              type="submit"
              variant={variant}
              size="sm"
              disabled={!password.trim() || isSubmitting}
              data-testid="btn-sudo-modal-confirm"
              className="gap-1.5"
            >
              {isSubmitting && <Loader2 className="h-3.5 w-3.5 animate-spin" />}
              {confirmLabel || t("common.confirm", "Confirm")}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
