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
import { HardDrive, Lock, Settings2 } from "lucide-react";
import { useTranslation } from "@builtpanel/i18n";
import { ServerSwapInfo } from "./types";

interface SwapCardProps {
  swap: ServerSwapInfo;
  onUpdateSwap: (totalMb: number, swappiness: number, sudoPassword: string) => Promise<void>;
}

export function SwapCard({ swap, onUpdateSwap }: SwapCardProps) {
  const { t } = useTranslation();
  const [totalMb, setTotalMb] = useState(swap.total_mb);
  const [swappiness, setSwappiness] = useState(swap.swappiness);
  const [sudoPassword, setSudoPassword] = useState("");
  const [isSubmitting, setIsSubmitting] = useState(false);

  React.useEffect(() => {
    setTotalMb(swap.total_mb);
    setSwappiness(swap.swappiness);
  }, [swap.total_mb, swap.swappiness]);

  const usedPercent = swap.total_mb > 0 ? Math.round((swap.used_mb / swap.total_mb) * 100) : 0;
  const hasChanged = totalMb !== swap.total_mb;
  const canSubmit = hasChanged && sudoPassword.trim().length > 0;

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!canSubmit) return;
    try {
      setIsSubmitting(true);
      await onUpdateSwap(totalMb, swappiness, sudoPassword);
      setSudoPassword("");
    } finally {
      setIsSubmitting(false);
    }
  };

  return (
    <Card className="flex flex-col justify-between">
      <CardHeader>
        <div className="flex items-center justify-between">
          <CardTitle className="flex items-center gap-2 text-base font-semibold">
            <HardDrive className="h-4 w-4 text-primary" />
            <span data-testid="title-swap-management">{t("serverTools.swapTitle")}</span>
          </CardTitle>
          <Badge variant="outline" className="font-mono text-xs" data-testid="badge-swap-usage">
            {swap.used_mb} / {swap.total_mb} MB ({usedPercent}%)
          </Badge>
        </div>
        <CardDescription className="text-xs text-muted-foreground">
          {t("serverTools.swapDescription")}
        </CardDescription>
      </CardHeader>

      <form onSubmit={handleSubmit}>
        <CardContent className="space-y-3">
          {/* Progress bar */}
          <div>
            <div className="flex justify-between text-[11px] text-muted-foreground mb-1">
              <span>{t("serverTools.swapUsed")}: {swap.used_mb} MB</span>
              <span>{t("serverTools.swapFree")}: {swap.free_mb} MB</span>
            </div>
            <div className="h-2 w-full overflow-hidden rounded-full bg-muted">
              <div
                className="h-full bg-primary transition-all duration-300"
                style={{ width: `${Math.min(100, usedPercent)}%` }}
              />
            </div>
          </div>

          {/* Quick presets for swap size */}
          <div>
            <label className="text-xs font-medium text-foreground">
              {t("serverTools.swapSizeLabel")} (MB)
            </label>
            <div className="mt-1 flex gap-1.5">
              {[1024, 2048, 4096, 8192].map((sz) => (
                <Button
                  key={sz}
                  type="button"
                  variant={totalMb === sz ? "default" : "outline"}
                  size="sm"
                  className="h-7 text-xs flex-1 font-mono"
                  data-testid={`btn-swap-preset-${sz}`}
                  onClick={() => setTotalMb(sz)}
                >
                  {sz / 1024} GB
                </Button>
              ))}
            </div>
          </div>

          <div className="grid grid-cols-2 gap-2">
            <div>
              <label className="text-xs font-medium text-foreground">
                {t("serverTools.customMbLabel")}
              </label>
              <Input
                data-testid="input-swap-total"
                type="number"
                min={0}
                max={32768}
                step={512}
                value={totalMb}
                onChange={(e) => setTotalMb(Number(e.target.value))}
                className="mt-1 text-xs font-mono"
              />
            </div>
          </div>

          <div>
            <label className="flex items-center gap-1.5 text-xs font-medium text-foreground">
              <Lock className="h-3 w-3 text-amber-500" />
              <span>{t("serverTools.sudoPasswordLabel")}</span>
            </label>
            <Input
              data-testid="input-swap-sudo"
              type="password"
              value={sudoPassword}
              onChange={(e) => setSudoPassword(e.target.value)}
              placeholder={t("serverTools.sudoPasswordPlaceholder")}
              className="mt-1 text-xs"
              autoComplete="current-password"
            />
          </div>
        </CardContent>

        <CardFooter className="flex items-center justify-between border-t pt-3">
          <span className="text-[11px] text-muted-foreground">
            {t("serverTools.auditedAction")}
          </span>
          <Button
            type="submit"
            size="sm"
            disabled={!canSubmit || isSubmitting}
            data-testid="btn-save-swap"
            className="gap-1.5 text-xs"
          >
            <Settings2 className="h-3.5 w-3.5" />
            {t("serverTools.applySwapConfig")}
          </Button>
        </CardFooter>
      </form>
    </Card>
  );
}
