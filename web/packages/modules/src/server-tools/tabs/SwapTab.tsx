import React, { useState } from "react";
import {
  Card,
  CardHeader,
  CardTitle,
  CardDescription,
  CardContent,
  CardFooter,
  Button,
  Input,
  Badge,
} from "@builtpanel/ui";
import { HardDrive, Trash2, Zap, ShieldCheck, Loader2 } from "lucide-react";
import { useTranslation } from "@builtpanel/i18n";
import { ServerSwapInfo, ServerMemoryInfo } from "../types";
import { SudoConfirmModal } from "../SudoConfirmModal";

interface SwapTabProps {
  swap: ServerSwapInfo;
  memory: ServerMemoryInfo;
  canManage: boolean;
  onUpdateSwap: (totalMb: number, swappiness: number, sudoPassword: string) => Promise<void>;
  onDropCaches: (sudoPassword: string) => Promise<void>;
}

export function SwapTab({
  swap,
  memory,
  canManage,
  onUpdateSwap,
  onDropCaches,
}: SwapTabProps) {
  const { t } = useTranslation();

  const [totalMb, setTotalMb] = useState(swap.total_mb);
  const [swappiness, setSwappiness] = useState(swap.swappiness);

  const [pendingAction, setPendingAction] = useState<
    | { type: "swap"; totalMb: number }
    | { type: "drop_caches" }
    | null
  >(null);

  React.useEffect(() => {
    setTotalMb(swap.total_mb);
    setSwappiness(swap.swappiness);
  }, [swap.total_mb, swap.swappiness]);

  const usedPercent = swap.total_mb > 0 ? Math.round((swap.used_mb / swap.total_mb) * 100) : 0;
  const hasChanged = totalMb !== swap.total_mb;

  const handleSudoConfirm = async (password: string) => {
    if (!pendingAction) return;

    if (pendingAction.type === "swap") {
      await onUpdateSwap(pendingAction.totalMb, swappiness, password);
    } else if (pendingAction.type === "drop_caches") {
      await onDropCaches(password);
    }
    setPendingAction(null);
  };

  return (
    <div className="space-y-6">
      {/* 1. Swap Configuration Card */}
      <Card data-testid="card-swap-config">
        <CardHeader>
          <div className="flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
            <div>
              <CardTitle className="text-base font-semibold flex items-center gap-2">
                <HardDrive className="h-4 w-4 text-primary" />
                <span data-testid="title-swap-management">{t("serverTools.swapTitle", "Virtual Swap File Management")}</span>
              </CardTitle>
              <CardDescription className="text-xs text-muted-foreground">
                {t(
                  "serverTools.swapDescription",
                  "Monitor and adjust Linux virtual swap file size and swappiness kernel parameter",
                )}
              </CardDescription>
            </div>
            <Badge variant="outline" className="font-mono text-xs" data-testid="badge-swap-usage">
              {swap.used_mb} / {swap.total_mb} MB ({usedPercent}%)
            </Badge>
          </div>
        </CardHeader>

        <CardContent className="space-y-4">
          {/* Progress gauge */}
          <div className="rounded-lg border bg-muted/20 p-4 space-y-2">
            <div className="flex justify-between text-xs text-muted-foreground">
              <span>
                {t("serverTools.swapUsed", "Used Swap")}: <strong className="font-mono text-foreground">{swap.used_mb} MB</strong>
              </span>
              <span>
                {t("serverTools.swapFree", "Free Swap")}: <strong className="font-mono text-foreground">{swap.free_mb} MB</strong>
              </span>
              <span>
                {t("common.total", "Total")}: <strong className="font-mono text-foreground">{swap.total_mb} MB</strong>
              </span>
            </div>
            <div className="h-2.5 w-full overflow-hidden rounded-full bg-muted">
              <div
                className="h-full bg-primary transition-all duration-300"
                style={{ width: `${Math.min(100, usedPercent)}%` }}
              />
            </div>
          </div>

          {/* Presets & Drop Swap */}
          <div className="space-y-2">
            <label className="text-xs font-medium text-foreground">
              {t("serverTools.swapSizeLabel", "Configure Swap Size (Presets or Custom MB)")}
            </label>
            <div className="flex flex-wrap gap-2">
              <Button
                type="button"
                variant={totalMb === 0 ? "destructive" : "outline"}
                size="sm"
                className="text-xs gap-1"
                data-testid="btn-swap-preset-0"
                onClick={() => setTotalMb(0)}
              >
                <Trash2 className="h-3.5 w-3.5" />
                <span>{t("system.swap.dropSwapBtn", "0 MB (Drop Swap)")}</span>
              </Button>

              {[1024, 2048, 4096, 8192, 16384].map((sz) => (
                <Button
                  key={sz}
                  type="button"
                  variant={totalMb === sz ? "default" : "outline"}
                  size="sm"
                  className="font-mono text-xs"
                  data-testid={`btn-swap-preset-${sz}`}
                  onClick={() => setTotalMb(sz)}
                >
                  {sz >= 1024 ? `${sz / 1024} GB` : `${sz} MB`}
                </Button>
              ))}
            </div>
          </div>

          {/* Custom Size Input */}
          <div className="max-w-xs space-y-1.5 pt-1">
            <label className="text-xs font-medium text-foreground">
              {t("serverTools.customMbLabel", "Custom Size (MB):")}
            </label>
            <Input
              data-testid="input-custom-swap-mb"
              type="number"
              min={0}
              max={32768}
              value={totalMb}
              onChange={(e) => setTotalMb(Math.max(0, parseInt(e.target.value, 10) || 0))}
              className="font-mono text-xs"
            />
            <span className="text-[11px] text-muted-foreground">
              {t("system.swap.rangeHelp", "Allowed range: 0 to 32,768 MB (32 GB). Setting to 0 deletes swapfile.")}
            </span>
          </div>
        </CardContent>

        <CardFooter className="flex items-center justify-between border-t px-6 py-3">
          <span className="text-[11px] text-muted-foreground">
            {hasChanged
              ? t("system.swap.pendingNotice", `New size: ${totalMb} MB pending save.`)
              : t("system.swap.currentActive", "Current swap configuration is active.")}
          </span>

          <Button
            size="sm"
            disabled={!hasChanged || !canManage}
            data-testid="btn-save-swap"
            onClick={() => setPendingAction({ type: "swap", totalMb })}
            className="gap-1.5 text-xs"
          >
            <HardDrive className="h-3.5 w-3.5" />
            <span>{totalMb === 0 ? t("system.swap.applyDropSwap", "Drop Swap File") : t("serverTools.applySwapConfig", "Apply Swap Config")}</span>
          </Button>
        </CardFooter>
      </Card>

      {/* 2. Drop Page Cache Card */}
      <Card data-testid="card-drop-caches">
        <CardHeader>
          <div className="flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
            <div>
              <CardTitle className="text-base font-semibold flex items-center gap-2">
                <Zap className="h-4 w-4 text-amber-500" />
                <span data-testid="title-drop-caches">{t("serverTools.dropCachesTitle", "Drop Page Cache & Kernel Memory")}</span>
              </CardTitle>
              <CardDescription className="text-xs text-muted-foreground">
                {t(
                  "serverTools.dropCachesDescription",
                  "Safely flush Linux kernel pagecache, dentries, and inodes via /proc/sys/vm/drop_caches without affecting active dirty memory",
                )}
              </CardDescription>
            </div>
            <Badge variant="outline" className="font-mono text-xs">
              {memory.available_mb} / {memory.total_mb} MB available
            </Badge>
          </div>
        </CardHeader>

        <CardContent>
          <div className="rounded-lg border bg-muted/20 p-4 flex flex-col sm:flex-row sm:items-center justify-between gap-4">
            <div className="space-y-1">
              <h5 className="font-semibold text-xs text-foreground">
                {t("system.swap.dropCacheExplTitle", "Free Clean Cached Pages")}
              </h5>
              <p className="text-xs text-muted-foreground max-w-lg">
                {t(
                  "system.swap.dropCacheExplBody",
                  "Linux caches disk files in memory automatically. Dropping clean caches is safe and releases unused pages back to the free pool immediately.",
                )}
              </p>
            </div>

            <Button
              variant="outline"
              size="sm"
              disabled={!canManage}
              data-testid="btn-drop-caches"
              onClick={() => setPendingAction({ type: "drop_caches" })}
              className="gap-1.5 text-xs shrink-0"
            >
              <Zap className="h-3.5 w-3.5 text-amber-500" />
              <span>{t("serverTools.dropCachesButton", "Drop Caches Now")}</span>
            </Button>
          </div>
        </CardContent>
      </Card>

      {/* Sudo Modal */}
      <SudoConfirmModal
        open={Boolean(pendingAction)}
        onOpenChange={(open) => !open && setPendingAction(null)}
        title={
          pendingAction?.type === "drop_caches"
            ? t("serverTools.dropCachesTitle", "Drop Memory Caches")
            : t("serverTools.swapTitle", "Update Swap Memory")
        }
        description={
          pendingAction?.type === "drop_caches"
            ? t("system.swap.sudoDropCacheDesc", "Flushing kernel caches executes privileged operations. Please enter your sudo password.")
            : t("system.swap.sudoSwapDesc", "Modifying Linux swapfile size requires privileged kernel allocation. Please enter sudo password.")
        }
        confirmLabel={t("common.confirm", "Confirm & Apply")}
        onConfirm={handleSudoConfirm}
      />
    </div>
  );
}
