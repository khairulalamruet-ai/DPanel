import React, { useState, useEffect } from "react";
import {
  Card,
  CardHeader,
  CardTitle,
  CardDescription,
  CardContent,
  Button,
  useToast,
} from "@builtpanel/ui";
import { useTranslation } from "@builtpanel/i18n";
import { apiFetch } from "@builtpanel/api";
import {
  Settings,
  Shield,
  HardDrive,
  Bell,
  Sliders,
  RefreshCw,
  CheckCircle2,
} from "lucide-react";
import { SettingsModuleEntry, JsonSchemaForm } from "./json-schema-form";

const getModuleIcon = (moduleId: string) => {
  switch (moduleId) {
    case "general":
      return <Sliders className="h-4 w-4" />;
    case "security":
      return <Shield className="h-4 w-4" />;
    case "backup":
      return <HardDrive className="h-4 w-4" />;
    case "notifications":
      return <Bell className="h-4 w-4" />;
    default:
      return <Settings className="h-4 w-4" />;
  }
};

export function SettingsPage() {
  const { t, i18n } = useTranslation();
  const { toast } = useToast();
  const lang = (i18n.language?.startsWith("bn") ? "bn" : "en") as "en" | "bn";

  const [modules, setModules] = useState<SettingsModuleEntry[]>([]);
  const [activeTab, setActiveTab] = useState<string>("general");
  const [isLoading, setIsLoading] = useState(true);
  const [isSaving, setIsSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const fetchSettings = async () => {
    try {
      setIsLoading(true);
      setError(null);
      const res = await apiFetch<{ modules: SettingsModuleEntry[] }>(
        "/api/v1/admin/settings"
      );
      setModules(res.modules || []);
      if (res.modules && res.modules.length > 0 && !res.modules.some((m) => m.id === activeTab)) {
        setActiveTab(res.modules[0].id);
      }
    } catch (err: any) {
      setError(err?.message || "Failed to load settings");
    } finally {
      setIsLoading(false);
    }
  };

  useEffect(() => {
    fetchSettings();
  }, []);

  const activeModule = modules.find((m) => m.id === activeTab);

  const handleSaveSettings = async (newValues: Record<string, any>) => {
    if (!activeModule) return;

    try {
      setIsSaving(true);
      const res = await apiFetch<{ success: boolean; values: Record<string, any> }>(
        `/api/v1/admin/settings/${activeModule.id}`,
        {
          method: "PUT",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ values: newValues }),
        }
      );

      if (res.success) {
        const modTitle =
          activeModule.title[lang] || activeModule.title.en || activeModule.id;
        toast({
          title: modTitle,
          description: t("settings.form.saveSuccess", { module: modTitle }),
          variant: "default",
        });

        // Update local state
        setModules((prev) =>
          prev.map((m) =>
            m.id === activeModule.id ? { ...m, values: res.values } : m
          )
        );
      }
    } catch (err: any) {
      toast({
        title: t("common.error"),
        description: err?.message || t("settings.form.saveError"),
        variant: "destructive",
      });
    } finally {
      setIsSaving(false);
    }
  };

  return (
    <div className="space-y-6">
      {/* Page Header */}
      <div className="flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
        <div>
          <div className="flex items-center gap-2">
            <Settings className="h-6 w-6 text-primary" />
            <h1 className="text-2xl font-bold tracking-tight">{t("settings.title")}</h1>
          </div>
          <p className="text-sm text-muted-foreground mt-1">{t("settings.desc")}</p>
        </div>
        <Button
          variant="outline"
          size="sm"
          onClick={fetchSettings}
          disabled={isLoading}
          className="self-start sm:self-auto gap-2"
        >
          <RefreshCw className={`h-4 w-4 ${isLoading ? "animate-spin" : ""}`} />
          {t("common.reload")}
        </Button>
      </div>

      {isLoading ? (
        <Card className="py-16 text-center text-sm text-muted-foreground">
          <RefreshCw className="h-6 w-6 animate-spin mx-auto mb-2 text-primary" />
          {t("common.loading")}
        </Card>
      ) : error ? (
        <Card className="py-12 text-center space-y-3">
          <p className="text-sm text-destructive">{error}</p>
          <Button variant="outline" size="sm" onClick={fetchSettings}>
            {t("common.retry")}
          </Button>
        </Card>
      ) : modules.length === 0 ? (
        <Card className="py-16 text-center text-sm text-muted-foreground">
          {t("settings.empty")}
        </Card>
      ) : (
        <div className="space-y-6">
          {/* Module Tabs */}
          <div className="border-b border-border/80">
            <nav className="flex space-x-2 overflow-x-auto pb-px" aria-label="Settings Modules">
              {modules.map((mod) => {
                const isActive = mod.id === activeTab;
                const tabTitle = mod.title[lang] || mod.title.en || mod.id;
                return (
                  <button
                    key={mod.id}
                    onClick={() => setActiveTab(mod.id)}
                    className={`flex items-center gap-2 px-4 py-2.5 text-sm font-medium border-b-2 whitespace-nowrap transition-colors ${
                      isActive
                        ? "border-primary text-primary font-semibold"
                        : "border-transparent text-muted-foreground hover:text-foreground hover:border-border"
                    }`}
                  >
                    {getModuleIcon(mod.id)}
                    <span>{tabTitle}</span>
                  </button>
                );
              })}
            </nav>
          </div>

          {/* Active Module Settings Form */}
          {activeModule && (
            <Card className="max-w-3xl">
              <CardHeader className="border-b pb-4">
                <CardTitle className="text-lg">
                  {activeModule.title[lang] || activeModule.title.en}
                </CardTitle>
                <CardDescription>
                  {activeModule.description[lang] || activeModule.description.en}
                </CardDescription>
              </CardHeader>
              <CardContent className="pt-6">
                <JsonSchemaForm
                  schema={activeModule.schema}
                  initialValues={activeModule.values}
                  onSubmit={handleSaveSettings}
                  isSubmitting={isSaving}
                />
              </CardContent>
            </Card>
          )}
        </div>
      )}
    </div>
  );
}
