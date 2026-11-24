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
import { archive } from "./api";
import { Archive } from "lucide-react";
import { FileItem, ArchivePayload } from "./types";
import { useTranslation } from "@builtpanel/i18n";

interface ArchiveDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  items: FileItem[];
  currentPath: string;
  onArchived: () => void;
}

export function ArchiveDialog({
  open,
  onOpenChange,
  items,
  currentPath: _currentPath,
  onArchived,
}: ArchiveDialogProps) {
  const { t } = useTranslation();
  const { toast } = useToast();
  const [archiveName, setArchiveName] = useState("archive.zip");
  const [format, setFormat] = useState<"zip" | "tar.gz">("zip");
  const [isSubmitting, setIsSubmitting] = useState(false);

  useEffect(() => {
    if (open) {
      if (items.length === 1) {
        const base = items[0].name.replace(/\.[^/.]+$/, "");
        setArchiveName(`${base}.${format}`);
      } else {
        setArchiveName(`archive.${format}`);
      }
    }
  }, [open, items, format]);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (items.length === 0) return;

    try {
      setIsSubmitting(true);
      await archive(
        items.map((i) => i.path),
        _currentPath,
        archiveName.trim(),
        format,
      );

      toast({
        title: t("common.success"),
        description: t("files.archiveCreated"),
      });
      onOpenChange(false);
      onArchived();
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
      <DialogContent className="sm:max-w-[425px]">
        <DialogHeader>
          <div className="flex items-center gap-2">
            <div className="p-2 rounded-lg bg-primary/10 text-primary">
              <Archive className="w-5 h-5" />
            </div>
            <DialogTitle data-testid="archive-dialog-title">
              {t("files.compress")}
            </DialogTitle>
          </div>
        </DialogHeader>

        <form onSubmit={handleSubmit} className="space-y-4 py-2">
          <div className="space-y-1.5">
            <label className="text-sm font-medium">
              {t("files.selectedItems")} ({items.length})
            </label>
            <div className="max-h-24 overflow-y-auto p-2 bg-muted rounded text-xs font-mono space-y-1">
              {items.map((it) => (
                <div key={it.id} className="truncate">
                  {it.name}
                </div>
              ))}
            </div>
          </div>

          <div className="space-y-1.5">
            <label className="text-sm font-medium">
              {t("files.archiveFormat")}
            </label>
            <div className="flex gap-4">
              <label className="flex items-center gap-2 text-sm cursor-pointer">
                <input
                  type="radio"
                  name="format"
                  value="zip"
                  checked={format === "zip"}
                  onChange={() => {
                    setFormat("zip");
                    setArchiveName((prev) => prev.replace(/\.(tar\.gz|zip)$/, ".zip"));
                  }}
                />
                <span>.zip</span>
              </label>
              <label className="flex items-center gap-2 text-sm cursor-pointer">
                <input
                  type="radio"
                  name="format"
                  value="tar.gz"
                  checked={format === "tar.gz"}
                  onChange={() => {
                    setFormat("tar.gz");
                    setArchiveName((prev) => prev.replace(/\.(tar\.gz|zip)$/, ".tar.gz"));
                  }}
                />
                <span>.tar.gz</span>
              </label>
            </div>
          </div>

          <div className="space-y-1.5">
            <label className="text-sm font-medium">
              {t("files.archiveName")}
            </label>
            <Input
              value={archiveName}
              onChange={(e) => setArchiveName(e.target.value)}
              placeholder="archive.zip"
              data-testid="archive-name-input"
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
              disabled={!archiveName.trim() || isSubmitting}
              data-testid="archive-submit-button"
            >
              {isSubmitting ? t("common.submitting") : t("files.compress")}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
