import React, { useState, useEffect } from "react";
import {
  Dialog,
  DialogContent,
  Button,
  Badge,
  useToast,
} from "@builtpanel/ui";
import { readFile, writeFile } from "./api";
import { FileCode, Save } from "lucide-react";
import { FileItem, ReadFileResponse, WriteFilePayload } from "./types";
import { useTranslation } from "@builtpanel/i18n";

interface FileEditorModalProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  file: FileItem | null;
  onSaved?: () => void;
}

export function FileEditorModal({
  open,
  onOpenChange,
  file,
  onSaved,
}: FileEditorModalProps) {
  const { t } = useTranslation();
  const { toast } = useToast();

  const [content, setContent] = useState<string>("");
  const [initialContent, setInitialContent] = useState<string>("");
  const [isLoading, setIsLoading] = useState(false);
  const [mtime, setMtime] = useState<number | undefined>(undefined);
  const [isSaving, setIsSaving] = useState(false);

  useEffect(() => {
    if (open && file && file.type === "file") {
      const loadContent = async () => {
        try {
          setIsLoading(true);
          const res = await readFile(file.path);
          setMtime(res.mtime);
          setContent(res.content);
          setInitialContent(res.content);
        } catch (err: any) {
          toast({
            title: t("common.error"),
            description: err?.message || t("files.editor.loadFailed", "Failed to load file content"),
            variant: "destructive",
          });
        } finally {
          setIsLoading(false);
        }
      };
      loadContent();
    }
  }, [open, file, toast, t]);

  const isDirty = content !== initialContent;
  const lineCount = content.split("\n").length;
  const charCount = content.length;

  const handleSave = async () => {
    if (!file) return;
    try {
      setIsSaving(true);
      await writeFile(file.path, content, mtime);
      // The next save is checked against what was just written.
      setMtime((await readFile(file.path)).mtime);

      setInitialContent(content);
      toast({
        title: t("common.success"),
        description: t("files.editor.saved", "File {{name}} saved successfully", { name: file.name }),
        variant: "default",
      });

      onSaved?.();
    } catch (err: any) {
      toast({
        title: t("common.error"),
        description: err?.message || t("files.editor.saveFailed", "Failed to save file"),
        variant: "destructive",
      });
    } finally {
      setIsSaving(false);
    }
  };

  if (!file) return null;

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-4xl h-[85vh] flex flex-col p-0 gap-0 overflow-hidden" onClose={() => onOpenChange(false)} data-testid="file-editor-modal">
        {/* Editor Header */}
        <div className="flex items-center justify-between border-b border-border bg-muted/40 px-5 py-3">
          <div className="flex items-center gap-2.5">
            <div className="rounded-full bg-primary/10 p-1.5 text-primary">
              <FileCode className="h-4 w-4" />
            </div>
            <div>
              <div className="flex items-center gap-2">
                <span className="font-semibold text-sm text-foreground" data-testid="file-editor-filename">{file.name}</span>
                {isDirty && (
                  <Badge variant="secondary" className="text-[10px] py-0">
                    {t("files.editor.unsaved", "Unsaved Changes")}
                  </Badge>
                )}
              </div>
              <span className="text-[11px] font-mono text-muted-foreground">{file.path}</span>
            </div>
          </div>

          <div className="flex items-center gap-2">
            <Button
              variant="outline"
              size="sm"
              onClick={() => onOpenChange(false)}
              className="text-xs"
              data-testid="btn-close-file-editor"
            >
              {t("common.close")}
            </Button>
            <Button
              size="sm"
              onClick={handleSave}
              disabled={!isDirty || isSaving || isLoading}
              className="text-xs"
              data-testid="btn-save-file-editor"
            >
              <Save className="h-3.5 w-3.5 mr-1.5" />
              {isSaving ? t("common.saving") : t("common.save")}
            </Button>
          </div>
        </div>

        {/* Editor Body */}
        {isLoading ? (
          <div className="flex-1 flex items-center justify-center">
            <p className="text-sm text-muted-foreground">{t("common.loading")}</p>
          </div>
        ) : (
          <div className="flex-1 flex overflow-hidden">
            {/* Line numbers gutter */}
            <div
              className="select-none bg-muted/30 px-3 py-3 text-right font-mono text-xs text-muted-foreground/60 border-r border-border overflow-hidden"
              style={{ width: "3.5rem" }}
              aria-hidden="true"
            >
              {Array.from({ length: Math.min(lineCount, 1000) }).map((_, i) => (
                <div key={i + 1} className="leading-6">
                  {i + 1}
                </div>
              ))}
            </div>

            {/* Code Textarea */}
            <textarea
              value={content}
              onChange={(e) => setContent(e.target.value)}
              spellCheck={false}
              className="flex-1 resize-none bg-background p-3 font-mono text-xs leading-6 text-foreground focus:outline-none overflow-auto"
              data-testid="textarea-file-editor"
            />
          </div>
        )}

        {/* Status bar */}
        <div className="flex items-center justify-between border-t border-border bg-muted/40 px-5 py-2 text-[11px] font-mono text-muted-foreground">
          <span>{file.extension ? `${file.extension.toUpperCase()} Document` : "Plain Text"}</span>
          <div className="flex items-center gap-4">
            <span>{lineCount} lines</span>
            <span>{charCount} characters</span>
            <span>{file.permissions}</span>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}
