import React, { useState, useEffect, useCallback } from "react";
import {
  Card,
  CardHeader,
  CardTitle,
  CardContent,
  Button,
  Badge,
  Table,
  TableHeader,
  TableRow,
  TableHead,
  TableBody,
  TableCell,
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
  useToast,
} from "@builtpanel/ui";
import { formatBytes, loadDatabases, deleteDatabase, deleteDbUser, exportDatabase, phpMyAdminUrl } from "./api";
import {
  Database,
  Users,
  HardDrive,
  Plus,
  RefreshCw,
  ExternalLink,
  Table2,
  UploadCloud,
  Download,
  Trash2,
  Shield,
  Network,
  AlertTriangle,
  Lock,
} from "lucide-react";
import {
  DatabaseItem,
  DatabaseUserItem,
  ClientDatabasesResponse,
} from "./types";
import { CreateDatabaseDialog } from "./CreateDatabaseDialog";
import { CreateDatabaseUserDialog } from "./CreateDatabaseUserDialog";
import { ManageGrantsDialog } from "./ManageGrantsDialog";
import { SqlImportDialog } from "./SqlImportDialog";
import { RemoteAccessDialog } from "./RemoteAccessDialog";
import { DatabaseStudio } from "./studio";
import { useTranslation } from "@builtpanel/i18n";

export function ClientDatabasesPage() {
  const { t } = useTranslation();
  const { toast } = useToast();

  const [activeTab, setActiveTab] = useState<"databases" | "users">("databases");
  const [data, setData] = useState<ClientDatabasesResponse | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  // Dialog states
  const [createDbOpen, setCreateDbOpen] = useState(false);
  const [createUserOpen, setCreateUserOpen] = useState(false);
  const [importDb, setImportDb] = useState<DatabaseItem | null>(null);
  const [studioDb, setStudioDb] = useState<DatabaseItem | null>(null);
  const [manageGrantsUser, setManageGrantsUser] = useState<DatabaseUserItem | null>(null);
  const [remoteAccessOpen, setRemoteAccessOpen] = useState(false);

  // Deletion modals
  const [deletingDb, setDeletingDb] = useState<DatabaseItem | null>(null);
  const [isDeletingDb, setIsDeletingDb] = useState(false);
  const [deletingUser, setDeletingUser] = useState<DatabaseUserItem | null>(null);
  const [isDeletingUser, setIsDeletingUser] = useState(false);

  // SSO state
  const [isSsoLoading, setIsSsoLoading] = useState(false);

  const fetchDatabases = useCallback(async () => {
    try {
      setIsLoading(true);
      setError(null);
      const res = await loadDatabases();
      setData(res);
    } catch (err: any) {
      setError(err?.message || "Failed to load database configuration");
    } finally {
      setIsLoading(false);
    }
  }, []);

  useEffect(() => {
    fetchDatabases();
  }, [fetchDatabases]);

  // Derived tenant prefix from existing databases or user session
  const tenantPrefix = data?.prefix || "";

  // Actions
  const handlePhpMyAdminSso = async (db: DatabaseItem) => {
    try {
      setIsSsoLoading(true);
      const url = await phpMyAdminUrl(db.id);
      // Open in new tab
      window.open(url, "_blank");
    } catch (err: any) {
      toast({
        title: t("common.error"),
        description: err?.message || t("databases.errors.ssoFailed", "Failed to launch phpMyAdmin SSO"),
        variant: "destructive",
      });
    } finally {
      setIsSsoLoading(false);
    }
  };

  const handleExportDb = async (db: DatabaseItem) => {
    try {
      toast({
        title: t("databases.export.preparing", "Generating Database Dump"),
        description: t("databases.export.preparingDesc", "Packaging SQL dump for {{name}}...", { name: db.name }),
        variant: "default",
      });

      const url = await exportDatabase(db.id);

      // Trigger download
      const a = document.createElement("a");
      a.href = url;
      a.download = `${db.name}.sql.gz`;
      document.body.appendChild(a);
      a.click();
      document.body.removeChild(a);

      toast({
        title: t("common.success"),
        description: t("databases.toast.exportReady", "Download started for {{file}}", { file: `${db.name}.sql.gz` }),
        variant: "default",
      });
    } catch (err: any) {
      toast({
        title: t("common.error"),
        description: err?.message || t("databases.errors.exportFailed", "Failed to export database"),
        variant: "destructive",
      });
    }
  };

  const handleDeleteDb = async () => {
    if (!deletingDb) return;
    try {
      setIsDeletingDb(true);
      await deleteDatabase(deletingDb.id);

      toast({
        title: t("common.success"),
        description: t("databases.toast.dbDeleted", "Database {{name}} deleted", { name: deletingDb.name }),
        variant: "default",
      });

      setDeletingDb(null);
      fetchDatabases();
    } catch (err: any) {
      toast({
        title: t("common.error"),
        description: err?.message || t("databases.errors.deleteDbFailed", "Failed to delete database"),
        variant: "destructive",
      });
    } finally {
      setIsDeletingDb(false);
    }
  };

  const handleDeleteUser = async () => {
    if (!deletingUser) return;
    try {
      setIsDeletingUser(true);
      await deleteDbUser(deletingUser.id);

      toast({
        title: t("common.success"),
        description: t("databases.toast.userDeleted", "Database user {{username}} deleted", { username: deletingUser.username }),
        variant: "default",
      });

      setDeletingUser(null);
      fetchDatabases();
    } catch (err: any) {
      toast({
        title: t("common.error"),
        description: err?.message || t("databases.errors.deleteUserFailed", "Failed to delete database user"),
        variant: "destructive",
      });
    } finally {
      setIsDeletingUser(false);
    }
  };

  if (isLoading) {
    return (
      <div className="flex h-96 items-center justify-center">
        <p className="text-sm text-muted-foreground">{t("common.loading")}</p>
      </div>
    );
  }

  if (error || !data) {
    return (
      <div className="rounded-lg border border-destructive/20 bg-destructive/10 p-6 text-center">
        <p className="text-sm text-destructive">{error || t("databases.errors.loadFailed", "Failed to load databases")}</p>
      </div>
    );
  }

  if (studioDb) {
    return <DatabaseStudio database={studioDb} onClose={() => setStudioDb(null)} />;
  }

  const { databases, users, limits } = data;
  const isDbLimitReached = limits.max_databases !== -1 && limits.used_databases >= limits.max_databases;
  const isUserLimitReached = limits.max_db_users !== -1 && limits.used_db_users >= limits.max_db_users;

  return (
    <div className="space-y-6">
      {/* Page Header */}
      <div className="flex flex-col gap-4 sm:flex-row sm:items-center sm:justify-between">
        <div>
          <h1 className="text-2xl font-bold tracking-tight text-foreground" data-testid="databases-title">
            {t("databases.title", "Databases (MySQL / MariaDB)")}
          </h1>
          <p className="text-sm text-muted-foreground mt-1">
            {t("databases.subtitle", "Manage databases, SQL users, permissions, import/export dumps, and phpMyAdmin access.")}
          </p>
        </div>

        <div className="flex flex-wrap items-center gap-2">
          {/* phpMyAdmin SSO (T07.09) */}
          <Button
            variant="outline"
            size="sm"
            onClick={() => {
              const first = databases.find((d) => d.engine !== "postgres" && d.status === "ready");
              if (first) void handlePhpMyAdminSso(first);
            }}
            disabled={isSsoLoading || !databases.some((d) => d.engine !== "postgres" && d.status === "ready")}
            className="text-xs"
            data-testid="btn-phpmyadmin-sso"
          >
            <ExternalLink className="h-3.5 w-3.5 mr-1.5 text-primary" />
            {isSsoLoading ? t("common.loading") : "phpMyAdmin SSO"}
          </Button>

          {/* Remote Access CIDR Allowlist (T07.05) */}
          <Button
            variant="outline"
            size="sm"
            onClick={() => setRemoteAccessOpen(true)}
            className="text-xs"
            data-testid="btn-remote-access"
          >
            <Network className="h-3.5 w-3.5 mr-1.5" />
            {t("databases.actions.remoteAccess", "Remote Access")}
          </Button>

          <Button
            variant="ghost"
            size="sm"
            onClick={fetchDatabases}
            className="h-9 w-9 p-0"
            title={t("common.refresh")}
            data-testid="btn-refresh-databases"
          >
            <RefreshCw className="h-4 w-4" />
          </Button>
        </div>
      </div>

      {/* Metrics Cards */}
      <div className="grid grid-cols-1 sm:grid-cols-3 gap-4">
        {/* Databases Count */}
        <Card>
          <CardContent className="p-4 flex items-center justify-between">
            <div>
              <p className="text-xs font-medium text-muted-foreground">
                {t("databases.metrics.databases", "Databases Provisioned")}
              </p>
              <div className="flex items-baseline gap-2 mt-1">
                <span className="text-2xl font-bold font-mono text-foreground" data-testid="metric-db-count">
                  {limits.used_databases}
                </span>
                <span className="text-xs text-muted-foreground font-mono">
                  / {limits.max_databases === -1 ? "∞" : limits.max_databases} {t("databases.metrics.limit", "allowed")}
                </span>
              </div>
            </div>
            <div className="rounded-full bg-primary/10 p-2.5 text-primary">
              <Database className="h-5 w-5" />
            </div>
          </CardContent>
        </Card>

        {/* Database Users Count */}
        <Card>
          <CardContent className="p-4 flex items-center justify-between">
            <div>
              <p className="text-xs font-medium text-muted-foreground">
                {t("databases.metrics.users", "Database Users")}
              </p>
              <div className="flex items-baseline gap-2 mt-1">
                <span className="text-2xl font-bold font-mono text-foreground" data-testid="metric-user-count">
                  {limits.used_db_users}
                </span>
                <span className="text-xs text-muted-foreground font-mono">
                  / {limits.max_db_users === -1 ? "∞" : limits.max_db_users} {t("databases.metrics.limit", "allowed")}
                </span>
              </div>
            </div>
            <div className="rounded-full bg-primary/10 p-2.5 text-primary">
              <Users className="h-5 w-5" />
            </div>
          </CardContent>
        </Card>

        {/* Total Disk Usage */}
        <Card>
          <CardContent className="p-4 flex items-center justify-between">
            <div>
              <p className="text-xs font-medium text-muted-foreground">
                {t("databases.metrics.storage", "Total Database Storage")}
              </p>
              <div className="flex items-baseline gap-2 mt-1">
                <span className="text-2xl font-bold font-mono text-foreground" data-testid="metric-db-size">
                  {formatBytes(limits.used_db_size_bytes)}
                </span>
              </div>
            </div>
            <div className="rounded-full bg-primary/10 p-2.5 text-primary">
              <HardDrive className="h-5 w-5" />
            </div>
          </CardContent>
        </Card>
      </div>

      {/* Main Subtabs: Databases vs Users */}
      <div className="flex border-b border-border gap-2" role="tablist">
        <button
          type="button"
          role="tab"
          aria-selected={activeTab === "databases"}
          onClick={() => setActiveTab("databases")}
          className={`flex items-center gap-2 px-4 py-2.5 text-sm font-medium border-b-2 transition-colors ${
            activeTab === "databases"
              ? "border-primary text-primary"
              : "border-transparent text-muted-foreground hover:text-foreground hover:border-border"
          }`}
          data-testid="subtab-databases"
        >
          <Database className="h-4 w-4" />
          <span>{t("databases.tabs.databases", "Databases")}</span>
          <span className="text-xs px-1.5 py-0.5 rounded bg-muted font-mono">
            {databases.length}
          </span>
        </button>

        <button
          type="button"
          role="tab"
          aria-selected={activeTab === "users"}
          onClick={() => setActiveTab("users")}
          className={`flex items-center gap-2 px-4 py-2.5 text-sm font-medium border-b-2 transition-colors ${
            activeTab === "users"
              ? "border-primary text-primary"
              : "border-transparent text-muted-foreground hover:text-foreground hover:border-border"
          }`}
          data-testid="subtab-users"
        >
          <Users className="h-4 w-4" />
          <span>{t("databases.tabs.users", "Database Users")}</span>
          <span className="text-xs px-1.5 py-0.5 rounded bg-muted font-mono">
            {users.length}
          </span>
        </button>
      </div>

      {/* Databases Tab Content */}
      {activeTab === "databases" && (
        <div className="space-y-4">
          <div className="flex items-center justify-between">
            <h2 className="text-base font-semibold text-foreground">
              {t("databases.table.title", "All Databases")}
            </h2>
            <Button
              size="sm"
              onClick={() => setCreateDbOpen(true)}
              disabled={isDbLimitReached}
              data-testid="btn-open-create-db"
            >
              <Plus className="h-4 w-4 mr-1.5" />
              {t("databases.actions.createDatabase", "Create Database")}
            </Button>
          </div>

          <div className="rounded-lg border border-border bg-card overflow-hidden">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>{t("databases.table.name", "Database Name")}</TableHead>
                  <TableHead>{t("databases.table.size", "Size")}</TableHead>
                  <TableHead>{t("databases.table.engine", "Engine")}</TableHead>
                  <TableHead>{t("common.status", "Status")}</TableHead>
                  <TableHead>{t("databases.table.assignedUsers", "Assigned Users")}</TableHead>
                  <TableHead className="text-right">{t("common.actions", "Actions")}</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {databases.length === 0 ? (
                  <TableRow>
                    <TableCell colSpan={6} className="text-center py-8 text-muted-foreground">
                      {t("databases.table.noDatabases", "No databases created yet.")}
                    </TableCell>
                  </TableRow>
                ) : (
                  databases.map((db) => (
                    <TableRow key={db.id} data-testid={`db-row-${db.name}`}>
                      <TableCell className="font-mono text-xs font-semibold text-foreground">
                        <div className="flex items-center gap-2">
                          <Database className="h-3.5 w-3.5 text-primary" />
                          <span>{db.name}</span>
                        </div>
                      </TableCell>
                      <TableCell className="font-mono text-xs text-muted-foreground">
                        {db.size_formatted}
                      </TableCell>
                      <TableCell className="font-mono text-xs text-muted-foreground">
                        {db.engine === "postgres" ? "PostgreSQL" : "MariaDB"}
                      </TableCell>
                      <TableCell className="font-mono text-[11px] text-muted-foreground">
                        {db.status}
                      </TableCell>
                      <TableCell>
                        <div className="flex flex-wrap gap-1">
                          {db.assigned_users.length === 0 ? (
                            <span className="text-xs text-muted-foreground italic">None</span>
                          ) : (
                            db.assigned_users.map((u) => (
                              <Badge key={u} variant="secondary" className="text-[10px] font-mono py-0">
                                {u}
                              </Badge>
                            ))
                          )}
                        </div>
                      </TableCell>
                      <TableCell className="text-right">
                        <div className="flex items-center justify-end gap-1">
                          {/* Database studio (MariaDB and PostgreSQL on the node's shared servers) */}
                          {db.status === "ready" && (
                            <Button
                              variant="ghost"
                              size="sm"
                              onClick={() => setStudioDb(db)}
                              className="h-8 w-8 p-0"
                              title={t("databases.actions.studio", "Open the database studio")}
                              data-testid={`btn-studio-${db.id}`}
                            >
                              <Table2 className="h-4 w-4" />
                            </Button>
                          )}

                          {/* SQL Import */}
                          <Button
                            variant="ghost"
                            size="sm"
                            onClick={() => setImportDb(db)}
                            className="h-8 w-8 p-0"
                            title={t("databases.actions.import", "Import SQL Dump")}
                            data-testid={`btn-import-sql-${db.id}`}
                          >
                            <UploadCloud className="h-4 w-4" />
                          </Button>

                          {/* SQL Export */}
                          <Button
                            variant="ghost"
                            size="sm"
                            onClick={() => handleExportDb(db)}
                            className="h-8 w-8 p-0"
                            title={t("databases.actions.export", "Export / Download Dump (.sql.gz)")}
                            data-testid={`btn-export-sql-${db.id}`}
                          >
                            <Download className="h-4 w-4" />
                          </Button>

                          {/* Delete DB */}
                          <Button
                            variant="ghost"
                            size="sm"
                            onClick={() => setDeletingDb(db)}
                            className="h-8 w-8 p-0 text-destructive hover:bg-destructive/10"
                            title={t("common.delete")}
                            data-testid={`btn-delete-db-${db.id}`}
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
        </div>
      )}

      {/* Database Users Tab Content */}
      {activeTab === "users" && (
        <div className="space-y-4">
          <div className="flex items-center justify-between">
            <h2 className="text-base font-semibold text-foreground">
              {t("databases.usersTable.title", "Database Users & Privileges")}
            </h2>
            <Button
              size="sm"
              onClick={() => setCreateUserOpen(true)}
              disabled={isUserLimitReached}
              data-testid="btn-open-create-user"
            >
              <Plus className="h-4 w-4 mr-1.5" />
              {t("databases.actions.createUser", "Create Database User")}
            </Button>
          </div>

          <div className="rounded-lg border border-border bg-card overflow-hidden">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>{t("databases.fields.username", "Username")}</TableHead>
                  <TableHead>{t("databases.fields.connectionHost", "Host")}</TableHead>
                  <TableHead>{t("databases.table.assignedDatabases", "Database Grants")}</TableHead>
                  <TableHead className="text-right">{t("common.actions", "Actions")}</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {users.length === 0 ? (
                  <TableRow>
                    <TableCell colSpan={4} className="text-center py-8 text-muted-foreground">
                      {t("databases.usersTable.noUsers", "No database users created yet.")}
                    </TableCell>
                  </TableRow>
                ) : (
                  users.map((usr) => (
                    <TableRow key={usr.id} data-testid={`user-row-${usr.username}`}>
                      <TableCell className="font-mono text-xs font-semibold text-foreground">
                        <div className="flex items-center gap-2">
                          <Users className="h-3.5 w-3.5 text-primary" />
                          <span>{usr.username}</span>
                        </div>
                      </TableCell>
                      <TableCell className="font-mono text-xs text-muted-foreground">
                        {usr.engine === "postgres" ? "PostgreSQL" : "MariaDB"}
                      </TableCell>
                      <TableCell>
                        <div className="flex flex-wrap gap-1.5">
                          {usr.grants.length === 0 ? (
                            <span className="text-xs text-muted-foreground italic">No databases assigned</span>
                          ) : (
                            usr.grants.map((g) => (
                              <Badge key={g.db_id} variant="outline" className="text-[10px] font-mono">
                                {g.db_name} ({g.access.replace("_", " ")})
                              </Badge>
                            ))
                          )}
                        </div>
                      </TableCell>
                      <TableCell className="text-right">
                        <div className="flex items-center justify-end gap-1">
                          {/* Manage Grants & Privileges */}
                          <Button
                            variant="ghost"
                            size="sm"
                            onClick={() => setManageGrantsUser(usr)}
                            className="h-8 w-8 p-0"
                            title={t("databases.actions.manageGrants", "Manage Grants & Privileges")}
                            data-testid={`btn-manage-grants-${usr.id}`}
                          >
                            <Shield className="h-4 w-4 text-primary" />
                          </Button>

                          {/* Delete User */}
                          <Button
                            variant="ghost"
                            size="sm"
                            onClick={() => setDeletingUser(usr)}
                            className="h-8 w-8 p-0 text-destructive hover:bg-destructive/10"
                            title={t("common.delete")}
                            data-testid={`btn-delete-user-${usr.id}`}
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
        </div>
      )}

      {/* Create Database Dialog */}
      <CreateDatabaseDialog
        open={createDbOpen}
        onOpenChange={setCreateDbOpen}
        prefix={tenantPrefix}
        onCreated={() => fetchDatabases()}
        isLimitReached={isDbLimitReached}
      />

      {/* Create Database User Dialog */}
      <CreateDatabaseUserDialog
        open={createUserOpen}
        onOpenChange={setCreateUserOpen}
        prefix={tenantPrefix}
        databases={databases}
        onUserCreated={() => fetchDatabases()}
        isLimitReached={isUserLimitReached}
      />

      {/* Manage Grants Dialog */}
      <ManageGrantsDialog
        open={!!manageGrantsUser}
        onOpenChange={(open) => !open && setManageGrantsUser(null)}
        user={manageGrantsUser}
        databases={databases}
        onGrantsUpdated={() => fetchDatabases()}
      />

      {/* SQL Import Dialog */}
      <SqlImportDialog
        open={!!importDb}
        onOpenChange={(open) => !open && setImportDb(null)}
        database={importDb}
        onImportComplete={() => fetchDatabases()}
      />

      {/* Remote Hosts Dialog */}
      <RemoteAccessDialog
        open={remoteAccessOpen}
        onOpenChange={setRemoteAccessOpen}
        users={users}
        onHostsUpdated={() => fetchDatabases()}
      />

      {/* Delete Database Confirmation Dialog */}
      <Dialog open={!!deletingDb} onOpenChange={(open) => !open && setDeletingDb(null)}>
        <DialogContent className="max-w-md" onClose={() => setDeletingDb(null)}>
          <DialogHeader>
            <div className="flex items-center gap-2 text-destructive">
              <AlertTriangle className="h-5 w-5" />
              <DialogTitle>{t("databases.deleteDb.title", "Delete Database")}</DialogTitle>
            </div>
            <DialogDescription className="mt-2 text-xs">
              {t(
                "databases.deleteDb.confirm",
                "Are you sure you want to permanently drop database {{name}}? All tables, views, and data will be permanently deleted.",
                { name: deletingDb?.name }
              )}
            </DialogDescription>
          </DialogHeader>

          <DialogFooter className="mt-6 border-t border-border pt-3">
            <Button
              type="button"
              variant="outline"
              size="sm"
              onClick={() => setDeletingDb(null)}
              disabled={isDeletingDb}
            >
              {t("common.cancel")}
            </Button>
            <Button
              variant="destructive"
              size="sm"
              onClick={handleDeleteDb}
              disabled={isDeletingDb}
              data-testid="btn-confirm-delete-db"
            >
              {isDeletingDb ? t("common.deleting") : t("common.delete")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* Delete Database User Confirmation Dialog */}
      <Dialog open={!!deletingUser} onOpenChange={(open) => !open && setDeletingUser(null)}>
        <DialogContent className="max-w-md" onClose={() => setDeletingUser(null)}>
          <DialogHeader>
            <div className="flex items-center gap-2 text-destructive">
              <AlertTriangle className="h-5 w-5" />
              <DialogTitle>{t("databases.deleteUser.title", "Delete Database User")}</DialogTitle>
            </div>
            <DialogDescription className="mt-2 text-xs">
              {t(
                "databases.deleteUser.confirm",
                "Are you sure you want to remove SQL user {{username}}? Any applications connecting using these credentials will be disconnected.",
                { username: deletingUser?.username }
              )}
            </DialogDescription>
          </DialogHeader>

          <DialogFooter className="mt-6 border-t border-border pt-3">
            <Button
              type="button"
              variant="outline"
              size="sm"
              onClick={() => setDeletingUser(null)}
              disabled={isDeletingUser}
            >
              {t("common.cancel")}
            </Button>
            <Button
              variant="destructive"
              size="sm"
              onClick={handleDeleteUser}
              disabled={isDeletingUser}
              data-testid="btn-confirm-delete-user"
            >
              {isDeletingUser ? t("common.deleting") : t("common.delete")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
