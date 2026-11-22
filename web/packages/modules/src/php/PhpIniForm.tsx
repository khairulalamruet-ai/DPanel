import React, { useState, useEffect } from "react";
import {
  Card,
  CardHeader,
  CardTitle,
  CardDescription,
  CardContent,
  Button,
  Input,
  Badge,
  useToast,
} from "@builtpanel/ui";
import { saveIni, resetIni } from "./api";
import { useTranslation } from "@builtpanel/i18n";
import { Settings2, RotateCcw, Save, AlertCircle } from "lucide-react";
import type { PhpIniSettings, PhpPlanLimits } from "./types";

interface PhpIniFormProps {
  initialSettings: PhpIniSettings;
  planDefaults: PhpIniSettings;
  planLimits: PhpPlanLimits;
  onUpdated: () => void;
}

export function PhpIniForm({
  initialSettings,
  planDefaults,
  planLimits,
  onUpdated,
}: PhpIniFormProps) {
  const { t } = useTranslation();
  const { toast } = useToast();

  const [formData, setFormData] = useState<PhpIniSettings>(initialSettings);
  const [isSaving, setIsSaving] = useState(false);
  const [isResetting, setIsResetting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    setFormData(initialSettings);
  }, [initialSettings]);

  const handleChange = (key: keyof PhpIniSettings, value: any) => {
    setFormData((prev) => ({ ...prev, [key]: value }));
    setError(null);
  };

  const handleSave = async (e: React.FormEvent) => {
    e.preventDefault();
    try {
      setIsSaving(true);
      setError(null);
      await saveIni(formData);
      toast({
        title: t("common.success", "Success"),
        description: t("php.toast.iniSaved", "php.ini directives updated successfully"),
      });
      onUpdated();
    } catch (err: any) {
      setError(err?.message || t("php.errors.iniSaveFailed", "Failed to update php.ini directives"));
      toast({
        title: t("common.error", "Error"),
        description: err?.message || t("php.errors.iniSaveFailed", "Failed to update php.ini directives"),
        variant: "destructive",
      });
    } finally {
      setIsSaving(false);
    }
  };

  const handleResetToDefaults = async () => {
    try {
      setIsResetting(true);
      setError(null);
      await resetIni();
      setFormData(planDefaults);
      toast({
        title: t("common.success", "Success"),
        description: t(
          "php.toast.iniReset",
          "php.ini configuration reverted to hosting plan defaults"
        ),
      });
      onUpdated();
    } catch (err: any) {
      setError(err?.message || "Failed to reset php.ini directives");
    } finally {
      setIsResetting(false);
    }
  };

  return (
    <Card className="border" data-testid="php-ini-form-card">
      <CardHeader className="pb-4">
        <div className="flex flex-col sm:flex-row sm:items-center sm:justify-between gap-3">
          <div>
            <CardTitle className="text-base flex items-center gap-2">
              <Settings2 className="h-5 w-5 text-primary" />
              {t("php.iniFormTitle", "PHP Directives Configuration (php.ini)")}
            </CardTitle>
            <CardDescription className="text-xs mt-1">
              {t(
                "php.iniFormDesc",
                "Adjust runtime limits and script behavior within your hosting plan quota."
              )}
            </CardDescription>
          </div>

          {/* Reset to plan defaults button (DoD requirement) */}
          <Button
            type="button"
            variant="outline"
            size="sm"
            onClick={handleResetToDefaults}
            disabled={isResetting || isSaving}
            className="text-xs h-8 gap-1.5 border-dashed"
            data-testid="btn-reset-php-ini"
            title="Reset to default settings"
          >
            <RotateCcw className={`h-3.5 w-3.5 ${isResetting ? "animate-spin" : ""}`} />
            {t("php.actions.resetDefaults", "Reset to Defaults")}
          </Button>
        </div>
      </CardHeader>

      <CardContent>
        <form onSubmit={handleSave} className="space-y-6">
          {error && (
            <div
              className="p-3 text-xs bg-destructive/10 border border-destructive/20 text-destructive rounded-lg flex items-center gap-2"
              data-testid="banner-php-ini-error"
            >
              <AlertCircle className="h-4 w-4 shrink-0" />
              <span>{error}</span>
            </div>
          )}

          {/* Section 1: Memory & Execution Limits */}
          <div className="space-y-3">
            <h3 className="text-xs font-semibold uppercase tracking-wider text-muted-foreground border-b pb-1">
              {t("php.sections.resourceLimits", "Resource Limits")}
            </h3>

            <div className="grid grid-cols-1 md:grid-cols-3 gap-4">
              <div>
                <div className="flex items-center justify-between mb-1">
                  <label htmlFor="input-memory-limit" className="text-xs font-medium text-foreground">
                    memory_limit
                  </label>
                  <Badge variant="secondary" className="text-[10px] h-4 font-mono">
                    Max: {planLimits.max_memory_limit}
                  </Badge>
                </div>
                <Input
                  id="input-memory-limit"
                  value={formData.memory_limit}
                  onChange={(e) => handleChange("memory_limit", e.target.value)}
                  placeholder="256M"
                  className="font-mono text-xs h-9"
                  data-testid="input-memory-limit"
                />
                <p className="text-[11px] text-muted-foreground mt-1">
                  Maximum RAM script can allocate (e.g. 128M, 256M, 512M)
                </p>
              </div>

              <div>
                <div className="flex items-center justify-between mb-1">
                  <label htmlFor="input-max-execution-time" className="text-xs font-medium text-foreground">
                    max_execution_time
                  </label>
                  <Badge variant="secondary" className="text-[10px] h-4 font-mono">
                    Max: {planLimits.max_execution_time}s
                  </Badge>
                </div>
                <Input
                  id="input-max-execution-time"
                  type="number"
                  value={formData.max_execution_time}
                  onChange={(e) => handleChange("max_execution_time", Number(e.target.value))}
                  placeholder="60"
                  className="font-mono text-xs h-9"
                  data-testid="input-max-execution-time"
                />
                <p className="text-[11px] text-muted-foreground mt-1">
                  Max time in seconds a script is allowed to run
                </p>
              </div>

              <div>
                <div className="flex items-center justify-between mb-1">
                  <label htmlFor="input-max-input-time" className="text-xs font-medium text-foreground">
                    max_input_time
                  </label>
                </div>
                <Input
                  id="input-max-input-time"
                  type="number"
                  value={formData.max_input_time}
                  onChange={(e) => handleChange("max_input_time", Number(e.target.value))}
                  placeholder="60"
                  className="font-mono text-xs h-9"
                  data-testid="input-max-input-time"
                />
                <p className="text-[11px] text-muted-foreground mt-1">
                  Max time in seconds parsing input data (e.g. POST, GET)
                </p>
              </div>
            </div>
          </div>

          {/* Section 2: Uploads & Payload Size */}
          <div className="space-y-3">
            <h3 className="text-xs font-semibold uppercase tracking-wider text-muted-foreground border-b pb-1">
              {t("php.sections.uploads", "Uploads & POST Size")}
            </h3>

            <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
              <div>
                <div className="flex items-center justify-between mb-1">
                  <label htmlFor="input-upload-max-filesize" className="text-xs font-medium text-foreground">
                    upload_max_filesize
                  </label>
                  <Badge variant="secondary" className="text-[10px] h-4 font-mono">
                    Max: {planLimits.max_upload_filesize}
                  </Badge>
                </div>
                <Input
                  id="input-upload-max-filesize"
                  value={formData.upload_max_filesize}
                  onChange={(e) => handleChange("upload_max_filesize", e.target.value)}
                  placeholder="64M"
                  className="font-mono text-xs h-9"
                  data-testid="input-upload-max-filesize"
                />
                <p className="text-[11px] text-muted-foreground mt-1">
                  Maximum size of an uploaded file
                </p>
              </div>

              <div>
                <div className="flex items-center justify-between mb-1">
                  <label htmlFor="input-post-max-size" className="text-xs font-medium text-foreground">
                    post_max_size
                  </label>
                </div>
                <Input
                  id="input-post-max-size"
                  value={formData.post_max_size}
                  onChange={(e) => handleChange("post_max_size", e.target.value)}
                  placeholder="64M"
                  className="font-mono text-xs h-9"
                  data-testid="input-post-max-size"
                />
                <p className="text-[11px] text-muted-foreground mt-1">
                  Maximum size of POST request payload (should be {">="} upload_max_filesize)
                </p>
              </div>
            </div>
          </div>

          {/* Section 3: Performance, Caching & Timezone */}
          <div className="space-y-3">
            <h3 className="text-xs font-semibold uppercase tracking-wider text-muted-foreground border-b pb-1">
              {t("php.sections.engine", "Engine & Caching")}
            </h3>

            <div className="grid grid-cols-1 md:grid-cols-3 gap-4">
              <div className="p-3 border rounded-lg flex items-center justify-between">
                <div>
                  <label htmlFor="checkbox-display-errors" className="text-xs font-semibold text-foreground block">
                    display_errors
                  </label>
                  <p className="text-[11px] text-muted-foreground">Print runtime warnings & fatal errors</p>
                </div>
                <input
                  id="checkbox-display-errors"
                  type="checkbox"
                  checked={formData.display_errors}
                  onChange={(e) => handleChange("display_errors", e.target.checked)}
                  className="h-4 w-4 rounded text-primary focus:ring-primary"
                  data-testid="checkbox-display-errors"
                />
              </div>

              <div>
                <label htmlFor="input-date-timezone" className="text-xs font-medium text-foreground mb-1 block">
                  date.timezone
                </label>
                <Input
                  id="input-date-timezone"
                  value={formData.date_timezone}
                  onChange={(e) => handleChange("date_timezone", e.target.value)}
                  placeholder="UTC"
                  className="font-mono text-xs h-9"
                  data-testid="input-date-timezone"
                />
                <p className="text-[11px] text-muted-foreground mt-1">
                  Default server timezone (e.g. UTC, Asia/Dhaka)
                </p>
              </div>
            </div>
          </div>

          <div className="flex items-center justify-end pt-3 border-t">
            <Button
              type="submit"
              disabled={isSaving}
              className="text-xs h-9 gap-1.5"
              data-testid="btn-save-php-ini"
            >
              <Save className="h-4 w-4" />
              {isSaving ? t("common.processing", "Saving...") : t("php.actions.saveIni", "Save php.ini Settings")}
            </Button>
          </div>
        </form>
      </CardContent>
    </Card>
  );
}
