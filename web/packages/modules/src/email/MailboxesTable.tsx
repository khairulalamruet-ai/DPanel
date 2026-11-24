import React, { useState } from "react";
import {
  Table,
  TableHeader,
  TableBody,
  TableRow,
  TableHead,
  TableCell,
  Button,
  Input,
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
} from "@builtpanel/ui";
import { useTranslation } from "@builtpanel/i18n";
import {
  Mail,
  ExternalLink,
  Trash2,
  Key,
  Search,
  Plus,
  AlertTriangle,
} from "lucide-react";
import { Mailbox } from "./types";
import { CreateMailboxDialog } from "./CreateMailboxDialog";

interface MailboxesTableProps {
  mailboxes: Mailbox[];
  domain: string;
  loading?: boolean;
  onOpenCreate?: () => void;
  onCreateMailbox?: (data: { username: string; domain: string; password: string; quota_mb: number }) => Promise<void>;
  onDeleteMailbox: (id: string) => Promise<void>;
  onChangePassword: (id: string, newPass: string) => Promise<void>;
}

export function MailboxesTable({
  mailboxes,
  domain,
  loading,
  onOpenCreate,
  onCreateMailbox,
  onDeleteMailbox,
  onChangePassword,
}: MailboxesTableProps) {
  const { t } = useTranslation();
  const [search, setSearch] = useState("");
  const [internalCreateOpen, setInternalCreateOpen] = useState(false);
  const [mailboxToDelete, setMailboxToDelete] = useState<Mailbox | null>(null);
  const [isDeleting, setIsDeleting] = useState(false);

  // Password change state
  const [mailboxToChangePass, setMailboxToChangePass] = useState<Mailbox | null>(null);
  const [newPassword, setNewPassword] = useState("");
  const [isChangingPass, setIsChangingPass] = useState(false);

  const filtered = mailboxes.filter((m) => {
    if (search.trim()) {
      return m.email.toLowerCase().includes(search.toLowerCase());
    }
    return true;
  });

  const formatBytesToMb = (bytes: number) => {
    return Math.round((bytes / (1024 * 1024)) * 10) / 10;
  };

  const confirmDelete = async () => {
    if (!mailboxToDelete) return;
    setIsDeleting(true);
    try {
      await onDeleteMailbox(mailboxToDelete.id);
      setMailboxToDelete(null);
    } finally {
      setIsDeleting(false);
    }
  };

  const confirmPasswordChange = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!mailboxToChangePass || !newPassword) return;
    setIsChangingPass(true);
    try {
      await onChangePassword(mailboxToChangePass.id, newPassword);
      setMailboxToChangePass(null);
      setNewPassword("");
    } finally {
      setIsChangingPass(false);
    }
  };

  const handleOpenCreateModal = () => {
    if (onOpenCreate) {
      onOpenCreate();
    } else {
      setInternalCreateOpen(true);
    }
  };

  return (
    <div className="space-y-4" data-testid="mailboxes-section">
      {/* Header & Search */}
      <div className="flex flex-col sm:flex-row items-start sm:items-center justify-between gap-3">
        <div className="relative w-full sm:w-72">
          <Search className="absolute left-2.5 top-2.5 h-4 w-4 text-muted-foreground" />
          <Input
            type="text"
            placeholder={t("email.searchMailbox", "Search accounts...")}
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            className="pl-8 text-xs"
            data-testid="input-search-mailboxes"
          />
        </div>

        <Button
          onClick={handleOpenCreateModal}
          data-testid="create-mailbox-btn"
          className="flex items-center gap-1.5 h-8 text-xs font-semibold"
        >
          <Plus className="h-4 w-4" />
          <span>{t("email.createAccount", "Create Email Account")}</span>
        </Button>
      </div>

      {loading ? (
        <div className="p-8 text-center text-muted-foreground">
          {t("common.loading", "Loading mailboxes...")}
        </div>
      ) : filtered.length === 0 ? (
        <div className="p-8 text-center border rounded-lg bg-card" data-testid="empty-mailboxes-placeholder">
          <Mail className="h-10 w-10 text-muted-foreground mx-auto mb-2 opacity-60" />
          <p className="font-semibold text-sm">{t("email.noAccountsFound", "No email accounts found for {{domain}}", { domain })}</p>
          <p className="text-xs text-muted-foreground mt-1">
            {t("email.createFirstAccountPrompt", "Click Create Email Account above to set up your primary mailbox.")}
          </p>
        </div>
      ) : (
        <div className="rounded-lg border bg-card overflow-hidden">
          <Table data-testid="mailboxes-table">
            <TableHeader>
              <TableRow>
                <TableHead>{t("email.accountHeader", "Account")}</TableHead>
                <TableHead>{t("email.storageQuota", "Storage / Quota")}</TableHead>
                <TableHead>{t("email.webmailHeader", "Webmail Access")}</TableHead>
                <TableHead className="text-right">{t("common.actions", "Actions")}</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {filtered.map((mb) => {
                const usedMb = formatBytesToMb(mb.used_bytes);
                const percent = Math.min(100, Math.round((usedMb / mb.quota_mb) * 100));

                return (
                  <TableRow key={mb.id} data-testid={`mailbox-row-${mb.id}`}>
                    <TableCell>
                      <div className="flex items-center gap-2.5">
                        <div className="p-1.5 rounded-full bg-primary/10 text-primary shrink-0">
                          <Mail className="h-4 w-4" />
                        </div>
                        <div>
                          <span className="font-bold text-sm block font-mono" data-testid={`mailbox-email-${mb.id}`}>
                            {mb.email}
                          </span>
                          <span className="text-[10px] text-muted-foreground">
                            Created: {new Date(mb.created_at).toLocaleDateString()}
                          </span>
                        </div>
                      </div>
                    </TableCell>

                    <TableCell>
                      <div className="space-y-1 w-44">
                        <div className="flex items-center justify-between text-[11px] font-mono">
                          <span>{usedMb} MB</span>
                          <span className="text-muted-foreground">/ {mb.quota_mb} MB ({percent}%)</span>
                        </div>
                        <div className="w-full bg-muted rounded-full h-1.5 overflow-hidden">
                          <div
                            className={`h-full rounded-full transition-all ${
                              percent > 85 ? "bg-red-500" : percent > 65 ? "bg-amber-500" : "bg-primary"
                            }`}
                            style={{ width: `${percent}%` }}
                          />
                        </div>
                      </div>
                    </TableCell>

                    <TableCell>
                      <a
                        href={mb.webmail_url}
                        target="_blank"
                        rel="noopener noreferrer"
                        data-testid={`webmail-btn-${mb.id}`}
                        className="inline-flex items-center gap-1 text-xs font-semibold text-primary hover:underline"
                      >
                        <ExternalLink className="h-3 w-3" />
                        {t("email.openWebmail", "Open Webmail")}
                      </a>
                    </TableCell>

                    <TableCell className="text-right">
                      <div className="flex items-center justify-end gap-1.5">
                        <Button
                          variant="outline"
                          size="sm"
                          onClick={() => setMailboxToChangePass(mb)}
                          data-testid={`change-pass-btn-${mb.id}`}
                          className="h-7 text-xs px-2"
                        >
                          <Key className="h-3 w-3 mr-1" />
                          {t("email.password", "Password")}
                        </Button>

                        <Button
                          variant="ghost"
                          size="sm"
                          onClick={() => setMailboxToDelete(mb)}
                          data-testid={`delete-mailbox-btn-${mb.id}`}
                          className="h-7 w-7 p-0 text-destructive hover:bg-destructive/10"
                        >
                          <Trash2 className="h-3.5 w-3.5" />
                        </Button>
                      </div>
                    </TableCell>
                  </TableRow>
                );
              })}
            </TableBody>
          </Table>
        </div>
      )}

      {/* Internal Create Dialog if managed internally */}
      {onCreateMailbox && (
        <CreateMailboxDialog
          open={internalCreateOpen}
          domain={domain}
          onClose={() => setInternalCreateOpen(false)}
          onCreate={onCreateMailbox}
        />
      )}

      {/* Delete Dialog */}
      <Dialog open={Boolean(mailboxToDelete)} onOpenChange={(val) => !val && setMailboxToDelete(null)}>
        <DialogContent data-testid="delete-mailbox-dialog" className="sm:max-w-md">
          <DialogHeader>
            <div className="flex items-center gap-2 text-destructive">
              <AlertTriangle className="h-5 w-5" />
              <DialogTitle>{t("email.deleteMailboxTitle", "Delete Email Account")}</DialogTitle>
            </div>
            <DialogDescription className="text-xs">
              {t(
                "email.deleteMailboxWarning",
                "Are you sure you want to permanently delete {{email}}? All messages and mail stored in this mailbox will be destroyed.",
                { email: mailboxToDelete?.email }
              )}
            </DialogDescription>
          </DialogHeader>

          <DialogFooter>
            <Button variant="outline" onClick={() => setMailboxToDelete(null)} disabled={isDeleting}>
              {t("common.cancel", "Cancel")}
            </Button>
            <Button
              variant="destructive"
              onClick={confirmDelete}
              disabled={isDeleting}
              data-testid="btn-confirm-delete-mailbox"
            >
              {isDeleting ? t("common.deleting", "Deleting...") : t("common.delete", "Delete Mailbox")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* Change Password Dialog */}
      <Dialog open={Boolean(mailboxToChangePass)} onOpenChange={(val) => !val && setMailboxToChangePass(null)}>
        <DialogContent data-testid="change-pass-dialog" className="sm:max-w-md">
          <DialogHeader>
            <div className="flex items-center gap-2">
              <Key className="h-5 w-5 text-primary" />
              <DialogTitle>{t("email.changePasswordTitle", "Change Mailbox Password")}</DialogTitle>
            </div>
            <DialogDescription className="text-xs">
              {mailboxToChangePass?.email}
            </DialogDescription>
          </DialogHeader>

          <form onSubmit={confirmPasswordChange} className="space-y-4 py-2">
            <div>
              <label className="text-xs font-semibold block mb-1">
                {t("email.newPasswordLabel", "New Password")}
              </label>
              <Input
                type="text"
                placeholder="Enter new secure password"
                value={newPassword}
                onChange={(e) => setNewPassword(e.target.value)}
                className="text-xs h-8 font-mono"
                required
                data-testid="input-new-mail-password"
              />
            </div>

            <DialogFooter>
              <Button type="button" variant="outline" onClick={() => setMailboxToChangePass(null)} disabled={isChangingPass}>
                {t("common.cancel", "Cancel")}
              </Button>
              <Button type="submit" disabled={isChangingPass || !newPassword} data-testid="btn-confirm-new-password">
                {isChangingPass ? t("common.saving", "Saving...") : t("common.save", "Update Password")}
              </Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>
    </div>
  );
}
