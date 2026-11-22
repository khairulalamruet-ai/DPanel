import React, { useState, useEffect } from "react";
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
import { setGrant, type DbAccess } from "./api";
import { Shield, Check, Lock } from "lucide-react";
import { DatabaseUserItem, DatabaseItem } from "./types";
import { useTranslation } from "@builtpanel/i18n";

interface ManageGrantsDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  user: DatabaseUserItem | null;
  databases: DatabaseItem[];
  onGrantsUpdated: (updatedUser: DatabaseUserItem) => void;
}

const ACCESS_LEVELS: { value: DbAccess; testId: string; label: string; hint: string }[] = [
  { value: "all", testId: "preset-all", label: "All privileges", hint: "Data, structure and everything else" },
  { value: "read_write", testId: "preset-readwrite", label: "Read / Write", hint: "Read and change data" },
  { value: "read_only", testId: "preset-readonly", label: "Read only", hint: "Read data only" },
  { value: "", testId: "preset-none", label: "No access", hint: "Take the access away" },
];

export function ManageGrantsDialog({
  open,
  onOpenChange,
  user,
  databases,
  onGrantsUpdated,
}: ManageGrantsDialogProps) {
  const { t } = useTranslation();
  const { toast } = useToast();

  const [selectedDb, setSelectedDb] = useState<string>("");
  const [access, setAccess] = useState<DbAccess>("");
  const [isSubmitting, setIsSubmitting] = useState(false);

  const accessOn = (u: DatabaseUserItem, dbId: string): DbAccess =>
    ((u.grants.find((g) => g.db_id === dbId)?.access as DbAccess | undefined) ?? "");

  useEffect(() => {
    if (open && user && databases.length > 0) {
      setSelectedDb(databases[0].id);
      setAccess(accessOn(user, databases[0].id));
    }
  }, [open, user, databases]);

  const handleDbChange = (dbId: string) => {
    setSelectedDb(dbId);
    if (user) setAccess(accessOn(user, dbId));
  };

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!user || !selectedDb) return;

    try {
      setIsSubmitting(true);
      await setGrant(selectedDb, user.id, access);
      const dbName = databases.find((d) => d.id === selectedDb)?.name ?? "";

      toast({
        title: t("common.success"),
        description: t("databases.toast.grantsUpdated", "Privileges updated for {{user}} on {{db}}", {
          user: user.username,
          db: dbName,
        }),
        variant: "default",
      });

      onGrantsUpdated(user);
      onOpenChange(false);
    } catch (err: any) {
      toast({
        title: t("common.error"),
        description: err?.message || t("databases.errors.grantsUpdateFailed", "Failed to update grants"),
        variant: "destructive",
      });
    } finally {
      setIsSubmitting(false);
    }
  };

  if (!user) return null;

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-lg" onClose={() => onOpenChange(false)}>
        <DialogHeader>
          <div className="flex items-center gap-3">
            <div className="rounded-full bg-primary/10 p-2.5 text-primary">
              <Shield className="h-5 w-5" />
            </div>
            <div>
              <DialogTitle>{t("databases.grantsDialog.title", "Manage Database Privileges")}</DialogTitle>
              <DialogDescription className="mt-0.5 text-xs font-mono">
                {user.username}
              </DialogDescription>
            </div>
          </div>
        </DialogHeader>

        <form onSubmit={handleSubmit} className="mt-4 space-y-4">
          {/* Target Database Select */}
          <div className="space-y-1.5">
            <label className="text-xs font-medium text-foreground">
              {t("databases.fields.targetDatabase", "Target Database")}
            </label>
            <select
              value={selectedDb}
              onChange={(e) => handleDbChange(e.target.value)}
              className="w-full h-9 rounded-md border border-input bg-background px-2.5 text-xs font-mono"
              data-testid="select-grant-database"
            >
              {databases.map((db) => (
                <option key={db.id} value={db.id}>
                  {db.name}
                </option>
              ))}
            </select>
          </div>

          {/* The access level */}
          <div className="space-y-2" role="radiogroup">
            {ACCESS_LEVELS.map((lvl) => (
              <label
                key={lvl.testId}
                className={`flex items-center gap-3 p-2.5 rounded border text-xs cursor-pointer transition-colors ${
                  access === lvl.value
                    ? "border-primary bg-primary/10 text-primary font-medium"
                    : "border-border hover:bg-muted text-muted-foreground hover:text-foreground"
                }`}
              >
                <input
                  type="radio"
                  name="db-access"
                  checked={access === lvl.value}
                  onChange={() => setAccess(lvl.value)}
                  className="h-3.5 w-3.5"
                  data-testid={lvl.testId}
                />
                <span className="flex flex-col">
                  <span>{lvl.label}</span>
                  <span className="text-[11px] font-normal text-muted-foreground">{lvl.hint}</span>
                </span>
              </label>
            ))}
          </div>

          <DialogFooter className="mt-6 border-t border-border pt-3">
            <Button
              type="button"
              variant="outline"
              size="sm"
              onClick={() => onOpenChange(false)}
              disabled={isSubmitting}
            >
              {t("common.cancel")}
            </Button>
            <Button
              type="submit"
              size="sm"
              disabled={isSubmitting}
              data-testid="btn-save-grants"
            >
              {isSubmitting ? t("common.saving") : t("databases.actions.savePrivileges", "Save Privileges")}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
