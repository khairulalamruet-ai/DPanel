import React, { useState } from "react";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
  Button,
  useToast,
} from "@builtpanel/ui";
import { importDatabase } from "./api";
import { UploadCloud, FileText, CheckCircle2, AlertTriangle } from "lucide-react";
import { DatabaseItem } from "./types";
import { useTranslation } from "@builtpanel/i18n";

interface SqlImportDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  database: DatabaseItem | null;
  onImportComplete: () => void;
}

export function SqlImportDialog({
  open,
  onOpenChange,
  database,
  onImportComplete,
}: SqlImportDialogProps) {
  const { t } = useTranslation();
  const { toast } = useToast();

  const [selectedFile, setSelectedFile] = useState<File | null>(null);
  const [isImporting, setIsImporting] = useState(false);
  const [progress, setProgress] = useState<number>(0);
  const [importStatus, setImportStatus] = useState<"idle" | "uploading" | "executing" | "done">("idle");
  const [errorMsg, setErrorMsg] = useState<string | null>(null);

  const handleFileChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    if (e.target.files && e.target.files[0]) {
      const file = e.target.files[0];
      setSelectedFile(file);
      setErrorMsg(null);
    }
  };

  const handleImport = async () => {
    if (!database || !selectedFile) return;

    try {
      setIsImporting(true);
      setErrorMsg(null);
      setImportStatus("uploading");
      setProgress(0);

      await importDatabase(database.id, selectedFile, (percent) => {
        setProgress(percent);
        setImportStatus(percent < 50 ? "uploading" : "executing");
      });

      setProgress(100);
      setImportStatus("done");

      toast({
        title: t("common.success"),
        description: t("databases.toast.importSuccess", "SQL dump imported successfully into {{db}}", {
          db: database.name,
        }),
        variant: "default",
      });

      onImportComplete();
      setTimeout(() => {
        setIsImporting(false);
        setImportStatus("idle");
        setSelectedFile(null);
        setProgress(0);
        onOpenChange(false);
      }, 1000);
    } catch (err: any) {
      setIsImporting(false);
      setImportStatus("idle");
      setErrorMsg(err?.message || t("databases.errors.importFailed", "Failed to import SQL dump"));
    }
  };

  if (!database) return null;

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-md" onClose={() => onOpenChange(false)}>
        <DialogHeader>
          <div className="flex items-center gap-3">
            <div className="rounded-full bg-primary/10 p-2.5 text-primary">
              <UploadCloud className="h-5 w-5" />
            </div>
            <div>
              <DialogTitle>{t("databases.importDialog.title", "Import SQL Dump")}</DialogTitle>
              <DialogDescription className="mt-0.5 text-xs font-mono">
                {database.name}
              </DialogDescription>
            </div>
          </div>
        </DialogHeader>

        <div className="mt-4 space-y-4">
          {errorMsg && (
            <div className="rounded-md border border-destructive/20 bg-destructive/10 p-3 text-xs text-destructive flex items-center gap-2">
              <AlertTriangle className="h-4 w-4 shrink-0" />
              <span>{errorMsg}</span>
            </div>
          )}

          {/* Warning */}
          <div className="rounded-lg border border-amber-500/20 bg-amber-500/10 p-3 text-xs text-amber-600 dark:text-amber-400">
            <p>
              {t(
                "databases.importDialog.warning",
                "Warning: Importing SQL statements will execute queries directly. Existing tables with matching names may be overwritten or modified."
              )}
            </p>
          </div>

          {/* File Picker */}
          <div className="space-y-1.5">
            <label className="text-xs font-medium text-foreground">
              {t("databases.importDialog.fileLabel", "Select SQL File (.sql or .sql.gz)")}
            </label>
            <div className="rounded-lg border-2 border-dashed border-border p-6 text-center hover:bg-muted/30 transition-colors">
              <input
                type="file"
                id="sql-file-input"
                accept=".sql,.gz,.sql.gz"
                onChange={handleFileChange}
                disabled={isImporting}
                className="hidden"
                data-testid="input-sql-file"
              />
              <label htmlFor="sql-file-input" className="cursor-pointer flex flex-col items-center">
                <FileText className="h-8 w-8 text-muted-foreground mb-2" />
                <span className="text-xs font-medium text-foreground">
                  {selectedFile ? selectedFile.name : t("databases.importDialog.browseFiles", "Click to select SQL dump file")}
                </span>
                <span className="text-[11px] text-muted-foreground mt-1">
                  {selectedFile
                    ? `${(selectedFile.size / (1024 * 1024)).toFixed(2)} MB`
                    : t("databases.importDialog.fileTypes", "Supports raw SQL and gzip compressed dumps up to 500MB")}
                </span>
              </label>
            </div>
          </div>

          {/* Progress Indicator */}
          {isImporting && (
            <div className="space-y-2" data-testid="import-progress-container">
              <div className="flex justify-between text-xs font-medium">
                <span>
                  {importStatus === "uploading"
                    ? t("databases.importDialog.uploading", "Uploading dump file...")
                    : importStatus === "executing"
                    ? t("databases.importDialog.executing", "Executing SQL statements...")
                    : t("common.completed", "Completed")}
                </span>
                <span>{progress}%</span>
              </div>
              <div className="w-full bg-secondary rounded-full h-2 overflow-hidden">
                <div
                  className="bg-primary h-2 rounded-full transition-all duration-300"
                  style={{ width: `${progress}%` }}
                />
              </div>
            </div>
          )}
        </div>

        <DialogFooter className="mt-6 border-t border-border pt-3">
          <Button
            type="button"
            variant="outline"
            size="sm"
            onClick={() => onOpenChange(false)}
            disabled={isImporting}
          >
            {t("common.cancel")}
          </Button>
          <Button
            size="sm"
            onClick={handleImport}
            disabled={!selectedFile || isImporting}
            data-testid="btn-submit-import-sql"
          >
            {isImporting ? t("databases.importDialog.importing", "Importing...") : t("databases.actions.import", "Import Dump")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
