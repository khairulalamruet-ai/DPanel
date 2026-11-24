import React, { useState, useEffect } from "react";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
  Button,
  Badge,
} from "@builtpanel/ui";
import { listVersions, revertTo } from "./api";
import { History, RotateCcw, User, Calendar, FileCode, Check } from "lucide-react";
import type { VhostTemplate, VhostTemplateVersion } from "./types";

interface VhostVersionHistoryDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  template: VhostTemplate | null;
  onReverted: (tpl: VhostTemplate) => void;
}

export function VhostVersionHistoryDialog({
  open,
  onOpenChange,
  template,
  onReverted,
}: VhostVersionHistoryDialogProps) {
  const [versions, setVersions] = useState<VhostTemplateVersion[]>([]);
  const [selectedVersion, setSelectedVersion] = useState<VhostTemplateVersion | null>(null);
  const [isLoading, setIsLoading] = useState(false);
  const [isReverting, setIsReverting] = useState(false);

  useEffect(() => {
    if (open && template) {
      setIsLoading(true);
      listVersions(template.id)
        .then((items) => {
          setVersions(items);
          if (items.length > 0) {
            setSelectedVersion(items[0]);
          }
        })
        .catch(() => {})
        .finally(() => setIsLoading(false));
    }
  }, [open, template]);

  const handleRevert = async (ver: VhostTemplateVersion) => {
    if (!template) return;
    try {
      setIsReverting(true);
      const reverted = await revertTo(template, ver);
      onReverted(reverted);
      onOpenChange(false);
    } catch {
      // handled
    } finally {
      setIsReverting(false);
    }
  };

  if (!template) return null;

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-3xl" data-testid="dialog-vhost-history">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <History className="h-5 w-5 text-primary" />
            Version History: {template.name}
          </DialogTitle>
          <DialogDescription>
            Inspect previous template revisions, view change logs, or restore a previous version.
          </DialogDescription>
        </DialogHeader>

        <div className="grid grid-cols-1 md:grid-cols-12 gap-4 my-2 max-h-[500px]">
          {/* Versions list */}
          <div className="md:col-span-5 space-y-2 overflow-y-auto max-h-[460px] pr-1">
            {isLoading ? (
              <p className="text-xs text-muted-foreground p-4 text-center">Loading revisions...</p>
            ) : versions.length === 0 ? (
              <p className="text-xs text-muted-foreground p-4 text-center">No version history found</p>
            ) : (
              versions.map((ver) => (
                <div
                  key={ver.id}
                  onClick={() => setSelectedVersion(ver)}
                  className={`p-3 rounded-lg border text-xs cursor-pointer transition-colors ${
                    selectedVersion?.id === ver.id
                      ? "border-primary bg-primary/5"
                      : "border-border hover:bg-muted/50"
                  }`}
                  data-testid={`version-item-${ver.version}`}
                >
                  <div className="flex items-center justify-between gap-2">
                    <Badge variant={selectedVersion?.id === ver.id ? "default" : "outline"} className="font-mono text-[10px]">
                      v{ver.version}
                    </Badge>
                    <span className="text-[10px] text-muted-foreground flex items-center gap-1">
                      <Calendar className="h-3 w-3" />
                      {new Date(ver.created_at).toLocaleDateString()}
                    </span>
                  </div>

                  <p className="mt-1.5 font-medium text-foreground line-clamp-2">
                    {ver.changelog}
                  </p>

                  <div className="mt-2 flex items-center justify-between text-[10px] text-muted-foreground">
                    <span className="flex items-center gap-1">
                      <User className="h-3 w-3" />
                      {ver.created_by}
                    </span>
                    {ver.version === template.version && (
                      <span className="text-primary font-semibold flex items-center gap-0.5">
                        <Check className="h-3 w-3" />
                        Current
                      </span>
                    )}
                  </div>
                </div>
              ))
            )}
          </div>

          {/* Selected version preview */}
          <div className="md:col-span-7 flex flex-col border rounded-lg overflow-hidden bg-zinc-950">
            <div className="flex items-center justify-between p-2.5 bg-zinc-900 border-b border-zinc-800 text-xs text-zinc-300">
              <span className="font-mono flex items-center gap-1.5">
                <FileCode className="h-3.5 w-3.5 text-primary" />
                Snapshot: v{selectedVersion?.version}
              </span>

              {selectedVersion && selectedVersion.version !== template.version && (
                <Button
                  size="sm"
                  variant="outline"
                  onClick={() => handleRevert(selectedVersion)}
                  disabled={isReverting}
                  className="h-6 text-[11px] gap-1 px-2 border-zinc-700 bg-zinc-800 hover:bg-zinc-700 text-zinc-200"
                  data-testid="btn-revert-version"
                >
                  <RotateCcw className="h-3 w-3 text-primary" />
                  {isReverting ? "Restoring..." : "Restore This Version"}
                </Button>
              )}
            </div>

            <pre className="p-3 font-mono text-[11px] leading-5 text-zinc-200 overflow-auto flex-1 max-h-[400px]">
              {selectedVersion?.content}
            </pre>
          </div>
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            Close
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
