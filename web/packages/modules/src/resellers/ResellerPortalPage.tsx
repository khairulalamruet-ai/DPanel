import React, { useState, useEffect, useCallback } from "react";
import { useTranslation } from "@builtpanel/i18n";
import { apiFetch } from "@builtpanel/api";
import { useToast, Button } from "@builtpanel/ui";
import {
  Users,
  Layers,
  Palette,
  RefreshCw,
  AlertCircle,
  Briefcase,
} from "lucide-react";
import {
  ResellerAccount,
  ResellerPackage,
  ResellerUser,
  ResellerPlan,
  ResellerBranding,
} from "./types";
import { ResellerStatsGrid } from "./ResellerStatsGrid";
import { ResellerUsersTable } from "./ResellerUsersTable";
import { ResellerPlansTable } from "./ResellerPlansTable";
import { ResellerBrandingCard } from "./ResellerBrandingCard";

export function ResellerPortalPage() {
  const { t } = useTranslation();
  const { toast } = useToast();

  const [activeTab, setActiveTab] = useState<"users" | "plans" | "branding">("users");

  const [account, setAccount] = useState<ResellerAccount | null>(null);
  const [pkg, setPkg] = useState<ResellerPackage | undefined>(undefined);
  const [users, setUsers] = useState<ResellerUser[]>([]);
  const [plans, setPlans] = useState<ResellerPlan[]>([]);

  const [isLoading, setIsLoading] = useState(true);
  const [isError, setIsError] = useState(false);

  const loadData = useCallback(async () => {
    try {
      setIsLoading(true);
      setIsError(false);

      const [accRes, pkgRes, usersRes, plansRes] = await Promise.all([
        apiFetch<ResellerAccount>("/api/v1/reseller/me"),
        apiFetch<ResellerPackage>("/api/v1/reseller/me/package"),
        apiFetch<ResellerUser[]>("/api/v1/reseller/users"),
        apiFetch<ResellerPlan[]>("/api/v1/reseller/plans"),
      ]);

      setAccount(accRes);
      setPkg(pkgRes);
      setUsers(usersRes);
      setPlans(plansRes);
    } catch {
      setIsError(true);
    } finally {
      setIsLoading(false);
    }
  }, []);

  useEffect(() => {
    loadData();
  }, [loadData]);

  // Handlers for User management
  const handleCreateUser = async (data: {
    username: string;
    email: string;
    domain: string;
    password: string;
    plan_name: string;
  }) => {
    await apiFetch("/api/v1/reseller/users", {
      method: "POST",
      body: JSON.stringify(data),
    });
    toast({
      title: t("reseller.userCreatedTitle", "Client account created"),
      description: t("reseller.userCreatedDesc", "Account {{username}} has been provisioned.", {
        username: data.username,
      }),
      variant: "success",
    });
    loadData();
  };

  const handleToggleSuspend = async (id: string, currentStatus: "active" | "suspended") => {
    const nextStatus = currentStatus === "active" ? "suspended" : "active";
    await apiFetch(`/api/v1/reseller/users/${id}/status`, {
      method: "POST",
      body: JSON.stringify({ status: nextStatus }),
    });
    toast({
      title: nextStatus === "active" ? t("reseller.userActivated", "Account Activated") : t("reseller.userSuspended", "Account Suspended"),
      variant: "success",
    });
    loadData();
  };

  const handleDeleteUser = async (id: string) => {
    await apiFetch(`/api/v1/reseller/users/${id}`, {
      method: "DELETE",
    });
    toast({
      title: t("reseller.userDeletedTitle", "Account deleted"),
      description: t("reseller.userDeletedDesc", "Client account has been terminated."),
      variant: "success",
    });
    loadData();
  };

  // Handlers for Custom Plans
  const handleCreatePlan = async (data: {
    name: string;
    disk_mb: number;
    bandwidth_mb: number;
    domains_limit: number;
    databases_limit: number;
    mailboxes_limit: number;
  }) => {
    await apiFetch("/api/v1/reseller/plans", {
      method: "POST",
      body: JSON.stringify(data),
    });
    toast({
      title: t("reseller.planCreatedTitle", "Plan created"),
      description: t("reseller.planCreatedDesc", "Hosting plan {{name}} is now ready.", {
        name: data.name,
      }),
      variant: "success",
    });
    loadData();
  };

  const handleDeletePlan = async (id: string) => {
    await apiFetch(`/api/v1/reseller/plans/${id}`, {
      method: "DELETE",
    });
    toast({
      title: t("reseller.planDeletedTitle", "Plan deleted"),
      description: t("reseller.planDeletedDesc", "Custom plan has been removed."),
      variant: "success",
    });
    loadData();
  };

  // Handler for Branding
  const handleSaveBranding = async (branding: ResellerBranding) => {
    await apiFetch("/api/v1/reseller/me/branding", {
      method: "PUT",
      body: JSON.stringify(branding),
    });
    toast({
      title: t("reseller.brandingSavedTitle", "Branding updated"),
      description: t("reseller.brandingSavedDesc", "White-label configuration has been saved."),
      variant: "success",
    });
    if (account) {
      setAccount({ ...account, branding });
    }
  };

  return (
    <div className="space-y-6" data-testid="reseller-portal-page">
      {/* Page Header */}
      <div className="flex flex-col sm:flex-row sm:items-center sm:justify-between gap-4">
        <div>
          <div className="flex items-center gap-2">
            <h1 className="text-2xl font-bold tracking-tight" data-testid="reseller-portal-title">
              {account?.company_name || t("reseller.portalTitle", "Reseller Management Portal")}
            </h1>
            <span className="text-xs font-mono px-2 py-0.5 rounded bg-muted text-muted-foreground border">
              Reseller
            </span>
          </div>
          <p className="text-muted-foreground text-sm">
            {t("reseller.portalSubtitle", "Manage sub-tenants, custom plans, resource quotas, and white-label branding.")}
          </p>
        </div>

        <Button
          variant="outline"
          size="sm"
          onClick={loadData}
          disabled={isLoading}
          className="gap-2 shrink-0"
          data-testid="refresh-reseller-btn"
        >
          <RefreshCw className={`h-4 w-4 ${isLoading ? "animate-spin" : ""}`} />
          {t("common.refresh", "Refresh")}
        </Button>
      </div>

      {isError && (
        <div className="p-4 rounded-lg bg-red-50 text-red-800 dark:bg-red-950/40 dark:text-red-300 border border-red-200 flex items-center gap-3">
          <AlertCircle className="h-5 w-5 shrink-0" />
          <div className="flex-1 text-sm">
            {t("reseller.loadError", "Failed to load reseller account details.")}
          </div>
          <Button variant="outline" size="sm" onClick={loadData}>
            {t("common.retry", "Retry")}
          </Button>
        </div>
      )}

      {/* Quotas and Stats */}
      {account && <ResellerStatsGrid account={account} pkg={pkg} />}

      {/* Navigation Tabs */}
      <div className="border-b flex gap-4">
        <button
          onClick={() => setActiveTab("users")}
          data-testid="tab-reseller-users"
          className={`pb-2.5 px-1 text-sm font-medium border-b-2 transition-colors flex items-center gap-2 ${
            activeTab === "users"
              ? "border-primary text-primary"
              : "border-transparent text-muted-foreground hover:text-foreground"
          }`}
        >
          <Users className="h-4 w-4" />
          {t("reseller.tabUsers", "Client Accounts")}
          <span className="ml-1 rounded-full bg-muted px-2 py-0.5 text-xs font-mono">
            {users.length}
          </span>
        </button>

        <button
          onClick={() => setActiveTab("plans")}
          data-testid="tab-reseller-plans"
          className={`pb-2.5 px-1 text-sm font-medium border-b-2 transition-colors flex items-center gap-2 ${
            activeTab === "plans"
              ? "border-primary text-primary"
              : "border-transparent text-muted-foreground hover:text-foreground"
          }`}
        >
          <Layers className="h-4 w-4" />
          {t("reseller.tabPlans", "Custom Plans")}
          <span className="ml-1 rounded-full bg-muted px-2 py-0.5 text-xs font-mono">
            {plans.length}
          </span>
        </button>

        <button
          onClick={() => setActiveTab("branding")}
          data-testid="tab-reseller-branding"
          className={`pb-2.5 px-1 text-sm font-medium border-b-2 transition-colors flex items-center gap-2 ${
            activeTab === "branding"
              ? "border-primary text-primary"
              : "border-transparent text-muted-foreground hover:text-foreground"
          }`}
        >
          <Palette className="h-4 w-4" />
          {t("reseller.tabBranding", "White-Label Branding")}
        </button>
      </div>

      {/* Tab Panels */}
      <div className="pt-2">
        {activeTab === "users" && (
          <ResellerUsersTable
            users={users}
            plans={plans}
            loading={isLoading}
            onCreateUser={handleCreateUser}
            onToggleSuspend={handleToggleSuspend}
            onDeleteUser={handleDeleteUser}
          />
        )}

        {activeTab === "plans" && (
          <ResellerPlansTable
            plans={plans}
            loading={isLoading}
            onCreatePlan={handleCreatePlan}
            onDeletePlan={handleDeletePlan}
          />
        )}

        {activeTab === "branding" && account && (
          <ResellerBrandingCard
            branding={account.branding}
            onSaveBranding={handleSaveBranding}
          />
        )}
      </div>
    </div>
  );
}
