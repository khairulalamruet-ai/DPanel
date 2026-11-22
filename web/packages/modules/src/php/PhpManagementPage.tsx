import React, { useState, useEffect, useCallback } from "react";
import {
  Card,
  Button,
  useToast,
} from "@builtpanel/ui";
import { loadPhp } from "./api";
import { useTranslation } from "@builtpanel/i18n";
import {
  Cpu,
  RefreshCw,
  Sliders,
  Blocks,
  Globe,
  Zap,
  HardDrive,
} from "lucide-react";
import type { PhpConfigResponse } from "./types";
import { PhpVersionSelector } from "./PhpVersionSelector";
import { PhpIniForm } from "./PhpIniForm";

export function PhpManagementPage() {
  const { t } = useTranslation();
  const { toast } = useToast();

  const [config, setConfig] = useState<PhpConfigResponse | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  const [activeTab, setActiveTab] = useState<"versions" | "ini">("versions");

  const fetchConfig = useCallback(async () => {
    try {
      setIsLoading(true);
      const data = await loadPhp();
      setConfig(data);
    } catch {
      toast({
        title: t("common.error", "Error"),
        description: t("php.errors.fetchFailed", "Failed to load PHP runtime configuration"),
        variant: "destructive",
      });
    } finally {
      setIsLoading(false);
    }
  }, [t, toast]);

  useEffect(() => {
    fetchConfig();
  }, [fetchConfig]);

  return (
    <div className="space-y-6" data-testid="php-management-page">
      {/* Header */}
      <div className="flex flex-col sm:flex-row sm:items-center sm:justify-between gap-4">
        <div>
          <h1 className="text-2xl font-bold tracking-tight flex items-center gap-2">
            <Cpu className="h-6 w-6 text-primary" />
            {t("php.title", "PHP Configuration & Runtime")}
          </h1>
          <p className="text-sm text-muted-foreground mt-1">
            {t(
              "php.subtitle",
              "Manage PHP runtime versions per domain, tune php.ini directives, and inspect installed extensions."
            )}
          </p>
        </div>

        <div className="flex items-center gap-2">
          <Button
            size="sm"
            variant="outline"
            onClick={fetchConfig}
            disabled={isLoading}
            className="text-xs h-9 gap-1.5"
            data-testid="btn-refresh-php"
          >
            <RefreshCw className={`h-4 w-4 ${isLoading ? "animate-spin" : ""}`} />
            {t("common.refresh", "Refresh")}
          </Button>
        </div>
      </div>

      {/* Metrics / Status Summary Cards */}
      {config && (
        <div className="grid grid-cols-2 lg:grid-cols-4 gap-3 sm:gap-4">
          <Card className="p-4 flex items-center justify-between">
            <div>
              <p className="text-xs text-muted-foreground uppercase font-semibold">
                {t("php.metrics.defaultVersion", "Default Runtime")}
              </p>
              <p className="text-2xl font-bold mt-1 font-mono text-primary" data-testid="metric-php-default-version">
                PHP {config.default_version}
              </p>
            </div>
            <Cpu className="h-8 w-8 text-primary/40 shrink-0" />
          </Card>

          <Card className="p-4 flex items-center justify-between">
            <div>
              <p className="text-xs text-muted-foreground uppercase font-semibold">
                {t("php.metrics.domainsConfigured", "Configured Domains")}
              </p>
              <p className="text-2xl font-bold mt-1" data-testid="metric-php-domains-count">
                {config.domain_configs.length}
              </p>
            </div>
            <Globe className="h-8 w-8 text-muted-foreground/40 shrink-0" />
          </Card>

          <Card className="p-4 flex items-center justify-between">
            <div>
              <p className="text-xs text-muted-foreground uppercase font-semibold">
                {t("php.metrics.memoryLimit", "Memory Limit")}
              </p>
              <p className="text-2xl font-bold mt-1 font-mono text-emerald-600 dark:text-emerald-400" data-testid="metric-php-memory-limit">
                {config.ini_settings.memory_limit}
              </p>
            </div>
            <HardDrive className="h-8 w-8 text-emerald-500/40 shrink-0" />
          </Card>

          <Card className="p-4 flex items-center justify-between">
            <div>
              <p className="text-xs text-muted-foreground uppercase font-semibold">
                {t("php.metrics.uploadLimit", "Upload Limit")}
              </p>
              <p className="text-2xl font-bold mt-1 text-primary" data-testid="metric-php-upload">
                {config.ini_settings.upload_max_filesize}
              </p>
            </div>
            <Zap className="h-8 w-8 text-primary/40 shrink-0" />
          </Card>
        </div>
      )}

      {/* Navigation Sub-Tabs */}
      <div className="flex items-center gap-1.5 border-b pb-3">
        <Button
          type="button"
          size="sm"
          variant={activeTab === "versions" ? "default" : "outline"}
          onClick={() => setActiveTab("versions")}
          className="text-xs h-8 gap-1.5 rounded-full"
          data-testid="tab-php-versions"
        >
          <Cpu className="h-3.5 w-3.5" />
          {t("php.tabs.versions", "PHP Versions")}
        </Button>

        <Button
          type="button"
          size="sm"
          variant={activeTab === "ini" ? "default" : "outline"}
          onClick={() => setActiveTab("ini")}
          className="text-xs h-8 gap-1.5 rounded-full"
          data-testid="tab-php-ini"
        >
          <Sliders className="h-3.5 w-3.5" />
          {t("php.tabs.ini", "php.ini Directives")}
        </Button>

      </div>

      {/* Main Tab Panels */}
      {isLoading ? (
        <Card className="p-12 text-center text-xs text-muted-foreground">
          <RefreshCw className="h-6 w-6 animate-spin mx-auto mb-2 text-primary" />
          {t("common.loading", "Loading PHP configuration...")}
        </Card>
      ) : config ? (
        <div className="space-y-6">
          {activeTab === "versions" && (
            <PhpVersionSelector
              defaultVersion={config.default_version}
              supportedVersions={config.supported_versions}
              domainConfigs={config.domain_configs}
              onConfigUpdated={fetchConfig}
            />
          )}

          {activeTab === "ini" && (
            <PhpIniForm
              initialSettings={config.ini_settings}
              planDefaults={config.plan_defaults}
              planLimits={config.plan_limits}
              onUpdated={fetchConfig}
            />
          )}
        </div>
      ) : null}
    </div>
  );
}
