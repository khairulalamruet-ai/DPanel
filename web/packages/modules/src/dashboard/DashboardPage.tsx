import React, { useState, useEffect, useCallback, useMemo } from "react";
import { AdminQuickstartChecklist } from "../onboarding";
import { useTranslation } from "@builtpanel/i18n";
import { useSession } from "@builtpanel/auth";
import { loadAdminSummary } from "./api";
import { Button } from "@builtpanel/ui";
import { LayoutGrid, RefreshCw, AlertCircle } from "lucide-react";
import { HealthCard } from "./HealthCard";
import {
  DashboardWidgetItem,
  DashboardWidgetId,
  AdminDashboardSummary,
} from "./types";
import { DashboardLayoutDialog } from "./DashboardLayoutDialog";
import {
  NodesStatusWidget,
  TenantsCountWidget,
  ResourceUsageWidget,
  RecentOperationsWidget,
  SecurityAlertsWidget,
  LicenseStatusWidget,
} from "./DashboardWidgets";

const STORAGE_KEY = "builtpanel-admin-dashboard-layout";

const DEFAULT_WIDGETS: DashboardWidgetItem[] = [
  { id: "nodes_status", enabled: true, order: 0 },
  { id: "tenants_count", enabled: true, order: 1 },
  { id: "resource_usage", enabled: true, order: 2 },
  { id: "recent_operations", enabled: true, order: 3 },
  { id: "security_alerts", enabled: true, order: 4 },
  { id: "license_status", enabled: true, order: 5 },
];

// Until the numbers arrive the page shows zeros, never made-up figures.
const DEFAULT_SUMMARY: AdminDashboardSummary = {
  nodes: { total: 0, online: 0, avg_cpu_percent: 0, avg_ram_percent: 0 },
  tenants: { total: 0, active: 0, domains_count: 0, databases_count: 0 },
  resources: {
    cpu_percent: 0,
    memory_used_mb: 0,
    memory_total_mb: 0,
    disk_used_gb: 0,
    disk_total_gb: 0,
    network_rx_mb: 0,
    network_tx_mb: 0,
  },
  operations: [],
  security: { blocked_threats_24h: 0, active_waf_rules: 0, ssh_bans: 0 },
  license: { tier: "", status: "active", signature_valid: true, expires_in_days: 0 },
};

export function DashboardPage() {
  const { t } = useTranslation();
  const { data: session } = useSession();

  const [layoutDialog, setLayoutDialog] = useState(false);
  const [widgets, setWidgets] = useState<DashboardWidgetItem[]>(() => {
    try {
      const saved = localStorage.getItem(STORAGE_KEY);
      if (saved) {
        const parsed = JSON.parse(saved);
        if (Array.isArray(parsed) && parsed.length > 0) return parsed;
      }
    } catch {
      // Ignore parse failure
    }
    return DEFAULT_WIDGETS;
  });

  const [summary, setSummary] = useState<AdminDashboardSummary>(DEFAULT_SUMMARY);
  const [isLoading, setIsLoading] = useState(false);

  const loadSummary = useCallback(async () => {
    try {
      setIsLoading(true);
      const res = await loadAdminSummary();
      setSummary(res);
    } catch {
      // Use fallback defaults
    } finally {
      setIsLoading(false);
    }
  }, []);

  useEffect(() => {
    loadSummary();
  }, [loadSummary]);

  const handleSaveLayout = (newWidgets: DashboardWidgetItem[]) => {
    setWidgets(newWidgets);
    try {
      localStorage.setItem(STORAGE_KEY, JSON.stringify(newWidgets));
    } catch {
      // Ignore storage errors
    }
  };

  const handleResetLayout = () => {
    setWidgets(DEFAULT_WIDGETS);
    try {
      localStorage.removeItem(STORAGE_KEY);
    } catch {
      // Ignore
    }
  };

  // Sort and filter active widgets
  const activeWidgets = useMemo(() => {
    return [...widgets]
      .filter((w) => w.enabled)
      .sort((a, b) => a.order - b.order);
  }, [widgets]);

  const renderWidget = (id: DashboardWidgetId) => {
    switch (id) {
      case "nodes_status":
        return <NodesStatusWidget key={id} summary={summary} />;
      case "tenants_count":
        return <TenantsCountWidget key={id} summary={summary} />;
      case "resource_usage":
        return <ResourceUsageWidget key={id} summary={summary} />;
      case "recent_operations":
        return <RecentOperationsWidget key={id} summary={summary} />;
      case "security_alerts":
        return <SecurityAlertsWidget key={id} summary={summary} />;
      case "license_status":
        return <LicenseStatusWidget key={id} summary={summary} />;
      default:
        return null;
    }
  };

  return (
    <div className="space-y-6" data-testid="admin-dashboard-page">
      {/* Header */}
      <div className="flex flex-col sm:flex-row sm:items-center sm:justify-between gap-4">
        <div>
          <h1 className="text-2xl font-bold tracking-tight" data-testid="dashboard-page-title">
            {t("common.dashboard", "Control Center")}
          </h1>
          <p className="text-sm text-muted-foreground">
            {session?.branding?.companyName || "Built Panel"} Cloud Infrastructure Overview
          </p>
        </div>

        <div className="flex items-center gap-2">
          <Button
            variant="outline"
            size="sm"
            onClick={() => setLayoutDialog(true)}
            className="gap-2 shrink-0 text-xs h-9"
            data-testid="btn-customize-dashboard"
          >
            <LayoutGrid className="h-4 w-4" />
            <span>{t("dashboard.customizeLayout", "Customize Layout")}</span>
          </Button>

          <Button
            variant="outline"
            size="sm"
            onClick={loadSummary}
            disabled={isLoading}
            className="gap-2 shrink-0 text-xs h-9"
            data-testid="btn-refresh-dashboard"
          >
            <RefreshCw className={`h-4 w-4 ${isLoading ? "animate-spin" : ""}`} />
            <span>{t("common.refresh", "Refresh")}</span>
          </Button>
        </div>
      </div>

      <AdminQuickstartChecklist />

      {/* Grid of Configured Active Widgets */}
      <div
        className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4"
        data-testid="dashboard-widgets-grid"
      >
        <HealthCard />
        {activeWidgets.map((w) => renderWidget(w.id))}
      </div>

      {/* Layout Customizer Dialog */}
      <DashboardLayoutDialog
        open={layoutDialog}
        onClose={() => setLayoutDialog(false)}
        widgets={widgets}
        onSave={handleSaveLayout}
        onReset={handleResetLayout}
      />
    </div>
  );
}
