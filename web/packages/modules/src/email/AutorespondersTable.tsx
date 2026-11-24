import React, { useState } from "react";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
  Button,
  Badge,
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
} from "@builtpanel/ui";
import { useTranslation } from "@builtpanel/i18n";
import { Autoresponder } from "./types";
import { CreateAutoresponderDialog } from "./CreateAutoresponderDialog";
import { Clock, Plus, Trash2 } from "lucide-react";

interface AutorespondersTableProps {
  autoresponders: Autoresponder[];
  domain: string;
  loading: boolean;
  onCreateAutoresponder: (data: {
    email: string;
    domain: string;
    subject: string;
    body: string;
    start_date?: string;
    end_date?: string;
  }) => Promise<void>;
  onDeleteAutoresponder: (id: string) => Promise<void>;
}

export function AutorespondersTable({
  autoresponders,
  domain,
  loading,
  onCreateAutoresponder,
  onDeleteAutoresponder,
}: AutorespondersTableProps) {
  const { t } = useTranslation();

  const [createDialogOpen, setCreateDialogOpen] = useState(false);
  const [deleteTarget, setDeleteTarget] = useState<Autoresponder | null>(null);
  const [isDeleting, setIsDeleting] = useState(false);

  const handleDeleteConfirm = async () => {
    if (!deleteTarget) return;
    setIsDeleting(true);
    try {
      await onDeleteAutoresponder(deleteTarget.id);
      setDeleteTarget(null);
    } finally {
      setIsDeleting(false);
    }
  };

  return (
    <div className="space-y-4" data-testid="autoresponders-section">
      <div className="flex flex-col sm:flex-row sm:items-center sm:justify-between gap-3">
        <div>
          <h3 className="text-base font-semibold">
            {t("email.autorespondersTitle", "Email Autoresponders")}
          </h3>
          <p className="text-xs text-muted-foreground">
            {t("email.autorespondersSubtitle", "Manage automatic replies and out-of-office notices for your accounts.")}
          </p>
        </div>
        <Button
          onClick={() => setCreateDialogOpen(true)}
          className="gap-2 shrink-0"
          data-testid="add-autoresponder-btn"
        >
          <Plus className="h-4 w-4" />
          {t("email.addAutoresponderBtn", "Add Autoresponder")}
        </Button>
      </div>

      <div className="rounded-lg border bg-card">
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>{t("email.mailboxAccountCol", "Email Account")}</TableHead>
              <TableHead>{t("email.autoresponderSubjectCol", "Subject")}</TableHead>
              <TableHead>{t("common.status", "Status")}</TableHead>
              <TableHead className="text-right">{t("common.actions", "Actions")}</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {loading ? (
              <TableRow>
                <TableCell colSpan={4} className="text-center py-8 text-muted-foreground">
                  {t("common.loading", "Loading autoresponders...")}
                </TableCell>
              </TableRow>
            ) : autoresponders.length === 0 ? (
              <TableRow>
                <TableCell colSpan={4} className="text-center py-8 text-muted-foreground">
                  {t("email.noAutoresponders", "No autoresponders configured for this domain.")}
                </TableCell>
              </TableRow>
            ) : (
              autoresponders.map((item) => (
                <TableRow key={item.id} data-testid={`autoresponder-row-${item.id}`}>
                  <TableCell className="font-mono text-sm font-medium">
                    <div className="flex items-center gap-2">
                      <Clock className="h-4 w-4 text-primary shrink-0" />
                      <span>{item.email}</span>
                    </div>
                  </TableCell>
                  <TableCell className="text-sm">
                    <div className="font-medium">{item.subject}</div>
                    <div className="text-xs text-muted-foreground truncate max-w-xs">{item.body}</div>
                  </TableCell>
                  <TableCell>
                    <Badge variant={item.is_active ? "success" : "secondary"}>
                      {item.is_active ? t("common.active", "Active") : t("common.inactive", "Inactive")}
                    </Badge>
                  </TableCell>
                  <TableCell className="text-right">
                    <Button
                      variant="ghost"
                      size="sm"
                      onClick={() => setDeleteTarget(item)}
                      className="text-destructive hover:bg-destructive/10"
                      data-testid={`delete-autoresponder-btn-${item.id}`}
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

      <CreateAutoresponderDialog
        open={createDialogOpen}
        domain={domain}
        onClose={() => setCreateDialogOpen(false)}
        onCreate={onCreateAutoresponder}
      />

      {deleteTarget && (
        <Dialog open={Boolean(deleteTarget)} onOpenChange={(val) => !val && setDeleteTarget(null)}>
          <DialogContent className="sm:max-w-md">
            <DialogHeader>
              <DialogTitle>{t("email.deleteAutoresponderConfirmTitle", "Delete Autoresponder")}</DialogTitle>
              <DialogDescription className="text-xs">
                {t(
                  "email.deleteAutoresponderConfirmDesc",
                  "Are you sure you want to delete the autoresponder for {{email}}?",
                  { email: deleteTarget.email }
                )}
              </DialogDescription>
            </DialogHeader>
            <DialogFooter className="pt-2">
              <Button variant="outline" onClick={() => setDeleteTarget(null)} disabled={isDeleting}>
                {t("common.cancel", "Cancel")}
              </Button>
              <Button variant="destructive" onClick={handleDeleteConfirm} disabled={isDeleting}>
                {isDeleting ? t("common.deleting", "Deleting...") : t("common.delete", "Delete")}
              </Button>
            </DialogFooter>
          </DialogContent>
        </Dialog>
      )}
    </div>
  );
}
