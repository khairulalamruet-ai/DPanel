import React, { useState } from "react";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogFooter,
  DialogTitle,
  DialogDescription,
  Button,
} from "@builtpanel/ui";
import { useTranslation } from "@builtpanel/i18n";
import { Package, Check } from "lucide-react";
import { UserItem } from "./types";

export interface ChangePlanDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  users: UserItem[];
  onConfirm: (planId: string, planName: string) => Promise<void>;
  isSubmitting?: boolean;
}

export const AVAILABLE_PLANS = [
  { id: "plan_starter", name: "Starter Cloud" },
  { id: "plan_standard", name: "Standard Business" },
  { id: "plan_pro", name: "Pro Developer" },
  { id: "plan_enterprise", name: "Enterprise Dedicated" },
];

export function ChangePlanDialog({
  open,
  onOpenChange,
  users,
  onConfirm,
  isSubmitting = false,
}: ChangePlanDialogProps) {
  const { t } = useTranslation();
  const [selectedPlanId, setSelectedPlanId] = useState(
    users.length === 1 ? users[0].plan_id : "plan_standard"
  );
  const [error, setError] = useState<string | null>(null);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    const plan = AVAILABLE_PLANS.find((p) => p.id === selectedPlanId);
    if (!plan) {
      setError(t("users.dialog.selectPlanError"));
      return;
    }

    try {
      setError(null);
      await onConfirm(plan.id, plan.name);
      onOpenChange(false);
    } catch (err: any) {
      setError(err?.message || "Failed to update plan");
    }
  };

  const isBulk = users.length > 1;

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent onClose={() => onOpenChange(false)} className="max-w-md p-6">
        <DialogHeader>
          <div className="flex items-center gap-2 text-primary mb-1">
            <Package className="h-5 w-5" />
            <DialogTitle>
              {isBulk
                ? t("users.dialog.bulkChangePlanTitle", { count: users.length })
                : t("users.dialog.changePlanTitle", { username: users[0]?.username })}
            </DialogTitle>
          </div>
          <DialogDescription>
            {isBulk
              ? t("users.dialog.bulkChangePlanDesc", { count: users.length })
              : t("users.dialog.changePlanDesc", {
                  username: users[0]?.username,
                  current: users[0]?.plan_name,
                })}
          </DialogDescription>
        </DialogHeader>

        <form noValidate onSubmit={handleSubmit} className="space-y-4 my-2">
          <div className="space-y-2">
            <label htmlFor="select-plan" className="text-sm font-medium text-foreground">
              {t("users.dialog.targetPlanLabel")}
            </label>
            <select
              id="select-plan"
              value={selectedPlanId}
              disabled={isSubmitting}
              onChange={(e) => setSelectedPlanId(e.target.value)}
              className="flex h-10 w-full rounded-md border border-input bg-background px-3 py-2 text-sm shadow-sm transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring text-foreground"
            >
              {AVAILABLE_PLANS.map((plan) => (
                <option key={plan.id} value={plan.id}>
                  {plan.name}
                </option>
              ))}
            </select>
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
              {isSubmitting ? t("common.saving") : t("users.dialog.applyPlan")}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
