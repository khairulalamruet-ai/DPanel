import React, { useState, useEffect } from "react";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogFooter,
  Button,
  useToast,
} from "@builtpanel/ui";
import { listTrash, restoreFromTrash, emptyTrash } from "./api";
import { Trash2, RotateCcw, File, Folder } from "lucide-react";
import { FileItem, TrashListResponse, RestoreTrashPayload } from "./types";
import { useTranslation } from "@builtpanel/i18n";

interface TrashViewModalProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onRestored: () => void;
}

export function TrashViewModal({
  open,
  onOpenChange,
  onRestored,
}: TrashViewModalProps) {
  const { t } = useTranslation();
  const { toast } = useToast();
  const [items, setItems] = useState<FileItem[]>([]);
  const [isLoading, setIsLoading] = useState(false);
  const [isActionPending, setIsActionPending] = useState(false);

  const fetchTrash = async () => {
    try {
      setIsLoading(true);
      const res = await listTrash();
      setItems(res.items || []);
    } catch (err: any) {
      toast({
        title: t("common.error"),
        description: err.message || t("common.unknownError"),
        variant: "destructive",
      });
    } finally {
      setIsLoading(false);
    }
  };

  useEffect(() => {
    if (open) {
      fetchTrash();
    }
  }, [open]);

  const handleRestore = async (item: FileItem) => {
    try {
      setIsActionPending(true);
      await restoreFromTrash(item.id);

      toast({
        title: t("common.success"),
        description: t("files.itemRestored"),
      });
      await fetchTrash();
      onRestored();
    } catch (err: any) {
      toast({
        title: t("common.error"),
        description: err.message || t("common.unknownError"),
        variant: "destructive",
      });
    } finally {
      setIsActionPending(false);
    }
  };

  const handleEmptyTrash = async () => {
    try {
      setIsActionPending(true);
      await emptyTrash();

      toast({
        title: t("common.success"),
        description: t("files.trashEmptied"),
      });
      setItems([]);
      onRestored();
    } catch (err: any) {
      toast({
        title: t("common.error"),
        description: err.message || t("common.unknownError"),
        variant: "destructive",
      });
    } finally {
      setIsActionPending(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-[650px] max-h-[85vh] flex flex-col">
        <DialogHeader>
          <div className="flex items-center justify-between pr-6">
            <div className="flex items-center gap-2">
              <div className="p-2 rounded-lg bg-destructive/10 text-destructive">
                <Trash2 className="w-5 h-5" />
              </div>
              <div>
                <DialogTitle data-testid="trash-modal-title">
                  {t("files.trash")}
                </DialogTitle>
                <p className="text-xs text-muted-foreground mt-0.5">
                  {t("files.trashDescription")}
                </p>
              </div>
            </div>
            {items.length > 0 && (
              <Button
                variant="destructive"
                size="sm"
                onClick={handleEmptyTrash}
                disabled={isActionPending}
                data-testid="empty-trash-button"
              >
                {t("files.emptyTrash")}
              </Button>
            )}
          </div>
        </DialogHeader>

        <div className="flex-1 overflow-y-auto min-h-[250px] max-h-[400px] border rounded-lg p-2 my-3">
          {isLoading ? (
            <div className="p-8 text-center text-sm text-muted-foreground">
              {t("common.loading")}
            </div>
          ) : items.length === 0 ? (
            <div className="p-8 text-center text-muted-foreground flex flex-col items-center justify-center">
              <Trash2 className="w-10 h-10 stroke-1 mb-2 opacity-50" />
              <p className="text-sm">{t("files.trashEmpty")}</p>
            </div>
          ) : (
            <div className="divide-y divide-border">
              {items.map((item) => (
                <div
                  key={item.id}
                  className="flex items-center justify-between p-2.5 hover:bg-muted/50 rounded transition-colors"
                  data-testid={`trash-item-${item.id}`}
                >
                  <div className="flex items-center gap-3 min-w-0">
                    {item.type === "directory" ? (
                      <Folder className="w-4 h-4 text-primary shrink-0" />
                    ) : (
                      <File className="w-4 h-4 text-muted-foreground shrink-0" />
                    )}
                    <div className="min-w-0">
                      <p className="text-sm font-medium truncate">{item.name}</p>
                      <p className="text-xs text-muted-foreground truncate">
                        {item.original_path || item.path} • {item.size_formatted}
                      </p>
                    </div>
                  </div>

                  <Button
                    size="sm"
                    variant="outline"
                    className="gap-1.5 shrink-0"
                    onClick={() => handleRestore(item)}
                    disabled={isActionPending}
                    data-testid={`restore-button-${item.id}`}
                  >
                    <RotateCcw className="w-3.5 h-3.5" />
                    <span>{t("files.restore")}</span>
                  </Button>
                </div>
              ))}
            </div>
          )}
        </div>

        <DialogFooter>
          <Button
            type="button"
            variant="outline"
            onClick={() => onOpenChange(false)}
            data-testid="close-trash-modal"
          >
            {t("common.close")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
