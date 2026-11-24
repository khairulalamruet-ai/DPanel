import React, { useState, useEffect } from "react";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogFooter,
  Button,
  Input,
  useToast,
} from "@builtpanel/ui";
import { renameItem } from "./api";
import { Edit } from "lucide-react";
import { FileItem, RenamePayload } from "./types";
import { useTranslation } from "@builtpanel/i18n";

interface RenameDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  file: FileItem | null;
  onRenamed: () => void;
}

export function RenameDialog({
  open,
  onOpenChange,
  file,
  onRenamed,
}: RenameDialogProps) {
  const { t } = useTranslation();
  const { toast } = useToast();
  const [newName, setNewName] = useState("");
  const [isSubmitting, setIsSubmitting] = useState(false);

  useEffect(() => {
    if (open && file) {
      setNewName(file.name);
    }
  }, [open, file]);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!file) return;
    const trimmed = newName.trim();
    if (!trimmed || trimmed === file.name) {
      onOpenChange(false);
      return;
    }

    try {
      setIsSubmitting(true);
      await renameItem(file.path, trimmed);

      toast({
        title: t("common.success"),
        description: t("files.renamedSuccess"),
      });
      onOpenChange(false);
      onRenamed();
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
      <DialogContent className="sm:max-w-[400px]">
        <DialogHeader>
          <div className="flex items-center gap-2">
            <div className="p-2 rounded-lg bg-primary/10 text-primary">
              <Edit className="w-5 h-5" />
            </div>
            <DialogTitle data-testid="rename-dialog-title">
              {t("files.rename")}
            </DialogTitle>
          </div>
        </DialogHeader>

        <form onSubmit={handleSubmit} className="space-y-4 py-2">
          <div className="space-y-1.5">
            <label className="text-sm font-medium">
              {t("files.newName")}
            </label>
            <Input
              value={newName}
              onChange={(e) => setNewName(e.target.value)}
              autoFocus
              data-testid="rename-input"
            />
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
              disabled={!newName.trim() || newName.trim() === file?.name || isSubmitting}
              data-testid="rename-submit-button"
            >
              {isSubmitting ? t("common.submitting") : t("common.save")}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
