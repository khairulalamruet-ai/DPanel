import React, { useState, useMemo } from "react";
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
import { Globe, Clock, Search, ShieldCheck, Check, Loader2 } from "lucide-react";
import { useTranslation } from "@builtpanel/i18n";
import { SudoConfirmModal } from "../SudoConfirmModal";

export const ALL_TIMEZONES = [
  "UTC",
  "Africa/Cairo",
  "Africa/Casablanca",
  "Africa/Johannesburg",
  "Africa/Lagos",
  "Africa/Nairobi",
  "America/Anchorage",
  "America/Argentina/Buenos_Aires",
  "America/Bogota",
  "America/Chicago",
  "America/Denver",
  "America/Halifax",
  "America/Los_Angeles",
  "America/Mexico_City",
  "America/New_York",
  "America/Phoenix",
  "America/Santiago",
  "America/Sao_Paulo",
  "America/Toronto",
  "America/Vancouver",
  "Asia/Almaty",
  "Asia/Amman",
  "Asia/Baghdad",
  "Asia/Baku",
  "Asia/Bangkok",
  "Asia/Beirut",
  "Asia/Dhaka",
  "Asia/Dubai",
  "Asia/Hong_Kong",
  "Asia/Jakarta",
  "Asia/Jerusalem",
  "Asia/Karachi",
  "Asia/Kathmandu",
  "Asia/Kolkata",
  "Asia/Kuala_Lumpur",
  "Asia/Kuwait",
  "Asia/Manila",
  "Asia/Qatar",
  "Asia/Riyadh",
  "Asia/Seoul",
  "Asia/Shanghai",
  "Asia/Singapore",
  "Asia/Taipei",
  "Asia/Tashkent",
  "Asia/Tbilisi",
  "Asia/Tehran",
  "Asia/Tokyo",
  "Asia/Yerevan",
  "Atlantic/Reykjavik",
  "Australia/Adelaide",
  "Australia/Brisbane",
  "Australia/Melbourne",
  "Australia/Perth",
  "Australia/Sydney",
  "Europe/Amsterdam",
  "Europe/Athens",
  "Europe/Belgrade",
  "Europe/Berlin",
  "Europe/Brussels",
  "Europe/Bucharest",
  "Europe/Budapest",
  "Europe/Copenhagen",
  "Europe/Dublin",
  "Europe/Helsinki",
  "Europe/Istanbul",
  "Europe/Kyiv",
  "Europe/Lisbon",
  "Europe/London",
  "Europe/Madrid",
  "Europe/Oslo",
  "Europe/Paris",
  "Europe/Prague",
  "Europe/Rome",
  "Europe/Stockholm",
  "Europe/Vienna",
  "Europe/Warsaw",
  "Europe/Zurich",
  "Pacific/Auckland",
  "Pacific/Fiji",
  "Pacific/Honolulu",
];

interface ServerTimeTabProps {
  currentTimezone: string;
  currentTimeStr: string;
  canManage: boolean;
  onUpdateTimezone: (timezone: string, sudoPassword: string) => Promise<void>;
}

export function ServerTimeTab({
  currentTimezone,
  currentTimeStr,
  canManage,
  onUpdateTimezone,
}: ServerTimeTabProps) {
  const { t } = useTranslation();
  const [selectedTz, setSelectedTz] = useState(currentTimezone || "UTC");
  const [searchQuery, setSearchQuery] = useState("");
  const [isSudoModalOpen, setIsSudoModalOpen] = useState(false);
  const [isUpdating, setIsUpdating] = useState(false);

  React.useEffect(() => {
    if (currentTimezone) {
      setSelectedTz(currentTimezone);
    }
  }, [currentTimezone]);

  const filteredTimezones = useMemo(() => {
    const q = searchQuery.trim().toLowerCase();
    if (!q) return ALL_TIMEZONES;
    return ALL_TIMEZONES.filter((tz) => tz.toLowerCase().includes(q));
  }, [searchQuery]);

  const hasChanged = selectedTz !== currentTimezone;

  const handleApply = async (sudoPassword: string) => {
    setIsUpdating(true);
    try {
      await onUpdateTimezone(selectedTz, sudoPassword);
    } finally {
      setIsUpdating(false);
    }
  };

  return (
    <div className="space-y-6">
      <Card>
        <CardHeader>
          <div className="flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
            <div>
              <CardTitle className="flex items-center gap-2 text-base font-semibold">
                <Globe className="h-4 w-4 text-primary" />
                <span data-testid="title-timezone-config">{t("system.time.title", "Server Time & Timezone")}</span>
              </CardTitle>
              <CardDescription className="text-xs text-muted-foreground mt-1">
                {t("system.time.description", "Inspect the server clock, preview local timestamp representations, and update system timezone via timedatectl")}
              </CardDescription>
            </div>
            <div className="flex items-center gap-2">
              <Badge variant="outline" className="font-mono text-xs gap-1 py-1" data-testid="badge-current-timezone">
                <Clock className="h-3 w-3 text-muted-foreground" />
                <span>{currentTimezone}</span>
              </Badge>
            </div>
          </div>
        </CardHeader>

        <CardContent className="space-y-4">
          {/* Clock preview card */}
          <div className="rounded-lg border bg-muted/30 p-4">
            <span className="text-[11px] font-medium uppercase tracking-wider text-muted-foreground">
              {t("system.time.currentClock", "Current Server Clock")}
            </span>
            <div className="mt-1 flex items-baseline gap-3">
              <p className="font-mono text-2xl font-bold tracking-tight text-foreground" data-testid="text-server-time">
                {currentTimeStr}
              </p>
              <span className="text-xs text-muted-foreground">
                ({currentTimezone})
              </span>
            </div>
          </div>

          {/* Searchable Timezone Picker */}
          <div className="space-y-2">
            <label className="text-xs font-medium text-foreground">
              {t("system.time.selectLabel", "Select Timezone (Searchable List)")}
            </label>
            
            <div className="relative">
              <Search className="absolute left-3 top-2.5 h-3.5 w-3.5 text-muted-foreground" />
              <Input
                data-testid="input-timezone-search"
                type="text"
                value={searchQuery}
                onChange={(e) => setSearchQuery(e.target.value)}
                placeholder={t("system.time.searchPlaceholder", "Search timezone (e.g. Dhaka, London, UTC)...")}
                className="pl-9 text-xs"
              />
            </div>

            <div className="max-h-56 overflow-y-auto rounded-md border bg-background p-1 space-y-0.5">
              {filteredTimezones.length === 0 ? (
                <p className="py-4 text-center text-xs text-muted-foreground">
                  {t("common.noMatchingResults", "No timezones match your search")}
                </p>
              ) : (
                filteredTimezones.map((tz) => {
                  const isSelected = selectedTz === tz;
                  return (
                    <button
                      key={tz}
                      type="button"
                      data-testid={`tz-option-${tz}`}
                      onClick={() => setSelectedTz(tz)}
                      className={`flex w-full items-center justify-between rounded px-2.5 py-1.5 text-left text-xs font-mono transition-colors ${
                        isSelected
                          ? "bg-primary text-primary-foreground font-semibold"
                          : "text-foreground hover:bg-muted"
                      }`}
                    >
                      <span>{tz}</span>
                      {isSelected && <Check className="h-3.5 w-3.5" />}
                    </button>
                  );
                })
              )}
            </div>

            {hasChanged && (
              <div className="rounded-md border border-primary/20 bg-primary/5 p-2.5 text-xs text-primary flex items-center justify-between">
                <span>
                  {t("system.time.selectedPending", "Selected timezone:")} <strong className="font-mono">{selectedTz}</strong>
                </span>
                <span className="text-[11px] opacity-80">
                  {t("system.time.requiresSudo", "Sudo authentication required to save")}
                </span>
              </div>
            )}
          </div>
        </CardContent>

        <CardFooter className="flex flex-col gap-2 border-t px-6 py-3 sm:flex-row sm:items-center sm:justify-between">
          <div className="flex items-center gap-1.5 text-xs text-muted-foreground">
            <ShieldCheck className="h-3.5 w-3.5 text-emerald-600 dark:text-emerald-400" />
            <span>{t("serverTools.auditedAction", "All server operations are cryptographically audited")}</span>
          </div>

          <Button
            size="sm"
            disabled={!hasChanged || !canManage || isUpdating}
            data-testid="btn-apply-timezone"
            onClick={() => setIsSudoModalOpen(true)}
            className="gap-1.5 text-xs"
          >
            {isUpdating ? (
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
            ) : (
              <Globe className="h-3.5 w-3.5" />
            )}
            {t("system.time.applyButton", "Save Timezone")}
          </Button>
        </CardFooter>
      </Card>

      <SudoConfirmModal
        open={isSudoModalOpen}
        onOpenChange={setIsSudoModalOpen}
        title={t("system.time.sudoConfirmTitle", "Confirm Timezone Update")}
        description={t(
          "system.time.sudoConfirmDescription",
          "Applying a new timezone modifies node system clock configuration (/etc/localtime and timedatectl). Please enter admin sudo password to verify.",
        )}
        confirmLabel={t("system.time.applyButton", "Save Timezone")}
        onConfirm={handleApply}
      />
    </div>
  );
}
