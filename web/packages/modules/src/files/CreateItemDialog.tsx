import React, { useState } from "react";
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
import { createItem } from "./api";
import { FilePlus, FolderPlus } from "lucide-react";
import { CreateItemPayload } from "./types";
import { useTranslation } from "@builtpanel/i18n";

interface CreateItemDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  currentPath: string;
  type: "file" | "directory";
  onCreated: () => void;
}

export function CreateItemDialog({
  open,
  onOpenChange,
  currentPath,
  type,
  onCreated,
}: CreateItemDialogProps) {
  const { t } = useTranslation();
  const { toast } = useToast();
  const [name, setName] = useState("");
  const [isSubmitting, setIsSubmitting] = useState(false);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    const trimmed = name.trim();
    if (!trimmed) return;

    try {
      setIsSubmitting(true);
      await createItem(currentPath, trimmed, type);

      toast({
        title: t("common.success"),
        description: t(
          type === "file" ? "files.fileCreated" : "files.folderCreated"
        ),
      });
      setName("");
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

  const isFile = type === "file";

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-[425px]">
        <DialogHeader>
          <div className="flex items-center gap-2">
            <div className="p-2 rounded-lg bg-primary/10 text-primary">
              {isFile ? (
                <FilePlus className="w-5 h-5" />
              ) : (
                <FolderPlus className="w-5 h-5" />
              )}
            </div>
            <DialogTitle data-testid="create-dialog-title">
              {t(isFile ? "files.newFile" : "files.newFolder")}
            </DialogTitle>
          </div>
        </DialogHeader>

        <form onSubmit={handleSubmit} className="space-y-4 py-2">
          <div className="space-y-1.5">
            <label className="text-sm font-medium text-muted-foreground">
              {t("files.location")}
            </label>
            <div className="p-2 rounded bg-muted font-mono text-xs text-foreground truncate">
              {currentPath}
            </div>
          </div>

          <div className="space-y-1.5">
            <label className="text-sm font-medium">
              {t(isFile ? "files.fileName" : "files.folderName")}
            </label>
            <Input
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder={isFile ? "example.txt" : "my-folder"}
              autoFocus
              data-testid="create-item-name-input"
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
              disabled={!name.trim() || isSubmitting}
              data-testid="create-item-submit-button"
            >
              {isSubmitting ? t("common.submitting") : t("common.create")}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
