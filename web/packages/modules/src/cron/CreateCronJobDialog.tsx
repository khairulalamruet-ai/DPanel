import React, { useState } from "react";
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
import { createCronJob } from "./api";
import { Clock, Terminal, AlertCircle } from "lucide-react";
import { useTranslation } from "@builtpanel/i18n";
import { CreateCronJobInput } from "./types";

interface CreateCronJobDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onSaved: () => void;
}

export function CreateCronJobDialog({
  open,
  onOpenChange,
  onSaved,
}: CreateCronJobDialogProps) {
  const { t } = useTranslation();
  const { toast } = useToast();

  const [name, setName] = useState("");
  const [command, setCommand] = useState("");
  const [schedule, setSchedule] = useState("0 * * * *");
  const [timeoutSeconds, setTimeoutSeconds] = useState(300);
  const [validationError, setValidationError] = useState<string | null>(null);
  const [isSubmitting, setIsSubmitting] = useState(false);

  const presets = [
    { label: t("cron.preset5min"), cron: "*/5 * * * *" },
    { label: t("cron.preset15min"), cron: "*/15 * * * *" },
    { label: t("cron.presetHourly"), cron: "0 * * * *" },
    { label: t("cron.presetDaily"), cron: "0 0 * * *" },
    { label: t("cron.presetWeekly"), cron: "0 0 * * 0" },
  ];

  const validateSchedule = (expr: string): string | null => {
    const trimmed = expr.trim();
    const parts = trimmed.split(/\s+/);
    if (parts.length !== 5) {
      return t("cron.error5FieldsRequired");
    }

    const minPart = parts[0];
    // T10.01: Interval rule - minimum interval 5 mins. Reject * or */1..*/4 in minute field
    if (
      minPart === "*" ||
      minPart === "*/1" ||
      minPart === "*/2" ||
      minPart === "*/3" ||
      minPart === "*/4"
    ) {
      return t("cron.errorMinInterval5Min");
    }

    return null;
  };

  const handlePresetSelect = (presetCron: string) => {
    setSchedule(presetCron);
    setValidationError(validateSchedule(presetCron));
  };

  const handleScheduleChange = (val: string) => {
    setSchedule(val);
    setValidationError(validateSchedule(val));
  };

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!name.trim()) return;
    if (!command.trim()) return;

    const error = validateSchedule(schedule);
    if (error) {
      setValidationError(error);
      return;
    }

    try {
      setIsSubmitting(true);
      const payload: CreateCronJobInput = {
        name: name.trim(),
        command: command.trim(),
        schedule: schedule.trim(),
        timeout_seconds: Number(timeoutSeconds) || 300,
      };

      await createCronJob(payload);

      toast({
        title: t("common.success"),
        description: t("cron.jobCreatedSuccess"),
      });

      setName("");
      setCommand("");
      setSchedule("0 * * * *");
      setValidationError(null);
      onOpenChange(false);
      onSaved();
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
      <DialogContent className="sm:max-w-[500px]">
        <DialogHeader>
          <div className="flex items-center gap-2">
            <div className="p-2 rounded-lg bg-primary/10 text-primary">
              <Clock className="w-5 h-5" />
            </div>
            <div>
              <DialogTitle data-testid="create-cron-dialog-title">
                {t("cron.createTitle")}
              </DialogTitle>
              <p className="text-xs text-muted-foreground mt-0.5">
                {t("cron.createSubtitle")}
              </p>
            </div>
          </div>
        </DialogHeader>

        <form onSubmit={handleSubmit} className="space-y-4 py-2">
          {/* Job Name */}
          <div className="space-y-1.5">
            <label className="text-xs font-semibold uppercase tracking-wider text-muted-foreground">
              {t("cron.jobName")} *
            </label>
            <Input
              required
              placeholder={t("cron.jobNamePlaceholder")}
              value={name}
              onChange={(e) => setName(e.target.value)}
              data-testid="input-cron-name"
            />
          </div>

          {/* Command */}
          <div className="space-y-1.5">
            <div className="flex items-center justify-between">
              <label className="text-xs font-semibold uppercase tracking-wider text-muted-foreground">
                {t("cron.command")} *
              </label>
              <span className="text-[11px] text-muted-foreground font-mono">
                bash / tenant UID
              </span>
            </div>
            <div className="relative">
              <Input
                required
                className="font-mono text-xs pl-8"
                placeholder={t("cron.commandPlaceholder")}
                value={command}
                onChange={(e) => setCommand(e.target.value)}
                data-testid="input-cron-command"
              />
              <Terminal className="w-3.5 h-3.5 absolute left-2.5 top-1/2 -translate-y-1/2 text-muted-foreground" />
            </div>
          </div>

          {/* Schedule Presets */}
          <div className="space-y-2">
            <label className="text-xs font-semibold uppercase tracking-wider text-muted-foreground">
              {t("cron.schedulePresets")}
            </label>
            <div className="flex flex-wrap gap-1.5">
              {presets.map((p, idx) => (
                <button
                  key={p.cron}
                  type="button"
                  onClick={() => handlePresetSelect(p.cron)}
                  className={`text-xs px-2.5 py-1 rounded-md border transition-colors ${
                    schedule === p.cron
                      ? "border-primary bg-primary/10 text-primary font-medium"
                      : "border-border bg-background hover:bg-muted text-muted-foreground"
                  }`}
                  data-testid={`preset-cron-${idx}`}
                >
                  {p.label}
                </button>
              ))}
            </div>
          </div>

          {/* 5-Field Cron Expression Input */}
          <div className="space-y-1.5">
            <div className="flex items-center justify-between">
              <label className="text-xs font-semibold uppercase tracking-wider text-muted-foreground">
                {t("cron.cronExpression")} *
              </label>
              <span className="text-[11px] text-muted-foreground font-mono">
                min hour dom mon dow
              </span>
            </div>
            <Input
              required
              className="font-mono text-sm"
              placeholder="* * * * *"
              value={schedule}
              onChange={(e) => handleScheduleChange(e.target.value)}
              data-testid="input-cron-schedule"
            />
            {validationError && (
              <div
                className="flex items-center gap-1.5 text-xs text-destructive pt-0.5"
                data-testid="cron-validation-error"
              >
                <AlertCircle className="w-3.5 h-3.5 flex-shrink-0" />
                <span>{validationError}</span>
              </div>
            )}
          </div>

          {/* Description & Timeout in grid */}
          <div className="grid grid-cols-1 gap-3">
            <div className="space-y-1.5">
              <label className="text-xs font-semibold uppercase tracking-wider text-muted-foreground">
                {t("cron.timeoutSec")}
              </label>
              <Input
                type="number"
                min={10}
                max={3600}
                value={timeoutSeconds}
                onChange={(e) => setTimeoutSeconds(Number(e.target.value))}
                data-testid="input-cron-timeout"
              />
            </div>
          </div>

          <DialogFooter className="pt-2 border-t">
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
              disabled={isSubmitting || !!validationError || !name.trim() || !command.trim()}
              data-testid="btn-submit-cron"
            >
              {isSubmitting ? t("common.saving") : t("cron.createButton")}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
