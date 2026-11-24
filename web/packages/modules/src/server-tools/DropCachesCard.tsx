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
import { Sparkles, Lock, Trash2, CheckCircle2 } from "lucide-react";
import { useTranslation } from "@builtpanel/i18n";
import { ServerMemoryInfo } from "./types";

interface DropCachesCardProps {
  memory: ServerMemoryInfo;
  onDropCaches: (sudoPassword: string) => Promise<void>;
}

export function DropCachesCard({ memory, onDropCaches }: DropCachesCardProps) {
  const { t } = useTranslation();
  const [sudoPassword, setSudoPassword] = useState("");
  const [isSubmitting, setIsSubmitting] = useState(false);

  const canSubmit = sudoPassword.trim().length > 0;

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!canSubmit) return;
    try {
      setIsSubmitting(true);
      await onDropCaches(sudoPassword);
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
            <Sparkles className="h-4 w-4 text-primary" />
            <span data-testid="title-drop-caches">{t("serverTools.dropCachesTitle")}</span>
          </CardTitle>
          <Badge variant="outline" className="font-mono text-xs" data-testid="badge-cached-memory">
            {memory.cached_mb} MB {t("serverTools.cached")}
          </Badge>
        </div>
        <CardDescription className="text-xs text-muted-foreground">
          {t("serverTools.dropCachesDescription")}
        </CardDescription>
      </CardHeader>

      <form onSubmit={handleSubmit}>
        <CardContent className="space-y-3">
          <div className="grid grid-cols-3 gap-2 rounded-md border bg-muted/20 p-2.5 text-center">
            <div>
              <span className="text-[10px] text-muted-foreground uppercase">{t("serverTools.buffers")}</span>
              <p className="font-mono text-xs font-semibold">{memory.buffers_mb} MB</p>
            </div>
            <div>
              <span className="text-[10px] text-muted-foreground uppercase">{t("serverTools.pageCache")}</span>
              <p className="font-mono text-xs font-semibold">{memory.cached_mb} MB</p>
            </div>
            <div>
              <span className="text-[10px] text-muted-foreground uppercase">{t("serverTools.available")}</span>
              <p className="font-mono text-xs font-semibold text-emerald-600 dark:text-emerald-400">
                {memory.available_mb} MB
              </p>
            </div>
          </div>

          <div>
            <label className="flex items-center gap-1.5 text-xs font-medium text-foreground">
              <Lock className="h-3 w-3 text-amber-500" />
              <span>{t("serverTools.sudoPasswordLabel")}</span>
            </label>
            <Input
              data-testid="input-drop-caches-sudo"
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
            variant="secondary"
            disabled={!canSubmit || isSubmitting}
            data-testid="btn-execute-drop-caches"
            className="gap-1.5 text-xs"
          >
            {isSubmitting ? (
              <Trash2 className="h-3.5 w-3.5 animate-spin" />
            ) : (
              <Trash2 className="h-3.5 w-3.5 text-amber-500" />
            )}
            {t("serverTools.dropCachesButton")}
          </Button>
        </CardFooter>
      </form>
    </Card>
  );
}
