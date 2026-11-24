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
import { Globe, Clock, Lock, Check } from "lucide-react";
import { useTranslation } from "@builtpanel/i18n";

const COMMON_TIMEZONES = [
  "UTC",
  "Asia/Dhaka",
  "Asia/Kolkata",
  "Asia/Singapore",
  "Asia/Tokyo",
  "Europe/London",
  "Europe/Berlin",
  "Europe/Paris",
  "America/New_York",
  "America/Chicago",
  "America/Los_Angeles",
  "Australia/Sydney",
];

interface TimezoneCardProps {
  currentTimezone: string;
  currentTimeStr: string;
  onUpdateTimezone: (timezone: string, sudoPassword: string) => Promise<void>;
}

export function TimezoneCard({
  currentTimezone,
  currentTimeStr,
  onUpdateTimezone,
}: TimezoneCardProps) {
  const { t } = useTranslation();
  const [selectedTz, setSelectedTz] = useState(currentTimezone || "UTC");
  const [sudoPassword, setSudoPassword] = useState("");
  const [isSubmitting, setIsSubmitting] = useState(false);

  React.useEffect(() => {
    if (currentTimezone) {
      setSelectedTz(currentTimezone);
    }
  }, [currentTimezone]);

  const hasChanged = selectedTz !== currentTimezone;
  const canSubmit = hasChanged && sudoPassword.trim().length > 0;

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!canSubmit) return;
    try {
      setIsSubmitting(true);
      await onUpdateTimezone(selectedTz, sudoPassword);
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
            <Globe className="h-4 w-4 text-primary" />
            <span data-testid="title-timezone-config">{t("serverTools.timezoneTitle")}</span>
          </CardTitle>
          <Badge variant="outline" className="font-mono text-xs" data-testid="badge-current-timezone">
            {currentTimezone}
          </Badge>
        </div>
        <CardDescription className="flex items-center gap-1.5 text-xs text-muted-foreground">
          <Clock className="h-3.5 w-3.5" />
          <span data-testid="text-server-time">{currentTimeStr}</span>
        </CardDescription>
      </CardHeader>

      <form onSubmit={handleSubmit}>
        <CardContent className="space-y-3">
          <div>
            <label className="text-xs font-medium text-foreground">
              {t("serverTools.selectTimezoneLabel")}
            </label>
            <select
              data-testid="select-timezone"
              value={selectedTz}
              onChange={(e) => setSelectedTz(e.target.value)}
              className="mt-1 w-full rounded-md border border-input bg-background px-3 py-1.5 text-xs font-mono shadow-sm focus:outline-none focus:ring-1 focus:ring-primary"
            >
              {COMMON_TIMEZONES.map((tz) => (
                <option key={tz} value={tz}>
                  {tz}
                </option>
              ))}
            </select>
          </div>

          <div>
            <label className="flex items-center gap-1.5 text-xs font-medium text-foreground">
              <Lock className="h-3 w-3 text-amber-500" />
              <span>{t("serverTools.sudoPasswordLabel")}</span>
            </label>
            <Input
              data-testid="input-tz-sudo"
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
            data-testid="btn-save-timezone"
            className="gap-1.5 text-xs"
          >
            <Check className="h-3.5 w-3.5" />
            {t("common.save")}
          </Button>
        </CardFooter>
      </form>
    </Card>
  );
}
