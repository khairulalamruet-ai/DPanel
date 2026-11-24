import React, { useCallback, useEffect, useState } from "react";
import { Card, CardHeader, CardTitle, CardContent, Button, Input, useToast } from "@builtpanel/ui";
import { useTranslation } from "@builtpanel/i18n";
import { CalendarClock } from "lucide-react";
import { getSchedule, saveSchedule, removeSchedule } from "./api";

const PRESETS: { label: string; cron: string }[] = [
  { label: "Every night, 03:00", cron: "0 3 * * *" },
  { label: "Sundays, 03:00", cron: "0 3 * * 0" },
  { label: "Every 12 hours", cron: "0 */12 * * *" },
];

/** The account's own backup schedule: when, and how many of the old ones are kept. */
export function BackupScheduleCard() {
  const { t } = useTranslation();
  const { toast } = useToast();

  const [exists, setExists] = useState(false);
  const [enabled, setEnabled] = useState(true);
  const [cron, setCron] = useState("0 3 * * *");
  const [keepDaily, setKeepDaily] = useState(7);
  const [keepWeekly, setKeepWeekly] = useState(4);
  const [isSaving, setIsSaving] = useState(false);

  const load = useCallback(async () => {
    try {
      const s = await getSchedule();
      setExists(!!s);
      if (s) {
        setEnabled(s.enabled);
        setCron(s.cron);
        setKeepDaily(s.keep_daily);
        setKeepWeekly(s.keep_weekly);
      }
    } catch {
      // The card stays with its defaults; saving shows what is wrong.
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const fail = (err: any) =>
    toast({
      title: t("common.error"),
      description: err?.message || t("common.unknownError"),
      variant: "destructive",
    });

  const handleSave = async (e: React.FormEvent) => {
    e.preventDefault();
    try {
      setIsSaving(true);
      await saveSchedule({ cron: cron.trim(), enabled, keep_daily: keepDaily, keep_weekly: keepWeekly });
      setExists(true);
      toast({ title: t("common.success"), description: t("backups.scheduleSaved", "Backup schedule saved") });
    } catch (err) {
      fail(err);
    } finally {
      setIsSaving(false);
    }
  };

  const handleRemove = async () => {
    try {
      await removeSchedule();
      setExists(false);
      toast({ title: t("common.success"), description: t("backups.scheduleRemoved", "Backup schedule removed; the backups already made stay") });
    } catch (err) {
      fail(err);
    }
  };

  return (
    <Card data-testid="card-backup-schedule">
      <CardHeader className="py-3 px-4 border-b">
        <CardTitle className="text-sm font-semibold flex items-center gap-2">
          <CalendarClock className="w-4 h-4 text-primary" />
          {t("backups.scheduleTitle", "Automatic backups")}
        </CardTitle>
      </CardHeader>
      <CardContent className="p-4">
        <form onSubmit={handleSave} className="grid grid-cols-1 md:grid-cols-5 gap-3 items-end">
          <div className="md:col-span-2 space-y-1.5">
            <label className="text-xs font-semibold text-muted-foreground uppercase tracking-wider">
              {t("backups.scheduleWhen", "When (cron, UTC)")}
            </label>
            <Input value={cron} onChange={(e) => setCron(e.target.value)} className="font-mono text-xs" data-testid="input-backup-cron" />
            <div className="flex flex-wrap gap-1.5">
              {PRESETS.map((p) => (
                <button
                  key={p.cron}
                  type="button"
                  className="px-2 py-0.5 rounded bg-secondary hover:bg-secondary/80 text-[11px]"
                  onClick={() => setCron(p.cron)}
                >
                  {p.label}
                </button>
              ))}
            </div>
          </div>
          <div className="space-y-1.5">
            <label className="text-xs font-semibold text-muted-foreground uppercase tracking-wider">
              {t("backups.keepDaily", "Keep daily")}
            </label>
            <Input type="number" min={1} value={keepDaily} onChange={(e) => setKeepDaily(Number(e.target.value))} data-testid="input-keep-daily" />
          </div>
          <div className="space-y-1.5">
            <label className="text-xs font-semibold text-muted-foreground uppercase tracking-wider">
              {t("backups.keepWeekly", "Keep weekly")}
            </label>
            <Input type="number" min={0} value={keepWeekly} onChange={(e) => setKeepWeekly(Number(e.target.value))} data-testid="input-keep-weekly" />
          </div>
          <div className="flex items-center gap-2">
            <label className="flex items-center gap-1.5 text-xs">
              <input type="checkbox" checked={enabled} onChange={(e) => setEnabled(e.target.checked)} data-testid="checkbox-schedule-enabled" />
              {t("common.enabled", "On")}
            </label>
            <Button type="submit" size="sm" disabled={isSaving} data-testid="btn-save-backup-schedule">
              {t("common.save", "Save")}
            </Button>
            {exists && (
              <Button type="button" size="sm" variant="outline" onClick={handleRemove} data-testid="btn-remove-backup-schedule">
                {t("common.delete", "Remove")}
              </Button>
            )}
          </div>
        </form>
      </CardContent>
    </Card>
  );
}
