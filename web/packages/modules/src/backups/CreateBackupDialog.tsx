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
import { Archive, Database, Folder, Globe } from "lucide-react";
import { useTranslation } from "@builtpanel/i18n";
import { createBackup } from "./api";

interface CreateBackupDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onCreated: () => void;
}

/** A backup is always the whole account: its files, its databases and its DNS records. */
export function CreateBackupDialog({ open, onOpenChange, onCreated }: CreateBackupDialogProps) {
  const { t } = useTranslation();
  const { toast } = useToast();
  const [isSubmitting, setIsSubmitting] = useState(false);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();

    try {
      setIsSubmitting(true);
      await createBackup();

      toast({
        title: t("common.success"),
        description: t("backups.backupStarted"),
      });

      onOpenChange(false);
      onCreated();
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

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-[480px]">
        <DialogHeader>
          <div className="flex items-center gap-2">
            <div className="p-2 rounded-lg bg-primary/10 text-primary">
              <Archive className="w-5 h-5" />
            </div>
            <DialogTitle data-testid="create-backup-dialog-title">{t("backups.takeBackupNow")}</DialogTitle>
          </div>
        </DialogHeader>

        <form onSubmit={handleSubmit} className="space-y-4 py-2">
          <p className="text-sm text-muted-foreground">
            {t("backups.wholeAccount", "The backup holds everything of this account. It is made in the background; it shows as ready in the list when it is done.")}
          </p>
          <ul className="space-y-2 p-3 bg-muted/30 border rounded-lg text-xs font-medium">
            <li className="flex items-center gap-2.5">
              <Folder className="w-3.5 h-3.5 text-amber-500" />
              <span>{t("backups.homeAndWebFiles")}</span>
            </li>
            <li className="flex items-center gap-2.5">
              <Database className="w-3.5 h-3.5 text-blue-500" />
              <span>{t("backups.mariaDbDatabases")}</span>
            </li>
            <li className="flex items-center gap-2.5">
              <Globe className="w-3.5 h-3.5 text-emerald-500" />
              <span>{t("backups.dnsRecords", "DNS records")}</span>
            </li>
          </ul>

          <DialogFooter className="pt-2">
            <Button type="button" variant="outline" onClick={() => onOpenChange(false)} disabled={isSubmitting}>
              {t("common.cancel")}
            </Button>
            <Button type="submit" disabled={isSubmitting} data-testid="btn-submit-create-backup">
              {isSubmitting ? t("common.submitting") : t("backups.startBackup")}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
