import React, { useState, useEffect } from "react";
import {
  Card,
  CardHeader,
  CardTitle,
  CardDescription,
  CardContent,
  Table,
  TableHeader,
  TableBody,
  TableHead,
  TableRow,
  TableCell,
  Button,
  Badge,
  useToast,
} from "@builtpanel/ui";
import { useTranslation } from "@builtpanel/i18n";
import { apiFetch } from "@builtpanel/api";
import {
  Server,
  Plus,
  Edit2,
  Trash2,
  RefreshCw,
  Star,
  Users,
  Cpu,
  Layers,
  Network,
  MapPin,
} from "lucide-react";
import { Plan, CreatePlanPayload } from "./types";
import { PlanEditorDialog } from "./PlanEditorDialog";

export function PlansPage() {
  const { t } = useTranslation();
  const { toast } = useToast();

  const [plans, setPlans] = useState<Plan[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  // Editor Dialog State
  const [isEditorOpen, setIsEditorOpen] = useState(false);
  const [editingPlan, setEditingPlan] = useState<Plan | null>(null);
  const [isSaving, setIsSaving] = useState(false);

  // Delete State
  const [deletingId, setDeletingId] = useState<string | null>(null);

  const fetchPlans = async () => {
    try {
      setIsLoading(true);
      setError(null);
      const res = await apiFetch<{ items: Plan[]; total: number }>(
        "/api/v1/admin/plans"
      );
      setPlans(res.items || []);
    } catch (err: any) {
      setError(err?.message || "Failed to load hosting plans");
    } finally {
      setIsLoading(false);
    }
  };

  useEffect(() => {
    fetchPlans();
  }, []);

  const handleOpenCreate = () => {
    setEditingPlan(null);
    setIsEditorOpen(true);
  };

  const handleOpenEdit = (plan: Plan) => {
    setEditingPlan(plan);
    setIsEditorOpen(true);
  };

  const handleSave = async (payload: CreatePlanPayload) => {
    setIsSaving(true);
    try {
      if (editingPlan) {
        // Edit existing
        const res = await apiFetch<Plan>(`/api/v1/admin/plans/${editingPlan.id}`, {
          method: "PUT",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify(payload),
        });
        toast({
          title: t("common.appName"),
          description: t("plans.toast.updateSuccess", { name: res.name }),
          variant: "default",
        });
        setPlans((prev) =>
          prev.map((item) => {
            if (payload.is_default && item.id !== res.id) {
              return { ...item, is_default: false };
            }
            return item.id === res.id ? res : item;
          })
        );
      } else {
        // Create new
        const res = await apiFetch<Plan>("/api/v1/admin/plans", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify(payload),
        });
        toast({
          title: t("common.appName"),
          description: t("plans.toast.createSuccess", { name: res.name }),
          variant: "default",
        });
        setPlans((prev) => {
          const list = payload.is_default
            ? prev.map((item) => ({ ...item, is_default: false }))
            : [...prev];
          return [res, ...list];
        });
      }
      setIsEditorOpen(false);
    } catch (err: any) {
      toast({
        title: t("common.error"),
        description: err?.message || t("plans.toast.error"),
        variant: "destructive",
      });
      throw err;
    } finally {
      setIsSaving(false);
    }
  };

  const handleDelete = async (plan: Plan) => {
    if (plan.is_default) {
      toast({
        title: t("common.error"),
        description: t("plans.deleteConfirm.cannotDeleteDefault"),
        variant: "destructive",
      });
      return;
    }

    if (plan.users_count > 0) {
      toast({
        title: t("common.error"),
        description: t("plans.deleteConfirm.cannotDeleteWithUsers"),
        variant: "destructive",
      });
      return;
    }

    if (!window.confirm(t("plans.deleteConfirm.desc", { name: plan.name }))) {
      return;
    }

    try {
      setDeletingId(plan.id);
      await apiFetch(`/api/v1/admin/plans/${plan.id}`, {
        method: "DELETE",
      });
      toast({
        title: t("common.appName"),
        description: t("plans.toast.deleteSuccess", { name: plan.name }),
        variant: "default",
      });
      setPlans((prev) => prev.filter((p) => p.id !== plan.id));
    } catch (err: any) {
      toast({
        title: t("common.error"),
        description: err?.message || "Failed to delete plan",
        variant: "destructive",
      });
    } finally {
      setDeletingId(null);
    }
  };

  const unlimitedText = t("plans.unlimited");

  const formatResource = (val: number, unit: string) => {
    return val === -1 ? `∞ ${unlimitedText}` : `${val} ${unit}`;
  };

  return (
    <div className="space-y-6">
      {/* Page Header */}
      <div className="flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
        <div>
          <div className="flex items-center gap-2">
            <Server className="h-6 w-6 text-primary" />
            <h1 className="text-2xl font-bold tracking-tight">{t("plans.title")}</h1>
          </div>
          <p className="text-sm text-muted-foreground mt-1">{t("plans.desc")}</p>
        </div>
        <div className="flex items-center gap-2">
          <Button
            variant="outline"
            size="sm"
            onClick={fetchPlans}
            disabled={isLoading}
            className="gap-2"
          >
            <RefreshCw className={`h-4 w-4 ${isLoading ? "animate-spin" : ""}`} />
            {t("common.reload")}
          </Button>
          <Button onClick={handleOpenCreate} className="gap-2">
            <Plus className="h-4 w-4" />
            {t("plans.createBtn")}
          </Button>
        </div>
      </div>

      {/* Main Card */}
      <Card>
        <CardContent className="p-0">
          {isLoading ? (
            <div className="py-16 text-center text-sm text-muted-foreground">
              <RefreshCw className="h-6 w-6 animate-spin mx-auto mb-2 text-primary" />
              {t("common.loading")}
            </div>
          ) : error ? (
            <div className="py-12 text-center space-y-3">
              <p className="text-sm text-destructive">{error}</p>
              <Button variant="outline" size="sm" onClick={fetchPlans}>
                {t("common.retry")}
              </Button>
            </div>
          ) : plans.length === 0 ? (
            <div className="py-16 text-center text-sm text-muted-foreground">
              {t("plans.empty")}
            </div>
          ) : (
            <div className="overflow-x-auto">
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead className="w-[22%]">{t("plans.table.name")}</TableHead>
                    <TableHead className="w-[18%]">{t("plans.table.nodeGroup")}</TableHead>
                    <TableHead className="w-[22%]">{t("plans.table.resources")}</TableHead>
                    <TableHead className="w-[18%]">{t("plans.table.limits")}</TableHead>
                    <TableHead className="w-[10%]">{t("plans.table.users")}</TableHead>
                    <TableHead className="w-[10%] text-right">
                      {t("plans.table.actions")}
                    </TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {plans.map((plan) => (
                    <TableRow key={plan.id} className="hover:bg-muted/50 transition-colors">
                      {/* Name & Description */}
                      <TableCell>
                        <div className="flex flex-col">
                          <div className="flex items-center gap-2">
                            <span className="font-semibold text-foreground text-sm">
                              {plan.name}
                            </span>
                            {plan.is_default && (
                              <Badge
                                variant="secondary"
                                className="text-[10px] px-1.5 py-0 font-medium bg-amber-500/10 text-amber-600 border border-amber-500/20 flex items-center gap-1"
                              >
                                <Star className="h-2.5 w-2.5 fill-amber-500" />
                                {t("plans.defaultBadge")}
                              </Badge>
                            )}
                          </div>
                          {plan.description && (
                            <p className="text-xs text-muted-foreground mt-0.5 line-clamp-1">
                              {plan.description}
                            </p>
                          )}
                          <div className="flex items-center gap-1 text-[11px] text-muted-foreground mt-1">
                            <Layers className="h-3 w-3" />
                            <span>{plan.feature_set_name}</span>
                          </div>
                        </div>
                      </TableCell>

                      {/* Node Group & Placement */}
                      <TableCell>
                        <div className="flex flex-col gap-1 items-start">
                          <Badge
                            variant="outline"
                            data-testid="plan-node-group-badge"
                            className="text-xs font-medium gap-1 py-0.5 bg-background"
                          >
                            <Network className="h-3 w-3 text-primary shrink-0" />
                            <span>{plan.node_group_name || t("plans.globalCluster")}</span>
                          </Badge>
                          {plan.target_region && (
                            <div className="flex items-center gap-1 text-[11px] text-muted-foreground font-mono">
                              <MapPin className="h-2.5 w-2.5" />
                              <span>{plan.target_region}</span>
                            </div>
                          )}
                        </div>
                      </TableCell>

                      {/* Resources */}
                      <TableCell>
                        <div className="flex flex-col gap-1 text-xs text-muted-foreground">
                          <div className="flex items-center gap-1.5">
                            <Cpu className="h-3 w-3 text-primary shrink-0" />
                            <span>{formatResource(plan.resources.cpu, "vCPU")}</span>
                            <span>•</span>
                            <span>{formatResource(plan.resources.memory_mb, "MB RAM")}</span>
                          </div>
                          <div className="text-[11px]">
                            <span>Disk: {formatResource(plan.resources.disk_mb, "MB")}</span>
                            <span> • Bandwidth: {formatResource(plan.resources.bandwidth_mb, "MB")}</span>
                          </div>
                        </div>
                      </TableCell>

                      {/* Limits */}
                      <TableCell>
                        <div className="flex flex-col gap-0.5 text-xs text-muted-foreground">
                          <div>
                            <span className="font-medium text-foreground">
                              {plan.limits.max_domains === -1 ? `∞ ${unlimitedText}` : plan.limits.max_domains}
                            </span>{" "}
                            Domains
                            <span> • </span>
                            <span className="font-medium text-foreground">
                              {plan.limits.max_databases === -1 ? `∞ ${unlimitedText}` : plan.limits.max_databases}
                            </span>{" "}
                            DBs
                          </div>
                          <div className="text-[11px]">
                            {plan.limits.max_email_accounts === -1 ? `∞ ${unlimitedText}` : plan.limits.max_email_accounts} Emails
                          </div>
                        </div>
                      </TableCell>

                      {/* Users Count (DoD requirement: কতজন user এই plan-এ আছে দেখানো) */}
                      <TableCell>
                        <Badge variant="outline" className="text-xs font-normal gap-1.5 py-0.5">
                          <Users className="h-3 w-3 text-muted-foreground" />
                          <span>{t("plans.usersAssigned", { count: plan.users_count })}</span>
                        </Badge>
                      </TableCell>

                      {/* Actions */}
                      <TableCell className="text-right">
                        <div className="flex items-center justify-end gap-1.5">
                          <Button
                            variant="outline"
                            size="sm"
                            onClick={() => handleOpenEdit(plan)}
                            className="h-8 px-2.5 text-xs gap-1"
                          >
                            <Edit2 className="h-3.5 w-3.5" />
                            {t("plans.editBtn")}
                          </Button>
                          <Button
                            variant="ghost"
                            size="sm"
                            disabled={plan.is_default || plan.users_count > 0 || deletingId === plan.id}
                            onClick={() => handleDelete(plan)}
                            title={
                              plan.is_default
                                ? t("plans.deleteConfirm.cannotDeleteDefault")
                                : plan.users_count > 0
                                ? t("plans.deleteConfirm.cannotDeleteWithUsers")
                                : t("plans.deleteBtn")
                            }
                            className="h-8 px-2.5 text-xs text-destructive hover:bg-destructive/10 hover:text-destructive disabled:opacity-40"
                          >
                            <Trash2 className="h-3.5 w-3.5" />
                            {t("plans.deleteBtn")}
                          </Button>
                        </div>
                      </TableCell>
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
            </div>
          )}
        </CardContent>
      </Card>

      {/* Plan Editor Dialog */}
      <PlanEditorDialog
        open={isEditorOpen}
        onOpenChange={setIsEditorOpen}
        plan={editingPlan}
        onSave={handleSave}
        isSaving={isSaving}
      />
    </div>
  );
}
