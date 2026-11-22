import React, { useState } from "react";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
  Button,
  Input,
  useToast,
} from "@builtpanel/ui";
import { createDatabase } from "./api";
import { Database, AlertCircle } from "lucide-react";
import { DatabaseItem } from "./types";
import { useTranslation } from "@builtpanel/i18n";

interface CreateDatabaseDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  prefix: string;
  onCreated: (newDb: DatabaseItem) => void;
  isLimitReached: boolean;
}

export function CreateDatabaseDialog({
  open,
  onOpenChange,
  prefix,
  onCreated,
  isLimitReached,
}: CreateDatabaseDialogProps) {
  const { t } = useTranslation();
  const { toast } = useToast();

  const [dbName, setDbName] = useState("");
  const [engine, setEngine] = useState<"mysql" | "postgres">("mysql");
  const [isSubmitting, setIsSubmitting] = useState(false);
  const [validationError, setValidationError] = useState<string | null>(null);

  const cleanName = dbName.trim().toLowerCase().replace(/[^a-z0-9_]/g, "");
  const fullName = `${prefix}${cleanName}`;

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!cleanName) {
      setValidationError(t("databases.validation.nameRequired", "Database name is required"));
      return;
    }
    if (cleanName.length > 32) {
      setValidationError(t("databases.validation.nameTooLong", "Database name must be 32 characters or fewer"));
      return;
    }

    try {
      setIsSubmitting(true);
      setValidationError(null);

      const res = await createDatabase(fullName, prefix, engine);

      toast({
        title: t("common.success"),
        description: t("databases.toast.dbCreated", "Database {{name}} created successfully", {
          name: res.name,
        }),
        variant: "default",
      });

      onCreated(res);
      setDbName("");
      onOpenChange(false);
    } catch (err: any) {
      setValidationError(err?.message || t("databases.errors.createFailed", "Failed to create database"));
    } finally {
      setIsSubmitting(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-md" onClose={() => onOpenChange(false)}>
        <DialogHeader>
          <div className="flex items-center gap-3">
            <div className="rounded-full bg-primary/10 p-2.5 text-primary">
              <Database className="h-5 w-5" />
            </div>
            <div>
              <DialogTitle>{t("databases.createDialog.title", "Create Database")}</DialogTitle>
              <DialogDescription className="mt-0.5 text-xs">
                {t("databases.createDialog.desc", "Databases are provisioned on the shared isolated database daemon.")}
              </DialogDescription>
            </div>
          </div>
        </DialogHeader>

        {isLimitReached ? (
          <div className="mt-4 rounded-lg border border-destructive/20 bg-destructive/10 p-4 text-xs text-destructive flex items-center gap-2">
            <AlertCircle className="h-4 w-4 shrink-0" />
            <span>
              {t(
                "databases.createDialog.limitReached",
                "You have reached the maximum number of databases allocated for your hosting plan."
              )}
            </span>
          </div>
        ) : (
          <form onSubmit={handleSubmit} className="mt-4 space-y-4">
            {validationError && (
              <div className="rounded-md border border-destructive/20 bg-destructive/10 p-3 text-xs text-destructive">
                {validationError}
              </div>
            )}

            {/* Database Name with enforced prefix */}
            <div className="space-y-1.5">
              <label className="text-xs font-medium text-foreground">
                {t("databases.fields.name", "Database Name")}
              </label>
              <div className="flex items-center rounded-md border border-input bg-background focus-within:ring-1 focus-within:ring-ring overflow-hidden">
                <span className="bg-muted px-3 py-2 text-xs font-mono text-muted-foreground select-none border-r border-input">
                  {prefix}
                </span>
                <input
                  type="text"
                  value={dbName}
                  onChange={(e) => {
                    setDbName(e.target.value);
                    setValidationError(null);
                  }}
                  placeholder="myapp_db"
                  maxLength={32}
                  className="flex-1 bg-transparent px-3 py-2 text-xs font-mono text-foreground focus:outline-none"
                  data-testid="input-db-name"
                  autoFocus
                />
              </div>
              <p className="text-[11px] text-muted-foreground font-mono">
                {t("databases.createDialog.fullDbName", "Full database name")}: <strong>{fullName}</strong>
              </p>
            </div>

            {/* Engine */}
            <div className="space-y-1.5">
              <label className="text-xs font-medium text-foreground">
                {t("databases.fields.engine", "Database engine")}
              </label>
              <select
                value={engine}
                onChange={(e) => setEngine(e.target.value as "mysql" | "postgres")}
                className="w-full h-9 rounded-md border border-input bg-background px-2.5 text-xs"
                data-testid="select-db-engine"
              >
                <option value="mysql">MariaDB (MySQL)</option>
                <option value="postgres">PostgreSQL</option>
              </select>
            </div>

            <DialogFooter className="mt-6 border-t border-border pt-3">
              <Button
                type="button"
                variant="outline"
                size="sm"
                onClick={() => onOpenChange(false)}
                disabled={isSubmitting}
                data-testid="btn-cancel-create-db"
              >
                {t("common.cancel")}
              </Button>
              <Button
                type="submit"
                size="sm"
                disabled={!cleanName || isSubmitting}
                data-testid="btn-submit-create-db"
              >
                {isSubmitting ? t("common.creating") : t("databases.actions.createDatabase", "Create Database")}
              </Button>
            </DialogFooter>
          </form>
        )}
      </DialogContent>
    </Dialog>
  );
}
