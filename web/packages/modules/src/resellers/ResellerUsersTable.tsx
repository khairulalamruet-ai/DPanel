import React, { useState } from "react";
import {
  Table,
  TableHeader,
  TableBody,
  TableRow,
  TableHead,
  TableCell,
  Button,
  Badge,
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
  User,
  Plus,
  Search,
  Globe,
  Trash2,
  AlertTriangle,
  PauseCircle,
  PlayCircle,
} from "lucide-react";
import { ResellerUser, ResellerPlan } from "./types";
import { CreateResellerUserDialog } from "./CreateResellerUserDialog";

interface ResellerUsersTableProps {
  users: ResellerUser[];
  plans: ResellerPlan[];
  loading: boolean;
  onCreateUser: (data: {
    username: string;
    email: string;
    domain: string;
    password: string;
    plan_name: string;
  }) => Promise<void>;
  onToggleSuspend: (id: string, currentStatus: "active" | "suspended") => Promise<void>;
  onDeleteUser: (id: string) => Promise<void>;
}

export function ResellerUsersTable({
  users,
  plans,
  loading,
  onCreateUser,
  onToggleSuspend,
  onDeleteUser,
}: ResellerUsersTableProps) {
  const { t } = useTranslation();

  const [search, setSearch] = useState("");
  const [createDialogOpen, setCreateDialogOpen] = useState(false);
  const [deleteTarget, setDeleteTarget] = useState<ResellerUser | null>(null);
  const [isDeleting, setIsDeleting] = useState(false);

  const filtered = users.filter((u) => {
    if (!search.trim()) return true;
    const q = search.toLowerCase();
    return u.username.toLowerCase().includes(q) || u.domain.toLowerCase().includes(q);
  });

  const handleDeleteConfirm = async () => {
    if (!deleteTarget) return;
    setIsDeleting(true);
    try {
      await onDeleteUser(deleteTarget.id);
      setDeleteTarget(null);
    } finally {
      setIsDeleting(false);
    }
  };

  return (
    <div className="space-y-4" data-testid="reseller-users-section">
      <div className="flex flex-col sm:flex-row items-start sm:items-center justify-between gap-3">
        <div className="relative w-full sm:w-72">
          <Search className="absolute left-2.5 top-2.5 h-4 w-4 text-muted-foreground" />
          <Input
            type="text"
            placeholder={t("reseller.searchUsers", "Search users or domains...")}
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            className="pl-8 text-xs"
            data-testid="input-search-reseller-users"
          />
        </div>

        <Button
          onClick={() => setCreateDialogOpen(true)}
          data-testid="btn-add-reseller-user"
          className="flex items-center gap-1.5 shrink-0"
        >
          <Plus className="h-4 w-4" />
          {t("reseller.createUserBtn", "Add Client Account")}
        </Button>
      </div>

      <div className="rounded-lg border bg-card overflow-hidden">
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>{t("reseller.userCol", "Client Account")}</TableHead>
              <TableHead>{t("reseller.domainCol", "Primary Domain")}</TableHead>
              <TableHead>{t("reseller.planCol", "Plan")}</TableHead>
              <TableHead>{t("reseller.diskCol", "Disk")}</TableHead>
              <TableHead>{t("common.status", "Status")}</TableHead>
              <TableHead className="text-right">{t("common.actions", "Actions")}</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {loading ? (
              <TableRow>
                <TableCell colSpan={6} className="text-center py-8 text-muted-foreground">
                  {t("common.loading", "Loading client accounts...")}
                </TableCell>
              </TableRow>
            ) : filtered.length === 0 ? (
              <TableRow>
                <TableCell colSpan={6} className="text-center py-8 text-muted-foreground">
                  {t("reseller.noUsersFound", "No client accounts found under this reseller.")}
                </TableCell>
              </TableRow>
            ) : (
              filtered.map((user) => (
                <TableRow key={user.id} data-testid={`reseller-user-row-${user.id}`}>
                  <TableCell>
                    <div className="flex items-center gap-2">
                      <div className="p-1.5 rounded-full bg-primary/10 text-primary shrink-0">
                        <User className="h-4 w-4" />
                      </div>
                      <div>
                        <span className="font-bold text-sm block font-mono" data-testid={`reseller-user-name-${user.id}`}>
                          {user.username}
                        </span>
                        <span className="text-[10px] text-muted-foreground">
                          {user.email}
                        </span>
                      </div>
                    </div>
                  </TableCell>

                  <TableCell className="font-mono text-xs">
                    <a
                      href={`https://${user.domain}`}
                      target="_blank"
                      rel="noopener noreferrer"
                      className="text-primary hover:underline flex items-center gap-1"
                    >
                      <Globe className="h-3 w-3" />
                      {user.domain}
                    </a>
                  </TableCell>

                  <TableCell className="text-xs font-medium">
                    {user.plan_name}
                  </TableCell>

                  <TableCell className="text-xs font-mono">
                    {Math.round((user.disk_used_mb / 1024) * 10) / 10} GB
                  </TableCell>

                  <TableCell>
                    <Badge variant={user.status === "active" ? "success" : "destructive"}>
                      {user.status === "active" ? t("common.active", "Active") : t("common.suspended", "Suspended")}
                    </Badge>
                  </TableCell>

                  <TableCell className="text-right">
                    <div className="flex items-center justify-end gap-1.5">
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={() => onToggleSuspend(user.id, user.status)}
                        data-testid={`btn-toggle-suspend-user-${user.id}`}
                        className="h-8 text-xs gap-1"
                      >
                        {user.status === "active" ? (
                          <>
                            <PauseCircle className="h-3.5 w-3.5 text-amber-500" />
                            {t("common.suspend", "Suspend")}
                          </>
                        ) : (
                          <>
                            <PlayCircle className="h-3.5 w-3.5 text-emerald-500" />
                            {t("common.unsuspend", "Activate")}
                          </>
                        )}
                      </Button>

                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={() => setDeleteTarget(user)}
                        data-testid={`btn-delete-reseller-user-${user.id}`}
                        className="h-8 w-8 p-0 text-destructive hover:bg-destructive/10"
                      >
                        <Trash2 className="h-4 w-4" />
                      </Button>
                    </div>
                  </TableCell>
                </TableRow>
              ))
            )}
          </TableBody>
        </Table>
      </div>

      <CreateResellerUserDialog
        open={createDialogOpen}
        plans={plans}
        onClose={() => setCreateDialogOpen(false)}
        onCreate={onCreateUser}
      />

      {/* Delete Confirmation */}
      {deleteTarget && (
        <Dialog open={Boolean(deleteTarget)} onOpenChange={(val) => !val && setDeleteTarget(null)}>
          <DialogContent className="sm:max-w-md">
            <DialogHeader>
              <div className="flex items-center gap-2 text-destructive">
                <AlertTriangle className="h-5 w-5" />
                <DialogTitle>{t("reseller.deleteUserTitle", "Delete Client Account")}</DialogTitle>
              </div>
              <DialogDescription className="text-xs">
                {t(
                  "reseller.deleteUserDesc",
                  "Are you sure you want to terminate account {{username}}? All domains, databases, and files will be permanently deleted.",
                  { username: deleteTarget.username }
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
                data-testid="btn-confirm-delete-user"
              >
                {isDeleting ? t("common.deleting", "Terminating...") : t("common.delete", "Terminate Account")}
              </Button>
            </DialogFooter>
          </DialogContent>
        </Dialog>
      )}
    </div>
  );
}
