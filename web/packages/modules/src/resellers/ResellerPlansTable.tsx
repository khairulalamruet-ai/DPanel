import React, { useState } from "react";
import {
  Table,
  TableHeader,
  TableBody,
  TableRow,
  TableHead,
  TableCell,
  Button,
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
} from "@builtpanel/ui";
import { useTranslation } from "@builtpanel/i18n";
import { Layers, Plus, Trash2, AlertTriangle } from "lucide-react";
import { ResellerPlan } from "./types";
import { CreateResellerPlanDialog } from "./CreateResellerPlanDialog";

interface ResellerPlansTableProps {
  plans: ResellerPlan[];
  loading: boolean;
  onCreatePlan: (data: {
    name: string;
    disk_mb: number;
    bandwidth_mb: number;
    domains_limit: number;
    databases_limit: number;
    mailboxes_limit: number;
  }) => Promise<void>;
  onDeletePlan: (id: string) => Promise<void>;
}

export function ResellerPlansTable({
  plans,
  loading,
  onCreatePlan,
  onDeletePlan,
}: ResellerPlansTableProps) {
  const { t } = useTranslation();

  const [createDialogOpen, setCreateDialogOpen] = useState(false);
  const [deleteTarget, setDeleteTarget] = useState<ResellerPlan | null>(null);
  const [isDeleting, setIsDeleting] = useState(false);

  const handleDeleteConfirm = async () => {
    if (!deleteTarget) return;
    setIsDeleting(true);
    try {
      await onDeletePlan(deleteTarget.id);
      setDeleteTarget(null);
    } finally {
      setIsDeleting(false);
    }
  };

  return (
    <div className="space-y-4" data-testid="reseller-plans-section">
      <div className="flex flex-col sm:flex-row items-start sm:items-center justify-between gap-3">
        <div>
          <h3 className="text-base font-semibold">
            {t("reseller.plansTitle", "Custom Client Plans")}
          </h3>
          <p className="text-xs text-muted-foreground">
            {t("reseller.plansSubtitle", "Package disk quotas, databases, and mailbox limits for your clients.")}
          </p>
        </div>

        <Button
          onClick={() => setCreateDialogOpen(true)}
          data-testid="btn-add-reseller-plan"
          className="flex items-center gap-1.5 shrink-0"
        >
          <Plus className="h-4 w-4" />
          {t("reseller.createPlanBtn", "Create Plan")}
        </Button>
      </div>

      <div className="rounded-lg border bg-card overflow-hidden">
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>{t("reseller.planNameCol", "Plan Name")}</TableHead>
              <TableHead>{t("reseller.diskCol", "Disk Space")}</TableHead>
              <TableHead>{t("reseller.bwCol", "Bandwidth")}</TableHead>
              <TableHead>{t("reseller.domainsLimitCol", "Domains")}</TableHead>
              <TableHead>{t("reseller.dbsLimitCol", "Databases")}</TableHead>
              <TableHead className="text-right">{t("common.actions", "Actions")}</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {loading ? (
              <TableRow>
                <TableCell colSpan={6} className="text-center py-8 text-muted-foreground">
                  {t("common.loading", "Loading plans...")}
                </TableCell>
              </TableRow>
            ) : plans.length === 0 ? (
              <TableRow>
                <TableCell colSpan={6} className="text-center py-8 text-muted-foreground">
                  {t("reseller.noPlans", "No custom plans configured. Create one to assign to clients.")}
                </TableCell>
              </TableRow>
            ) : (
              plans.map((p) => (
                <TableRow key={p.id} data-testid={`reseller-plan-row-${p.id}`}>
                  <TableCell className="font-semibold text-sm">
                    <div className="flex items-center gap-2">
                      <Layers className="h-4 w-4 text-primary shrink-0" />
                      <span>{p.name}</span>
                    </div>
                  </TableCell>
                  <TableCell className="font-mono text-xs">
                    {Math.round((p.disk_mb / 1024) * 10) / 10} GB
                  </TableCell>
                  <TableCell className="font-mono text-xs">
                    {Math.round((p.bandwidth_mb / 1024) * 10) / 10} GB
                  </TableCell>
                  <TableCell className="text-xs">
                    {p.domains_limit}
                  </TableCell>
                  <TableCell className="text-xs">
                    {p.databases_limit}
                  </TableCell>
                  <TableCell className="text-right">
                    <Button
                      variant="ghost"
                      size="sm"
                      onClick={() => setDeleteTarget(p)}
                      data-testid={`btn-delete-reseller-plan-${p.id}`}
                      className="h-8 w-8 p-0 text-destructive hover:bg-destructive/10"
                    >
                      <Trash2 className="h-4 w-4" />
                    </Button>
                  </TableCell>
                </TableRow>
              ))
            )}
          </TableBody>
        </Table>
      </div>

      <CreateResellerPlanDialog
        open={createDialogOpen}
        onClose={() => setCreateDialogOpen(false)}
        onCreate={onCreatePlan}
      />

      {deleteTarget && (
        <Dialog open={Boolean(deleteTarget)} onOpenChange={(val) => !val && setDeleteTarget(null)}>
          <DialogContent className="sm:max-w-md">
            <DialogHeader>
              <div className="flex items-center gap-2 text-destructive">
                <AlertTriangle className="h-5 w-5" />
                <DialogTitle>{t("reseller.deletePlanTitle", "Delete Plan")}</DialogTitle>
              </div>
              <DialogDescription className="text-xs">
                {t(
                  "reseller.deletePlanDesc",
                  "Are you sure you want to delete plan {{name}}? Existing accounts will retain their current limits.",
                  { name: deleteTarget.name }
                )}
              </DialogDescription>
            </DialogHeader>
            <DialogFooter className="pt-2">
              <Button variant="outline" onClick={() => setDeleteTarget(null)} disabled={isDeleting}>
                {t("common.cancel", "Cancel")}
              </Button>
              <Button
                variant="destructive"
                onClick={handleDeleteConfirm}
                disabled={isDeleting}
                data-testid="btn-confirm-delete-plan"
              >
                {isDeleting ? t("common.deleting", "Deleting...") : t("common.delete", "Delete Plan")}
              </Button>
            </DialogFooter>
          </DialogContent>
        </Dialog>
      )}
    </div>
  );
}
