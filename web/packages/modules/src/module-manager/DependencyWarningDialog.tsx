import React from "react";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogFooter,
  DialogTitle,
  DialogDescription,
  Button,
  Badge,
} from "@builtpanel/ui";
import { useTranslation } from "@builtpanel/i18n";
import { AlertTriangle, Layers } from "lucide-react";
import { ModuleItem } from "./types";

export interface DependencyWarningDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  targetModule: ModuleItem | null;
  activeDependents: ModuleItem[];
  onConfirm: () => void;
  isConfirming?: boolean;
}

export function DependencyWarningDialog({
  open,
  onOpenChange,
  targetModule,
  activeDependents,
  onConfirm,
  isConfirming = false,
}: DependencyWarningDialogProps) {
  const { t, i18n } = useTranslation();
  const lang = (i18n.language?.startsWith("bn") ? "bn" : "en") as "en" | "bn";

  if (!targetModule) return null;

  const targetTitle = targetModule.title[lang] || targetModule.title.en || targetModule.name;

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent onClose={() => onOpenChange(false)} className="max-w-md">
        <DialogHeader>
          <div className="flex items-center gap-2 text-destructive mb-1">
            <AlertTriangle className="h-5 w-5 shrink-0" />
            <DialogTitle>{t("modules.warning.title")}</DialogTitle>
          </div>
          <DialogDescription>
            {t("modules.warning.desc", { name: targetTitle })}
          </DialogDescription>
        </DialogHeader>

        <div className="my-4 rounded-lg border border-destructive/20 bg-destructive/5 p-4">
          <div className="flex items-center gap-2 text-xs font-semibold text-destructive uppercase tracking-wider mb-2">
            <Layers className="h-4 w-4" />
            <span>{t("modules.warning.dependentsList")}</span>
          </div>
          <ul className="space-y-2">
            {activeDependents.map((dep) => (
              <li
                key={dep.name}
                className="flex items-center justify-between text-sm bg-background/80 rounded px-2.5 py-1.5 border"
              >
                <div className="flex items-center gap-2">
                  <span className="font-medium text-foreground">
                    {dep.title[lang] || dep.title.en || dep.name}
                  </span>
                  <span className="font-mono text-xs text-muted-foreground">({dep.name})</span>
                </div>
                <div className="flex items-center gap-1.5">
                  <Badge variant="outline" className="text-xs">
                    v{dep.version}
                  </Badge>
                  <Badge variant="secondary" className="text-xs capitalize">
                    {dep.category}
                  </Badge>
                </div>
              </li>
            ))}
          </ul>
        </div>

        <DialogFooter>
          <Button
            type="button"
            variant="outline"
            onClick={() => onOpenChange(false)}
            disabled={isConfirming}
          >
            {t("modules.warning.cancelBtn")}
          </Button>
          <Button
            type="button"
            variant="destructive"
            onClick={onConfirm}
            disabled={isConfirming}
          >
            {isConfirming ? t("modules.toggle.disabling") : t("modules.warning.confirmBtn")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
