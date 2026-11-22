import React, { useState } from "react";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogFooter,
  Button,
  useToast,
} from "@builtpanel/ui";
import { restoreBackup } from "./api";
import { RotateCcw, ShieldCheck, Database, Folder } from "lucide-react";
import { useTranslation } from "@builtpanel/i18n";
import { BackupItem, RestoreBackupPayload } from "./types";

interface RestoreBackupWizardProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  backup: BackupItem | null;
  onRestored: () => void;
}

export function RestoreBackupWizard({
  open,
  onOpenChange,
  backup,
  onRestored,
}: RestoreBackupWizardProps) {
  const { t } = useTranslation();
  const { toast } = useToast();

  const [restoreMode, setRestoreMode] = useState<"all" | "files_only" | "databases_only" | "selective">("all");
  const [selectedDatabases, setSelectedDatabases] = useState<Set<string>>(new Set());
  const [selectedDomains, setSelectedDomains] = useState<Set<string>>(new Set());
  const [takeSnapshot, setTakeSnapshot] = useState(true);
  const [isSubmitting, setIsSubmitting] = useState(false);

  // Initialize selected databases & domains from backup manifest
  React.useEffect(() => {
    if (open && backup) {
      setRestoreMode("all");
      setSelectedDatabases(new Set(backup.manifest.databases || []));
      setSelectedDomains(new Set(backup.manifest.domains || []));
      setTakeSnapshot(true);
    }
  }, [open, backup]);

  const toggleDatabase = (db: string) => {
    const next = new Set(selectedDatabases);
    if (next.has(db)) next.delete(db);
    else next.add(db);
    setSelectedDatabases(next);
  };

  const toggleDomain = (domain: string) => {
    const next = new Set(selectedDomains);
    if (next.has(domain)) next.delete(domain);
    else next.add(domain);
    setSelectedDomains(next);
  };

  const handleRestoreSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!backup) return;

    try {
      setIsSubmitting(true);
      await restoreBackup(backup, {
        backup_id: backup.id,
        restore_mode: restoreMode,
        selected_databases: restoreMode === "selective" ? Array.from(selectedDatabases) : undefined,
        selected_domains: restoreMode === "selective" ? Array.from(selectedDomains) : undefined,
        take_snapshot_before_restore: takeSnapshot,
      });

      toast({
        title: t("common.success"),
        description: t("backups.restoreStarted"),
      });

      onOpenChange(false);
      onRestored();
    } catch (err: any) {
      toast({
        title: t("common.error"),
        description: err.message || t("common.unknownError"),
        variant: "destructive",
      });
    } finally {
      setIsSubmitting(false);
    }
  };

  if (!backup) return null;

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-[550px]">
        <DialogHeader>
          <div className="flex items-center gap-2">
            <div className="p-2 rounded-lg bg-primary/10 text-primary">
              <RotateCcw className="w-5 h-5" />
            </div>
            <div>
              <DialogTitle data-testid="restore-wizard-title">
                {t("backups.restoreWizard")}
              </DialogTitle>
              <p className="text-xs text-muted-foreground mt-0.5 font-mono">
                {backup.name} • {backup.size_formatted}
              </p>
            </div>
          </div>
        </DialogHeader>

        <form onSubmit={handleRestoreSubmit} className="space-y-4 py-2">
          {/* Mode Selection */}
          <div className="space-y-2">
            <label className="text-sm font-medium">
              {t("backups.whatToRestore")}
            </label>
            <div className="grid grid-cols-2 gap-2 text-xs">
              <button
                type="button"
                onClick={() => setRestoreMode("all")}
                className={`p-3 rounded-lg border text-left flex flex-col gap-1 transition-colors ${
                  restoreMode === "all" ? "border-primary bg-primary/5 font-semibold text-primary" : "border-border hover:bg-muted/50"
                }`}
                data-testid="mode-select-all"
              >
                <span>{t("backups.restoreAll")}</span>
                <span className="text-[11px] font-normal text-muted-foreground">
                  {t("backups.restoreAllDescription")}
                </span>
              </button>

              <button
                type="button"
                onClick={() => setRestoreMode("files_only")}
                className={`p-3 rounded-lg border text-left flex flex-col gap-1 transition-colors ${
                  restoreMode === "files_only" ? "border-primary bg-primary/5 font-semibold text-primary" : "border-border hover:bg-muted/50"
                }`}
                data-testid="mode-select-files"
              >
                <span>{t("backups.filesOnly")}</span>
                <span className="text-[11px] font-normal text-muted-foreground">
                  {t("backups.restoreFilesDescription")}
                </span>
              </button>

              <button
                type="button"
                onClick={() => setRestoreMode("databases_only")}
                className={`p-3 rounded-lg border text-left flex flex-col gap-1 transition-colors ${
                  restoreMode === "databases_only" ? "border-primary bg-primary/5 font-semibold text-primary" : "border-border hover:bg-muted/50"
                }`}
                data-testid="mode-select-databases"
              >
                <span>{t("backups.databasesOnly")}</span>
                <span className="text-[11px] font-normal text-muted-foreground">
                  {t("backups.restoreDatabasesDescription")}
                </span>
              </button>

              <button
                type="button"
                onClick={() => setRestoreMode("selective")}
                className={`p-3 rounded-lg border text-left flex flex-col gap-1 transition-colors ${
                  restoreMode === "selective" ? "border-primary bg-primary/5 font-semibold text-primary" : "border-border hover:bg-muted/50"
                }`}
                data-testid="mode-select-selective"
              >
                <span>{t("backups.selective")}</span>
                <span className="text-[11px] font-normal text-muted-foreground">
                  {t("backups.selectiveDescription")}
                </span>
              </button>
            </div>
          </div>

          {/* Selective Pickers */}
          {restoreMode === "selective" && (
            <div className="space-y-3 p-3 bg-muted/40 border rounded-lg text-xs" data-testid="selective-components-area">
              {backup.manifest.databases.length > 0 && (
                <div className="space-y-1.5">
                  <span className="font-semibold text-foreground flex items-center gap-1.5">
                    <Database className="w-3.5 h-3.5 text-blue-500" />
                    {t("backups.selectDatabases")}
                  </span>
                  <div className="space-y-1 pl-4">
                    {backup.manifest.databases.map((db) => (
                      <label key={db} className="flex items-center gap-2 cursor-pointer font-mono">
                        <input
                          type="checkbox"
                          checked={selectedDatabases.has(db)}
                          onChange={() => toggleDatabase(db)}
                          data-testid={`checkbox-select-db-${db}`}
                        />
                        <span>{db}</span>
                      </label>
                    ))}
                  </div>
                </div>
              )}

              {backup.manifest.domains.length > 0 && (
                <div className="space-y-1.5 pt-2 border-t">
                  <span className="font-semibold text-foreground flex items-center gap-1.5">
                    <Folder className="w-3.5 h-3.5 text-amber-500" />
                    {t("backups.selectDomains")}
                  </span>
                  <div className="space-y-1 pl-4">
                    {backup.manifest.domains.map((dom) => (
                      <label key={dom} className="flex items-center gap-2 cursor-pointer font-mono">
                        <input
                          type="checkbox"
                          checked={selectedDomains.has(dom)}
                          onChange={() => toggleDomain(dom)}
                          data-testid={`checkbox-select-dom-${dom}`}
                        />
                        <span>{dom}</span>
                      </label>
                    ))}
                  </div>
                </div>
              )}
            </div>
          )}

          {/* Safety snapshot checkpoint */}
          <div className="p-3 bg-primary/5 border border-primary/20 rounded-lg">
            <label className="flex items-start gap-2.5 cursor-pointer text-xs">
              <input
                type="checkbox"
                checked={takeSnapshot}
                onChange={(e) => setTakeSnapshot(e.target.checked)}
                className="mt-0.5"
                data-testid="checkbox-safety-snapshot"
              />
              <div className="space-y-0.5">
                <span className="font-semibold text-foreground flex items-center gap-1">
                  <ShieldCheck className="w-3.5 h-3.5 text-primary" />
                  {t("backups.takeSnapshotTitle")}
                </span>
                <p className="text-muted-foreground">
                  {t("backups.takeSnapshotDescription")}
                </p>
              </div>
            </label>
          </div>

          <DialogFooter className="pt-2">
            <Button
              type="button"
              variant="outline"
              onClick={() => onOpenChange(false)}
              disabled={isSubmitting}
            >
              {t("common.cancel")}
            </Button>
            <Button
              type="submit"
              disabled={
                (restoreMode === "selective" && selectedDatabases.size === 0 && selectedDomains.size === 0) ||
                isSubmitting
              }
              data-testid="btn-submit-restore"
            >
              {isSubmitting ? t("common.submitting") : t("backups.startRestore")}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
