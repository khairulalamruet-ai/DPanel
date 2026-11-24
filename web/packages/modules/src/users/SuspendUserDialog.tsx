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
import { AlertTriangle, ShieldAlert, CheckCircle2 } from "lucide-react";
import { UserItem } from "./types";

export interface SuspendUserDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  users: UserItem[];
  action: "suspend" | "unsuspend";
  onConfirm: (reason?: string) => Promise<void>;
  isSubmitting?: boolean;
}

export function SuspendUserDialog({
  open,
  onOpenChange,
  users,
  action,
  onConfirm,
  isSubmitting = false,
}: SuspendUserDialogProps) {
  const { t } = useTranslation();
  const [reason, setReason] = useState("");
  const isSuspending = action === "suspend";
  const isBulk = users.length > 1;

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    await onConfirm(reason.trim() || undefined);
    onOpenChange(false);
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent onClose={() => onOpenChange(false)} className="max-w-md p-6">
        <DialogHeader>
          <div
            className={`flex items-center gap-2 mb-1 ${
              isSuspending ? "text-destructive" : "text-emerald-500"
            }`}
          >
            {isSuspending ? (
              <ShieldAlert className="h-5 w-5" />
            ) : (
              <CheckCircle2 className="h-5 w-5" />
            )}
            <DialogTitle>
              {isSuspending
                ? isBulk
                  ? t("users.dialog.bulkSuspendTitle", { count: users.length })
                  : t("users.dialog.suspendTitle", { username: users[0]?.username })
                : isBulk
                ? t("users.dialog.bulkUnsuspendTitle", { count: users.length })
                : t("users.dialog.unsuspendTitle", { username: users[0]?.username })}
            </DialogTitle>
          </div>
          <DialogDescription>
            {isSuspending
              ? isBulk
                ? t("users.dialog.bulkSuspendDesc", { count: users.length })
                : t("users.dialog.suspendDesc", { username: users[0]?.username })
              : isBulk
              ? t("users.dialog.bulkUnsuspendDesc", { count: users.length })
              : t("users.dialog.unsuspendDesc", { username: users[0]?.username })}
          </DialogDescription>
        </DialogHeader>

        <form noValidate onSubmit={handleSubmit} className="space-y-4 my-2">
          {isSuspending && (
            <div className="space-y-1.5">
              <label htmlFor="suspend-reason" className="text-sm font-medium text-foreground">
                {t("users.dialog.reasonLabel")}
              </label>
              <Input
                id="suspend-reason"
                value={reason}
                onChange={(e) => setReason(e.target.value)}
                placeholder={t("users.dialog.reasonPlaceholder")}
                disabled={isSubmitting}
              />
            </div>
          )}

          <DialogFooter className="pt-4 border-t">
            <Button
              type="button"
              variant="outline"
              disabled={isSubmitting}
              onClick={() => onOpenChange(false)}
            >
              {t("common.cancel")}
            </Button>
            <Button
              type="submit"
              variant={isSuspending ? "destructive" : "default"}
              disabled={isSubmitting}
              className="gap-2"
            >
              {isSuspending ? (
                <>
                  <AlertTriangle className="h-4 w-4" />
                  {isSubmitting ? t("common.processing") : t("users.dialog.confirmSuspend")}
                </>
              ) : (
                <>
                  <CheckCircle2 className="h-4 w-4" />
                  {isSubmitting ? t("common.processing") : t("users.dialog.confirmUnsuspend")}
                </>
              )}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
