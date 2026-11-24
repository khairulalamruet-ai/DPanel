import React, { useState } from "react";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
  Button,
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
} from "@builtpanel/ui";
import { useTranslation } from "@builtpanel/i18n";
import { EmailForwarder } from "./types";
import { CreateForwarderDialog } from "./CreateForwarderDialog";
import { CornerUpRight, Plus, Trash2, ArrowRight } from "lucide-react";

interface ForwardersTableProps {
  forwarders: EmailForwarder[];
  domain: string;
  loading: boolean;
  onCreateForwarder: (data: { source: string; destination: string; domain: string }) => Promise<void>;
  onDeleteForwarder: (id: string) => Promise<void>;
}

export function ForwardersTable({
  forwarders,
  domain,
  loading,
  onCreateForwarder,
  onDeleteForwarder,
}: ForwardersTableProps) {
  const { t } = useTranslation();

  const [createDialogOpen, setCreateDialogOpen] = useState(false);
  const [deleteTarget, setDeleteTarget] = useState<EmailForwarder | null>(null);
  const [isDeleting, setIsDeleting] = useState(false);

  const handleDeleteConfirm = async () => {
    if (!deleteTarget) return;
    setIsDeleting(true);
    try {
      await onDeleteForwarder(deleteTarget.id);
      setDeleteTarget(null);
    } finally {
      setIsDeleting(false);
    }
  };

  return (
    <div className="space-y-4" data-testid="forwarders-section">
      <div className="flex flex-col sm:flex-row sm:items-center sm:justify-between gap-3">
        <div>
          <h3 className="text-base font-semibold">
            {t("email.forwardersTitle", "Email Forwarders")}
          </h3>
          <p className="text-xs text-muted-foreground">
            {t("email.forwardersSubtitle", "Route copies of incoming messages to alternative email destinations.")}
          </p>
        </div>
        <Button
          onClick={() => setCreateDialogOpen(true)}
          className="gap-2 shrink-0"
          data-testid="add-forwarder-btn"
        >
          <Plus className="h-4 w-4" />
          {t("email.addForwarderBtn", "Add Forwarder")}
        </Button>
      </div>

      <div className="rounded-lg border bg-card">
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>{t("email.forwarderSourceCol", "Source Address")}</TableHead>
              <TableHead></TableHead>
              <TableHead>{t("email.forwarderDestCol", "Destination")}</TableHead>
              <TableHead className="text-right">{t("common.actions", "Actions")}</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {loading ? (
              <TableRow>
                <TableCell colSpan={4} className="text-center py-8 text-muted-foreground">
                  {t("common.loading", "Loading forwarders...")}
                </TableCell>
              </TableRow>
            ) : forwarders.length === 0 ? (
              <TableRow>
                <TableCell colSpan={4} className="text-center py-8 text-muted-foreground">
                  {t("email.noForwarders", "No email forwarders configured for this domain.")}
                </TableCell>
              </TableRow>
            ) : (
              forwarders.map((fwd) => (
                <TableRow key={fwd.id} data-testid={`forwarder-row-${fwd.id}`}>
                  <TableCell className="font-mono text-sm font-medium">
                    <div className="flex items-center gap-2">
                      <CornerUpRight className="h-4 w-4 text-primary shrink-0" />
                      <span>{fwd.source}</span>
                    </div>
                  </TableCell>
                  <TableCell className="w-12 text-center text-muted-foreground">
                    <ArrowRight className="h-4 w-4 inline-block opacity-60" />
                  </TableCell>
                  <TableCell className="font-mono text-sm text-foreground">
                    {fwd.destination}
                  </TableCell>
                  <TableCell className="text-right">
                    <Button
                      variant="ghost"
                      size="sm"
                      onClick={() => setDeleteTarget(fwd)}
                      className="text-destructive hover:bg-destructive/10"
                      data-testid={`delete-forwarder-btn-${fwd.id}`}
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

      <CreateForwarderDialog
        open={createDialogOpen}
        domain={domain}
        onClose={() => setCreateDialogOpen(false)}
        onCreate={onCreateForwarder}
      />

      {deleteTarget && (
        <Dialog open={Boolean(deleteTarget)} onOpenChange={(val) => !val && setDeleteTarget(null)}>
          <DialogContent className="sm:max-w-md">
            <DialogHeader>
              <DialogTitle>{t("email.deleteForwarderConfirmTitle", "Delete Forwarder")}</DialogTitle>
              <DialogDescription className="text-xs">
                {t(
                  "email.deleteForwarderConfirmDesc",
                  "Are you sure you want to delete the forwarder from {{source}} to {{destination}}?",
                  { source: deleteTarget.source, destination: deleteTarget.destination }
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
