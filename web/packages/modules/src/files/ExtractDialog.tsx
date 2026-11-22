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
import { extract } from "./api";
import { Archive } from "lucide-react";
import { FileItem, ExtractPayload } from "./types";
import { useTranslation } from "@builtpanel/i18n";

interface ExtractDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  file: FileItem | null;
  currentPath: string;
  onExtracted: () => void;
}

export function ExtractDialog({
  open,
  onOpenChange,
  file,
  currentPath,
  onExtracted,
}: ExtractDialogProps) {
  const { t } = useTranslation();
  const { toast } = useToast();
  const [destinationPath, setDestinationPath] = useState(currentPath);
  const [isSubmitting, setIsSubmitting] = useState(false);

  useEffect(() => {
    if (open) {
      setDestinationPath(currentPath);
    }
  }, [open, currentPath]);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!file) return;

    try {
      setIsSubmitting(true);
      await extract(file.path, destinationPath.trim());

      toast({
        title: t("common.success"),
        description: t("files.extractSuccess"),
      });
      onOpenChange(false);
      onExtracted();
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
            <DialogTitle data-testid="extract-dialog-title">
              {t("files.extractArchive")}
            </DialogTitle>
          </div>
        </DialogHeader>

        <form onSubmit={handleSubmit} className="space-y-4 py-2">
          <div className="space-y-1.5">
            <label className="text-sm font-medium">
              {t("files.file")}
            </label>
            <div className="p-2 bg-muted rounded text-xs font-mono truncate">
              {file?.name}
            </div>
          </div>

          <div className="space-y-1.5">
            <label className="text-sm font-medium">
              {t("files.destinationDirectory")}
            </label>
            <Input
              value={destinationPath}
              onChange={(e) => setDestinationPath(e.target.value)}
              placeholder="/public_html"
              data-testid="extract-destination-input"
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
              disabled={!destinationPath.trim() || isSubmitting}
              data-testid="extract-submit-button"
            >
              {isSubmitting ? t("common.submitting") : t("files.extract")}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
