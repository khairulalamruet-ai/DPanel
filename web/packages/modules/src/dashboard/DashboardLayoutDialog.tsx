import React, { useState, useEffect } from "react";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
  Button,
} from "@builtpanel/ui";
import { useTranslation } from "@builtpanel/i18n";
import {
  LayoutGrid,
  ArrowUp,
  ArrowDown,
  RotateCcw,
  Check,
  Server,
  Users,
  Activity,
  History,
  ShieldAlert,
  KeyRound,
} from "lucide-react";
import { DashboardWidgetItem, DashboardWidgetId } from "./types";

interface DashboardLayoutDialogProps {
  open: boolean;
  onClose: () => void;
  widgets: DashboardWidgetItem[];
  onSave: (widgets: DashboardWidgetItem[]) => void;
  onReset: () => void;
}

export function DashboardLayoutDialog({
  open,
  onClose,
  widgets: initialWidgets,
  onSave,
  onReset,
}: DashboardLayoutDialogProps) {
  const { t } = useTranslation();
  const [items, setItems] = useState<DashboardWidgetItem[]>(initialWidgets);

  useEffect(() => {
    setItems(initialWidgets);
  }, [initialWidgets, open]);

  const getWidgetLabel = (id: DashboardWidgetId) => {
    switch (id) {
      case "nodes_status":
        return {
          title: t("dashboard.widgetNodes", "Cluster Nodes Status"),
          desc: t("dashboard.widgetNodesDesc", "Online vs total nodes and cluster health."),
          icon: <Server className="h-4 w-4 text-primary shrink-0" />,
        };
      case "tenants_count":
        return {
          title: t("dashboard.widgetTenants", "Tenants & Hosted Domains"),
          desc: t("dashboard.widgetTenantsDesc", "Active tenant accounts, domains, and MySQL databases."),
          icon: <Users className="h-4 w-4 text-emerald-500 shrink-0" />,
        };
      case "resource_usage":
        return {
          title: t("dashboard.widgetResources", "Resource Consumption (CPU / RAM / Disk)"),
          desc: t("dashboard.widgetResourcesDesc", "Real-time hardware utilization meters."),
          icon: <Activity className="h-4 w-4 text-primary shrink-0" />,
        };
      case "recent_operations":
        return {
          title: t("dashboard.widgetOperations", "Recent Operations Ledger"),
          desc: t("dashboard.widgetOperationsDesc", "Live async job statuses, durations, and logs."),
          icon: <History className="h-4 w-4 text-blue-500 shrink-0" />,
        };
      case "security_alerts":
        return {
          title: t("dashboard.widgetSecurity", "Security & Threat Alerts"),
          desc: t("dashboard.widgetSecurityDesc", "Coraza WAF blocks, Fail2ban jails, and login anomalies."),
          icon: <ShieldAlert className="h-4 w-4 text-amber-500 shrink-0" />,
        };
      case "license_status":
        return {
          title: t("dashboard.widgetLicense", "License Authority Status"),
          desc: t("dashboard.widgetLicenseDesc", "Ed25519 signature validity and capacity quotas."),
          icon: <KeyRound className="h-4 w-4 text-purple-500 shrink-0" />,
        };
    }
  };

  const handleToggle = (id: DashboardWidgetId) => {
    setItems((prev) =>
      prev.map((item) => (item.id === id ? { ...item, enabled: !item.enabled } : item))
    );
  };

  const handleMove = (index: number, direction: "up" | "down") => {
    const targetIndex = direction === "up" ? index - 1 : index + 1;
    if (targetIndex < 0 || targetIndex >= items.length) return;

    const copy = [...items];
    const [moved] = copy.splice(index, 1);
    copy.splice(targetIndex, 0, moved);

    // Reassign order indices
    const updated = copy.map((item, idx) => ({ ...item, order: idx }));
    setItems(updated);
  };

  const handleSave = () => {
    onSave(items);
    onClose();
  };

  const handleReset = () => {
    onReset();
    onClose();
  };

  return (
    <Dialog open={open} onOpenChange={(val) => !val && onClose()}>
      <DialogContent data-testid="dashboard-layout-dialog" className="sm:max-w-md max-h-[90vh] flex flex-col">
        <DialogHeader className="pb-2 border-b">
          <div className="flex items-center gap-2">
            <div className="p-2 rounded-full bg-primary/10 text-primary">
              <LayoutGrid className="h-5 w-5" />
            </div>
            <div>
              <DialogTitle data-testid="dashboard-layout-dialog-title">
                {t("dashboard.customizeLayoutTitle", "Customize Dashboard Layout")}
              </DialogTitle>
              <DialogDescription className="text-xs">
                {t(
                  "dashboard.customizeLayoutDesc",
                  "Reorder and toggle visibility of widgets. Your layout is automatically persisted."
                )}
              </DialogDescription>
            </div>
          </div>
        </DialogHeader>

        <div className="py-2 space-y-2 flex-1 overflow-y-auto" data-testid="layout-widget-list">
          {items.map((item, idx) => {
            const meta = getWidgetLabel(item.id);
            return (
              <div
                key={item.id}
                className={`p-3 rounded-lg border transition-colors flex items-center justify-between gap-3 ${
                  item.enabled ? "bg-card" : "bg-muted/40 opacity-60"
                }`}
                data-testid={`layout-widget-item-${item.id}`}
              >
                <div className="flex items-center gap-3 min-w-0">
                  <input
                    type="checkbox"
                    checked={item.enabled}
                    onChange={() => handleToggle(item.id)}
                    className="rounded border-gray-300 text-primary focus:ring-primary h-4 w-4 shrink-0"
                    data-testid={`checkbox-widget-${item.id}`}
                  />
                  {meta.icon}
                  <div className="min-w-0">
                    <span className="text-xs font-bold block truncate">{meta.title}</span>
                    <span className="text-[11px] text-muted-foreground block truncate">{meta.desc}</span>
                  </div>
                </div>

                <div className="flex items-center gap-1 shrink-0">
                  <button
                    type="button"
                    onClick={() => handleMove(idx, "up")}
                    disabled={idx === 0}
                    className="p-1 text-muted-foreground hover:text-foreground disabled:opacity-30 rounded hover:bg-muted"
                    title="Move up"
                    data-testid={`btn-move-up-${item.id}`}
                  >
                    <ArrowUp className="h-3.5 w-3.5" />
                  </button>
                  <button
                    type="button"
                    onClick={() => handleMove(idx, "down")}
                    disabled={idx === items.length - 1}
                    className="p-1 text-muted-foreground hover:text-foreground disabled:opacity-30 rounded hover:bg-muted"
                    title="Move down"
                    data-testid={`btn-move-down-${item.id}`}
                  >
                    <ArrowDown className="h-3.5 w-3.5" />
                  </button>
                </div>
              </div>
            );
          })}
        </div>

        <DialogFooter className="pt-3 border-t flex items-center justify-between sm:justify-between w-full">
          <Button
            type="button"
            variant="ghost"
            size="sm"
            onClick={handleReset}
            className="text-xs gap-1.5 text-muted-foreground hover:text-foreground"
            data-testid="btn-reset-dashboard-layout"
          >
            <RotateCcw className="h-3.5 w-3.5" />
            {t("common.reset", "Reset Default")}
          </Button>

          <div className="flex items-center gap-2">
            <Button type="button" variant="outline" size="sm" onClick={onClose}>
              {t("common.cancel", "Cancel")}
            </Button>
            <Button type="button" size="sm" onClick={handleSave} data-testid="btn-save-dashboard-layout">
              {t("common.save", "Apply Layout")}
            </Button>
          </div>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
