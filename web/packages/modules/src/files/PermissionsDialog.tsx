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
import { chmod } from "./api";
import { Shield } from "lucide-react";
import { FileItem, ChmodPayload } from "./types";
import { useTranslation } from "@builtpanel/i18n";

interface PermissionsDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  file: FileItem | null;
  onPermissionsUpdated?: (updatedFile: FileItem) => void;
}

export function PermissionsDialog({
  open,
  onOpenChange,
  file,
  onPermissionsUpdated,
}: PermissionsDialogProps) {
  const { t } = useTranslation();
  const { toast } = useToast();

  const [ownerRead, setOwnerRead] = useState(true);
  const [ownerWrite, setOwnerWrite] = useState(true);
  const [ownerExec, setOwnerExec] = useState(false);

  const [groupRead, setGroupRead] = useState(true);
  const [groupWrite, setGroupWrite] = useState(false);
  const [groupExec, setGroupExec] = useState(false);

  const [publicRead, setPublicRead] = useState(true);
  const [publicWrite, setPublicWrite] = useState(false);
  const [publicExec, setPublicExec] = useState(false);

  const [isSubmitting, setIsSubmitting] = useState(false);

  // Initialize from file.permissions (e.g. "0644" or "0755")
  useEffect(() => {
    if (open && file) {
      const mode = file.permissions.replace(/^0/, "");
      const o = parseInt(mode[0] || "6", 10);
      const g = parseInt(mode[1] || "4", 10);
      const p = parseInt(mode[2] || "4", 10);

      setOwnerRead(Boolean(o & 4));
      setOwnerWrite(Boolean(o & 2));
      setOwnerExec(Boolean(o & 1));

      setGroupRead(Boolean(g & 4));
      setGroupWrite(Boolean(g & 2));
      setGroupExec(Boolean(g & 1));

      setPublicRead(Boolean(p & 4));
      setPublicWrite(Boolean(p & 2));
      setPublicExec(Boolean(p & 1));
    }
  }, [open, file]);

  const ownerVal = (ownerRead ? 4 : 0) + (ownerWrite ? 2 : 0) + (ownerExec ? 1 : 0);
  const groupVal = (groupRead ? 4 : 0) + (groupWrite ? 2 : 0) + (groupExec ? 1 : 0);
  const publicVal = (publicRead ? 4 : 0) + (publicWrite ? 2 : 0) + (publicExec ? 1 : 0);
  const octalMode = `0${ownerVal}${groupVal}${publicVal}`;

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!file) return;

    try {
      setIsSubmitting(true);
      await chmod(file.path, octalMode);
      const res: FileItem = { ...file, permissions: octalMode };

      toast({
        title: t("common.success"),
        description: t("files.permissions.updated", "Permissions for {{name}} changed to {{mode}}", {
          name: file.name,
          mode: octalMode,
        }),
        variant: "default",
      });

      onPermissionsUpdated?.(res);
      onOpenChange(false);
    } catch (err: any) {
      toast({
        title: t("common.error"),
        description: err?.message || t("files.permissions.updateFailed", "Failed to update permissions"),
        variant: "destructive",
      });
    } finally {
      setIsSubmitting(false);
    }
  };

  if (!file) return null;

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-md" onClose={() => onOpenChange(false)} data-testid="permissions-dialog">
        <DialogHeader>
          <div className="flex items-center gap-3">
            <div className="rounded-full bg-primary/10 p-2.5 text-primary">
              <Shield className="h-5 w-5" />
            </div>
            <div>
              <DialogTitle>{t("files.permissions.title", "Change Permissions")}</DialogTitle>
              <DialogDescription className="mt-0.5 text-xs font-mono">
                {file.name} ({octalMode})
              </DialogDescription>
            </div>
          </div>
        </DialogHeader>

        <form onSubmit={handleSubmit} className="mt-4 space-y-4">
          <div className="rounded-lg border border-border bg-card overflow-hidden">
            <table className="w-full text-xs">
              <thead className="bg-muted/40 border-b border-border">
                <tr>
                  <th className="py-2.5 px-3 text-left font-medium text-foreground">
                    {t("files.permissions.scope", "Scope")}
                  </th>
                  <th className="py-2.5 px-3 text-center font-medium text-foreground">
                    {t("files.permissions.read", "Read (4)")}
                  </th>
                  <th className="py-2.5 px-3 text-center font-medium text-foreground">
                    {t("files.permissions.write", "Write (2)")}
                  </th>
                  <th className="py-2.5 px-3 text-center font-medium text-foreground">
                    {t("files.permissions.execute", "Execute (1)")}
                  </th>
                </tr>
              </thead>
              <tbody className="divide-y divide-border">
                {/* User / Owner */}
                <tr>
                  <td className="py-2.5 px-3 font-medium text-foreground">
                    {t("files.permissions.owner", "Owner (User)")}
                  </td>
                  <td className="py-2.5 px-3 text-center">
                    <input
                      type="checkbox"
                      checked={ownerRead}
                      onChange={(e) => setOwnerRead(e.target.checked)}
                      data-testid="chmod-owner-read"
                    />
                  </td>
                  <td className="py-2.5 px-3 text-center">
                    <input
                      type="checkbox"
                      checked={ownerWrite}
                      onChange={(e) => setOwnerWrite(e.target.checked)}
                      data-testid="chmod-owner-write"
                    />
                  </td>
                  <td className="py-2.5 px-3 text-center">
                    <input
                      type="checkbox"
                      checked={ownerExec}
                      onChange={(e) => setOwnerExec(e.target.checked)}
                      data-testid="chmod-owner-exec"
                    />
                  </td>
                </tr>

                {/* Group */}
                <tr>
                  <td className="py-2.5 px-3 font-medium text-foreground">
                    {t("files.permissions.group", "Group")}
                  </td>
                  <td className="py-2.5 px-3 text-center">
                    <input
                      type="checkbox"
                      checked={groupRead}
                      onChange={(e) => setGroupRead(e.target.checked)}
                      data-testid="chmod-group-read"
                    />
                  </td>
                  <td className="py-2.5 px-3 text-center">
                    <input
                      type="checkbox"
                      checked={groupWrite}
                      onChange={(e) => setGroupWrite(e.target.checked)}
                      data-testid="chmod-group-write"
                    />
                  </td>
                  <td className="py-2.5 px-3 text-center">
                    <input
                      type="checkbox"
                      checked={groupExec}
                      onChange={(e) => setGroupExec(e.target.checked)}
                      data-testid="chmod-group-exec"
                    />
                  </td>
                </tr>

                {/* Public / Others */}
                <tr>
                  <td className="py-2.5 px-3 font-medium text-foreground">
                    {t("files.permissions.public", "Public (World)")}
                  </td>
                  <td className="py-2.5 px-3 text-center">
                    <input
                      type="checkbox"
                      checked={publicRead}
                      onChange={(e) => setPublicRead(e.target.checked)}
                      data-testid="chmod-public-read"
                    />
                  </td>
                  <td className="py-2.5 px-3 text-center">
                    <input
                      type="checkbox"
                      checked={publicWrite}
                      onChange={(e) => setPublicWrite(e.target.checked)}
                      data-testid="chmod-public-write"
                    />
                  </td>
                  <td className="py-2.5 px-3 text-center">
                    <input
                      type="checkbox"
                      checked={publicExec}
                      onChange={(e) => setPublicExec(e.target.checked)}
                      data-testid="chmod-public-exec"
                    />
                  </td>
                </tr>
              </tbody>
            </table>
          </div>

          <div className="flex items-center justify-between text-xs bg-muted/20 p-2.5 rounded border border-border">
            <span className="text-muted-foreground">{t("files.permissions.numericNotation", "Octal Mode")}:</span>
            <span className="font-mono font-bold text-foreground text-sm" data-testid="chmod-octal-value">
              {octalMode}
            </span>
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
              data-testid="btn-save-chmod"
            >
              {isSubmitting ? t("common.saving") : t("common.save")}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
