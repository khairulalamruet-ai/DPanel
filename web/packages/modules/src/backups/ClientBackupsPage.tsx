import React, { useState, useEffect, useCallback } from "react";
import {
  Card,
  CardHeader,
  CardTitle,
  CardContent,
  Button,
  Badge,
  Input,
  Table,
  TableHeader,
  TableRow,
  TableHead,
  TableBody,
  TableCell,
  useToast,
} from "@builtpanel/ui";
import { listBackups, deleteBackup, exportBackup } from "./api";
import { useTranslation } from "@builtpanel/i18n";
import {
  Archive,
  RefreshCw,
  Search,
  Download,
  RotateCcw,
  Trash2,
  HardDrive,
  Clock,
  Shield,
  Plus,
} from "lucide-react";
import { BackupItem } from "./types";
import { CreateBackupDialog } from "./CreateBackupDialog";
import { RestoreBackupWizard } from "./RestoreBackupWizard";
import { BackupScheduleCard } from "./BackupScheduleCard";

export function ClientBackupsPage() {
  const { t } = useTranslation();
  const { toast } = useToast();

  const [backups, setBackups] = useState<BackupItem[]>([]);
  const [searchQuery, setSearchQuery] = useState("");
  const [isLoading, setIsLoading] = useState(false);

  // Dialog states
  const [isCreateOpen, setIsCreateOpen] = useState(false);
  const [restoreTarget, setRestoreTarget] = useState<BackupItem | null>(null);

  const fetchBackups = useCallback(async () => {
    try {
      setIsLoading(true);
      const res = await listBackups();
      setBackups(res);
    } catch (err: any) {
      toast({
        title: t("common.error"),
        description: err.message || t("common.unknownError"),
        variant: "destructive",
      });
    } finally {
      setIsLoading(false);
    }
  }, [t, toast]);

  useEffect(() => {
    fetchBackups();
  }, [fetchBackups]);

  const handleDelete = async (backup: BackupItem) => {
    if (!window.confirm(t("backups.confirmDelete", { name: backup.name }))) {
      return;
    }

    try {
      await deleteBackup(backup.id);

      toast({
        title: t("common.success"),
        description: t("backups.deletedSuccess", { name: backup.name }),
      });
      fetchBackups();
    } catch (err: any) {
      toast({
        title: t("common.error"),
        description: err.message || t("common.unknownError"),
        variant: "destructive",
      });
    }
  };

  const handleDownload = async (backup: BackupItem) => {
    try {
      // A download is an account export: one file, sealed with a passphrase, that another panel can import.
      const passphrase = window.prompt(t("backups.exportPassphrase", "Choose a passphrase for the export file (keep it: it is needed to open the file)"));
      if (!passphrase) return;
      toast({
        title: t("backups.exportPreparing", "Preparing the export"),
        description: backup.name,
      });
      const url = await exportBackup(backup.id, passphrase);
      toast({
        title: t("common.success"),
        description: t("backups.downloadStarted", { name: backup.name }),
      });
      window.open(url, "_blank");
    } catch (err: any) {
      toast({
        title: t("common.error"),
        description: err.message || t("common.unknownError"),
        variant: "destructive",
      });
    }
  };

  const totalBytes = backups.reduce((sum, b) => sum + (b.size_bytes || 0), 0);
  const totalStorageFormatted =
    totalBytes > 1073741824
      ? `${(totalBytes / 1073741824).toFixed(1)} GB`
      : `${(totalBytes / 1048576).toFixed(1)} MB`;

  const filteredBackups = backups.filter(
    (b) =>
      b.name.toLowerCase().includes(searchQuery.toLowerCase()) ||
      b.type.toLowerCase().includes(searchQuery.toLowerCase()) ||
      b.destination.toLowerCase().includes(searchQuery.toLowerCase())
  );

  return (
    <div className="p-6 space-y-6" data-testid="client-backups-page">
      {/* Header */}
      <div className="flex flex-col sm:flex-row sm:items-center sm:justify-between gap-4">
        <div>
          <h1 className="text-2xl font-bold tracking-tight" data-testid="backups-page-title">
            {t("backups.title")}
          </h1>
          <p className="text-sm text-muted-foreground">
            {t("backups.description")}
          </p>
        </div>

        <div className="flex items-center gap-2">
          <Button
            size="sm"
            className="gap-2"
            onClick={() => setIsCreateOpen(true)}
            data-testid="btn-open-create-backup"
          >
            <Plus className="w-4 h-4" />
            <span>{t("backups.takeBackupNow")}</span>
          </Button>

          <Button
            variant="outline"
            size="sm"
            className="gap-2"
            onClick={fetchBackups}
            disabled={isLoading}
            data-testid="btn-refresh-backups"
          >
            <RefreshCw className={`w-4 h-4 ${isLoading ? "animate-spin" : ""}`} />
            <span>{t("common.refresh")}</span>
          </Button>
        </div>
      </div>

      {/* Metric Summary Cards */}
      <div className="grid grid-cols-1 sm:grid-cols-3 gap-4">
        <Card data-testid="card-metric-total-backups">
          <CardHeader className="py-3 px-4 border-b flex flex-row items-center justify-between space-y-0">
            <span className="text-xs font-medium text-muted-foreground">{t("backups.totalBackups")}</span>
            <Archive className="w-4 h-4 text-primary" />
          </CardHeader>
          <CardContent className="p-4">
            <div className="text-2xl font-bold" data-testid="metric-backup-count">{backups.length}</div>
          </CardContent>
        </Card>

        <Card data-testid="card-metric-storage-used">
          <CardHeader className="py-3 px-4 border-b flex flex-row items-center justify-between space-y-0">
            <span className="text-xs font-medium text-muted-foreground">{t("backups.storageConsumed")}</span>
            <HardDrive className="w-4 h-4 text-primary" />
          </CardHeader>
          <CardContent className="p-4">
            <div className="text-2xl font-bold" data-testid="metric-backup-storage">{totalStorageFormatted}</div>
          </CardContent>
        </Card>

        <Card data-testid="card-metric-last-backup">
          <CardHeader className="py-3 px-4 border-b flex flex-row items-center justify-between space-y-0">
            <span className="text-xs font-medium text-muted-foreground">{t("backups.lastBackup")}</span>
            <Clock className="w-4 h-4 text-primary" />
          </CardHeader>
          <CardContent className="p-4">
            <div className="text-sm font-semibold truncate" data-testid="metric-last-backup-date">
              {backups[0] ? new Date(backups[0].created_at).toLocaleDateString() : t("backups.noBackupsYet")}
            </div>
          </CardContent>
        </Card>
      </div>

      {/* The account's own schedule */}
      <BackupScheduleCard />

      {/* Backups List Table */}
      <Card>
        <CardHeader className="py-4 px-6 border-b flex flex-col sm:flex-row sm:items-center sm:justify-between gap-3">
          <CardTitle className="text-base font-semibold flex items-center gap-2">
            <Archive className="w-5 h-5 text-primary" />
            <span data-testid="backups-heading">{t("backups.availableBackups")}</span>
            <Badge variant="secondary" className="ml-1 text-xs">
              {backups.length}
            </Badge>
          </CardTitle>

          {/* Search */}
          <div className="relative w-full sm:w-64">
            <Search className="w-4 h-4 absolute left-2.5 top-2.5 text-muted-foreground" />
            <Input
              placeholder={t("common.search")}
              value={searchQuery}
              onChange={(e) => setSearchQuery(e.target.value)}
              className="pl-8 h-9 text-xs"
              data-testid="input-search-backups"
            />
          </div>
        </CardHeader>

        <div className="overflow-x-auto">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>{t("backups.name")}</TableHead>
                <TableHead className="w-28">{t("backups.type")}</TableHead>
                <TableHead className="w-28">{t("backups.size")}</TableHead>
                <TableHead className="w-36">{t("backups.destination")}</TableHead>
                <TableHead className="w-28 text-center">{t("common.status")}</TableHead>
                <TableHead className="w-32">{t("backups.date")}</TableHead>
                <TableHead className="w-36 text-right">{t("common.actions")}</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {isLoading ? (
                <TableRow>
                  <TableCell colSpan={7} className="text-center py-10 text-muted-foreground">
                    {t("common.loading")}
                  </TableCell>
                </TableRow>
              ) : filteredBackups.length === 0 ? (
                <TableRow>
                  <TableCell colSpan={7} className="text-center py-10 text-muted-foreground">
                    {t("backups.noBackupsFound")}
                  </TableCell>
                </TableRow>
              ) : (
                filteredBackups.map((item) => (
                  <TableRow
                    key={item.id}
                    className="hover:bg-muted/50 transition-colors"
                    data-testid={`backup-row-${item.id}`}
                  >
                    <TableCell>
                      <div className="space-y-1">
                        <div className="flex items-center gap-2">
                          <Archive className="w-4 h-4 text-primary shrink-0" />
                          <span
                            className="font-mono text-sm font-semibold text-foreground truncate max-w-xs"
                            data-testid={`backup-name-${item.id}`}
                          >
                            {item.name}
                          </span>
                        </div>
                        <div className="flex items-center gap-1.5 text-[11px] font-mono text-muted-foreground">
                          <Shield className="w-3 h-3 text-emerald-500" />
                          <span className="truncate max-w-[200px]" title={item.sha256}>
                            SHA-256: {item.sha256.substring(0, 12)}...
                          </span>
                        </div>
                      </div>
                    </TableCell>

                    <TableCell>
                      <Badge variant="outline" className="capitalize text-xs">
                        {item.type}
                      </Badge>
                    </TableCell>

                    <TableCell className="font-mono text-xs font-semibold">
                      {item.size_formatted}
                    </TableCell>

                    <TableCell className="text-xs text-muted-foreground">
                      {item.destination}
                    </TableCell>

                    <TableCell className="text-center">
                      <Badge
                        variant={item.status === "completed" ? "success" : item.status === "in_progress" ? "secondary" : "destructive"}
                        className="text-xs capitalize"
                        data-testid={`backup-status-${item.id}`}
                      >
                        {item.status === "completed" ? t("common.active") : item.status}
                      </Badge>
                    </TableCell>

                    <TableCell className="text-xs text-muted-foreground">
                      {new Date(item.created_at).toLocaleDateString()}
                    </TableCell>

                    <TableCell className="text-right">
                      <div className="flex items-center justify-end gap-1">
                        <Button
                          size="sm"
                          variant="outline"
                          className="h-7 px-2 text-xs gap-1"
                          onClick={() => setRestoreTarget(item)}
                          disabled={item.status !== "completed"}
                          title={t("backups.restore")}
                          data-testid={`btn-restore-${item.id}`}
                        >
                          <RotateCcw className="w-3.5 h-3.5 text-primary" />
                          <span>{t("backups.restore")}</span>
                        </Button>

                        <Button
                          size="sm"
                          variant="ghost"
                          className="h-7 w-7 p-0"
                          onClick={() => handleDownload(item)}
                          title={t("common.download")}
                          data-testid={`btn-download-${item.id}`}
                        >
                          <Download className="w-3.5 h-3.5" />
                        </Button>

                        <Button
                          size="sm"
                          variant="destructive"
                          className="h-7 w-7 p-0"
                          title={t("common.delete")}
                          onClick={() => handleDelete(item)}
                          data-testid={`btn-delete-backup-${item.id}`}
                        >
                          <Trash2 className="w-3.5 h-3.5" />
                        </Button>
                      </div>
                    </TableCell>
                  </TableRow>
                ))
              )}
            </TableBody>
          </Table>
        </div>
      </Card>

      {/* Dialogs */}
      <CreateBackupDialog
        open={isCreateOpen}
        onOpenChange={setIsCreateOpen}
        onCreated={fetchBackups}
      />

      <RestoreBackupWizard
        open={!!restoreTarget}
        onOpenChange={(open) => !open && setRestoreTarget(null)}
        backup={restoreTarget}
        onRestored={fetchBackups}
      />
    </div>
  );
}
