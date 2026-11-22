import React, { useState, useEffect, useCallback } from "react";
import { useTranslation } from "@builtpanel/i18n";
import { apiFetch } from "@builtpanel/api";
import {
  Table,
  TableHeader,
  TableBody,
  TableRow,
  TableHead,
  TableCell,
  Button,
  Badge,
  useToast,
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
} from "@builtpanel/ui";
import {
  Briefcase,
  Package,
  Plus,
  RefreshCw,
  PauseCircle,
  PlayCircle,
  Trash2,
  AlertTriangle,
  ShieldCheck,
  AlertCircle,
} from "lucide-react";
import { ResellerAccount, ResellerPackage } from "./types";
import { CreateResellerDialog } from "./CreateResellerDialog";
import { CreateResellerPackageDialog } from "./CreateResellerPackageDialog";
import { AccountsTabs } from "../accounts/AccountsTabs";

export function AdminResellersPage() {
  const { t } = useTranslation();
  const { toast } = useToast();

  const [activeTab, setActiveTab] = useState<"resellers" | "packages">("resellers");
  const [resellers, setResellers] = useState<ResellerAccount[]>([]);
  const [packages, setPackages] = useState<ResellerPackage[]>([]);

  const [loading, setLoading] = useState(true);
  const [createResellerOpen, setCreateResellerOpen] = useState(false);
  const [createPkgOpen, setCreatePkgOpen] = useState(false);
  const [deleteTarget, setDeleteTarget] = useState<ResellerAccount | null>(null);
  const [isDeleting, setIsDeleting] = useState(false);

  const loadData = useCallback(async () => {
    try {
      setLoading(true);
      const [rRes, pRes] = await Promise.all([
        apiFetch<ResellerAccount[]>("/api/v1/admin/resellers"),
        apiFetch<ResellerPackage[]>("/api/v1/admin/reseller-packages"),
      ]);
      setResellers(rRes);
      setPackages(pRes);
    } catch {
      toast({
        title: t("common.error", "Error"),
        description: t("reseller.loadError", "Failed to load reseller accounts"),
        variant: "destructive",
      });
    } finally {
      setLoading(false);
    }
  }, [t, toast]);

  useEffect(() => {
    loadData();
  }, [loadData]);

  // Handlers for Resellers
  const handleCreateReseller = async (data: {
    username: string;
    email: string;
    company_name: string;
    password: string;
    package_id: string;
  }) => {
    await apiFetch("/api/v1/admin/resellers", {
      method: "POST",
      body: JSON.stringify(data),
    });
    toast({
      title: t("reseller.createdTitle", "Reseller Provisioned"),
      description: t("reseller.createdDesc", "Reseller {{username}} has been created.", {
        username: data.username,
      }),
      variant: "success",
    });
    loadData();
  };

  const handleToggleSuspend = async (id: string, currentStatus: "active" | "suspended") => {
    const nextStatus = currentStatus === "active" ? "suspended" : "active";
    await apiFetch(`/api/v1/admin/resellers/${id}/status`, {
      method: "POST",
      body: JSON.stringify({ status: nextStatus }),
    });
    toast({
      title:
        nextStatus === "active"
          ? t("reseller.activatedTitle", "Reseller Activated")
          : t("reseller.suspendedTitle", "Reseller Suspended (T14.07)"),
      description:
        nextStatus === "suspended"
          ? t("reseller.suspendedNotice", "Reseller and all associated sub-tenant logins are now locked.")
          : undefined,
      variant: "success",
    });
    loadData();
  };

  const handleDeleteReseller = async () => {
    if (!deleteTarget) return;
    setIsDeleting(true);
    try {
      await apiFetch(`/api/v1/admin/resellers/${deleteTarget.id}`, { method: "DELETE" });
      toast({
        title: t("reseller.deletedTitle", "Reseller Terminated"),
        variant: "success",
      });
      setDeleteTarget(null);
      loadData();
    } finally {
      setIsDeleting(false);
    }
  };

  // Handlers for Packages (T14.02)
  const handleCreatePackage = async (data: {
    name: string;
    max_accounts: number;
    max_disk_gb: number;
    max_bandwidth_gb: number;
    allow_overselling: boolean;
  }) => {
    await apiFetch("/api/v1/admin/reseller-packages", {
      method: "POST",
      body: JSON.stringify(data),
    });
    toast({
      title: t("reseller.pkgCreatedTitle", "Reseller Package Created"),
      variant: "success",
    });
    loadData();
  };

  const handleDeletePackage = async (id: string) => {
    await apiFetch(`/api/v1/admin/reseller-packages/${id}`, { method: "DELETE" });
    toast({
      title: t("reseller.pkgDeletedTitle", "Package Deleted"),
      variant: "success",
    });
    loadData();
  };

  return (
    <div className="space-y-6" data-testid="admin-resellers-page">
      <AccountsTabs active="resellers" />
      {/* Header */}
      <div className="flex flex-col sm:flex-row sm:items-center sm:justify-between gap-4">
        <div>
          <h1 className="text-2xl font-bold tracking-tight" data-testid="admin-resellers-title">
            {t("reseller.adminPageTitle", "Resellers & Partner Accounts")}
          </h1>
          <p className="text-muted-foreground text-sm">
            {t("reseller.adminPageSubtitle", "Manage multi-tier reseller packages, resource allocations, and sub-accounts.")}
          </p>
        </div>

        <div className="flex items-center gap-2">
          {activeTab === "resellers" ? (
            <Button
              onClick={() => setCreateResellerOpen(true)}
              className="gap-2 shrink-0"
              data-testid="btn-open-create-reseller"
            >
              <Plus className="h-4 w-4" />
              {t("reseller.createResellerBtn", "Provision Reseller")}
            </Button>
          ) : (
            <Button
              onClick={() => setCreatePkgOpen(true)}
              className="gap-2 shrink-0"
              data-testid="btn-open-create-pkg"
            >
              <Plus className="h-4 w-4" />
              {t("reseller.createPackageBtn", "Create Package")}
            </Button>
          )}

          <Button
            variant="outline"
            size="sm"
            onClick={loadData}
            disabled={loading}
            className="gap-2 shrink-0"
            data-testid="refresh-admin-resellers-btn"
          >
            <RefreshCw className={`h-4 w-4 ${loading ? "animate-spin" : ""}`} />
            {t("common.refresh", "Refresh")}
          </Button>
        </div>
      </div>

      {/* Tabs */}
      <div className="border-b flex gap-4">
        <button
          onClick={() => setActiveTab("resellers")}
          data-testid="tab-admin-resellers"
          className={`pb-2.5 px-1 text-sm font-medium border-b-2 transition-colors flex items-center gap-2 ${
            activeTab === "resellers"
              ? "border-primary text-primary"
              : "border-transparent text-muted-foreground hover:text-foreground"
          }`}
        >
          <Briefcase className="h-4 w-4" />
          {t("reseller.adminTabResellers", "Reseller Accounts")}
          <span className="ml-1 rounded-full bg-muted px-2 py-0.5 text-xs font-mono">
            {resellers.length}
          </span>
        </button>

        <button
          onClick={() => setActiveTab("packages")}
          data-testid="tab-admin-packages"
          className={`pb-2.5 px-1 text-sm font-medium border-b-2 transition-colors flex items-center gap-2 ${
            activeTab === "packages"
              ? "border-primary text-primary"
              : "border-transparent text-muted-foreground hover:text-foreground"
          }`}
        >
          <Package className="h-4 w-4" />
          {t("reseller.adminTabPackages", "Reseller Packages (T14.02)")}
          <span className="ml-1 rounded-full bg-muted px-2 py-0.5 text-xs font-mono">
            {packages.length}
          </span>
        </button>
      </div>

      {/* Tab Panels */}
      {activeTab === "resellers" && (
        <div className="rounded-lg border bg-card overflow-hidden" data-testid="resellers-table-container">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>{t("reseller.partnerCol", "Partner / Company")}</TableHead>
                <TableHead>{t("reseller.packageCol", "Package Tier")}</TableHead>
                <TableHead>{t("reseller.accountsUsageCol", "Accounts")}</TableHead>
                <TableHead>{t("reseller.diskUsageCol", "Disk Used")}</TableHead>
                <TableHead>{t("common.status", "Status")}</TableHead>
                <TableHead className="text-right">{t("common.actions", "Actions")}</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {loading ? (
                <TableRow>
                  <TableCell colSpan={6} className="text-center py-8 text-muted-foreground">
                    {t("common.loading", "Loading reseller accounts...")}
                  </TableCell>
                </TableRow>
              ) : resellers.length === 0 ? (
                <TableRow>
                  <TableCell colSpan={6} className="text-center py-8 text-muted-foreground">
                    {t("reseller.noResellers", "No reseller accounts provisioned yet.")}
                  </TableCell>
                </TableRow>
              ) : (
                resellers.map((r) => (
                  <TableRow key={r.id} data-testid={`admin-reseller-row-${r.id}`}>
                    <TableCell>
                      <div className="flex items-center gap-2.5">
                        <div
                          className="h-3 w-3 rounded-full shrink-0"
                          style={{ backgroundColor: r.branding?.primary_color || "#2563eb" }}
                          title="Brand Color"
                        />
                        <div>
                          <span className="font-bold text-sm block" data-testid={`reseller-company-${r.id}`}>
                            {r.company_name}
                          </span>
                          <span className="text-[11px] font-mono text-muted-foreground">
                            @{r.username} • {r.email}
                          </span>
                        </div>
                      </div>
                    </TableCell>

                    <TableCell className="text-xs font-semibold">
                      <Badge variant="outline">{r.package_name}</Badge>
                    </TableCell>

                    <TableCell className="text-xs font-mono">
                      {r.accounts_count} accounts
                    </TableCell>

                    <TableCell className="text-xs font-mono">
                      {r.disk_used_gb} GB
                    </TableCell>

                    <TableCell>
                      <Badge variant={r.status === "active" ? "success" : "destructive"}>
                        {r.status === "active" ? t("common.active", "Active") : t("common.suspended", "Suspended")}
                      </Badge>
                    </TableCell>

                    <TableCell className="text-right">
                      <div className="flex items-center justify-end gap-1.5">
                        <Button
                          variant="ghost"
                          size="sm"
                          onClick={() => handleToggleSuspend(r.id, r.status)}
                          data-testid={`btn-suspend-reseller-${r.id}`}
                          className="h-8 text-xs gap-1"
                        >
                          {r.status === "active" ? (
                            <>
                              <PauseCircle className="h-3.5 w-3.5 text-amber-500" />
                              {t("common.suspend", "Suspend")}
                            </>
                          ) : (
                            <>
                              <PlayCircle className="h-3.5 w-3.5 text-emerald-500" />
                              {t("common.unsuspend", "Activate")}
                            </>
                          )}
                        </Button>

                        <Button
                          variant="ghost"
                          size="sm"
                          onClick={() => setDeleteTarget(r)}
                          data-testid={`btn-delete-reseller-${r.id}`}
                          className="h-8 w-8 p-0 text-destructive hover:bg-destructive/10"
                        >
                          <Trash2 className="h-4 w-4" />
                        </Button>
                      </div>
                    </TableCell>
                  </TableRow>
                ))
              )}
            </TableBody>
          </Table>
        </div>
      )}

      {activeTab === "packages" && (
        <div className="rounded-lg border bg-card overflow-hidden" data-testid="packages-table-container">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>{t("reseller.packageNameCol", "Package Name")}</TableHead>
                <TableHead>{t("reseller.maxAccountsCol", "Max Accounts")}</TableHead>
                <TableHead>{t("reseller.maxDiskCol", "Disk Limit")}</TableHead>
                <TableHead>{t("reseller.maxBwCol", "Bandwidth Limit")}</TableHead>
                <TableHead>{t("reseller.oversellingCol", "Overselling")}</TableHead>
                <TableHead className="text-right">{t("common.actions", "Actions")}</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {loading ? (
                <TableRow>
                  <TableCell colSpan={6} className="text-center py-8 text-muted-foreground">
                    {t("common.loading", "Loading packages...")}
                  </TableCell>
                </TableRow>
              ) : packages.length === 0 ? (
                <TableRow>
                  <TableCell colSpan={6} className="text-center py-8 text-muted-foreground">
                    {t("reseller.noPackages", "No reseller packages found.")}
                  </TableCell>
                </TableRow>
              ) : (
                packages.map((pkg) => (
                  <TableRow key={pkg.id} data-testid={`reseller-pkg-row-${pkg.id}`}>
                    <TableCell className="font-semibold text-sm">
                      <div className="flex items-center gap-2">
                        <Package className="h-4 w-4 text-primary shrink-0" />
                        <span>{pkg.name}</span>
                      </div>
                    </TableCell>
                    <TableCell className="font-mono text-xs">
                      {pkg.max_accounts} accounts
                    </TableCell>
                    <TableCell className="font-mono text-xs">
                      {pkg.max_disk_gb} GB
                    </TableCell>
                    <TableCell className="font-mono text-xs">
                      {pkg.max_bandwidth_gb} GB
                    </TableCell>
                    <TableCell>
                      {pkg.allow_overselling ? (
                        <Badge variant="success" className="gap-1">
                          <ShieldCheck className="h-3 w-3" />
                          {t("common.allowed", "Allowed")}
                        </Badge>
                      ) : (
                        <Badge variant="outline" className="gap-1 text-muted-foreground">
                          <AlertCircle className="h-3 w-3" />
                          {t("common.restricted", "Restricted")}
                        </Badge>
                      )}
                    </TableCell>
                    <TableCell className="text-right">
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={() => handleDeletePackage(pkg.id)}
                        data-testid={`btn-delete-pkg-${pkg.id}`}
                        className="h-8 w-8 p-0 text-destructive hover:bg-destructive/10"
                      >
                        <Trash2 className="h-4 w-4" />
                      </Button>
                    </TableCell>
                  </TableRow>
                ))
              )}
            </TableBody>
          </Table>
        </div>
      )}

      {/* Dialogs */}
      <CreateResellerDialog
        open={createResellerOpen}
        packages={packages}
        onClose={() => setCreateResellerOpen(false)}
        onCreate={handleCreateReseller}
      />

      <CreateResellerPackageDialog
        open={createPkgOpen}
        onClose={() => setCreatePkgOpen(false)}
        onCreate={handleCreatePackage}
      />

      {/* Delete Confirmation */}
      {deleteTarget && (
        <Dialog open={Boolean(deleteTarget)} onOpenChange={(val) => !val && setDeleteTarget(null)}>
          <DialogContent className="sm:max-w-md">
            <DialogHeader>
              <div className="flex items-center gap-2 text-destructive">
                <AlertTriangle className="h-5 w-5" />
                <DialogTitle>{t("reseller.deleteResellerTitle", "Terminate Reseller Account")}</DialogTitle>
              </div>
              <DialogDescription className="text-xs">
                {t(
                  "reseller.deleteResellerDesc",
                  "Are you sure you want to terminate reseller {{name}}? All associated sub-tenant accounts and websites will be removed.",
                  { name: deleteTarget.company_name }
                )}
              </DialogDescription>
            </DialogHeader>
            <DialogFooter className="pt-2">
              <Button variant="outline" onClick={() => setDeleteTarget(null)} disabled={isDeleting}>
                {t("common.cancel", "Cancel")}
              </Button>
              <Button
                variant="destructive"
                onClick={handleDeleteReseller}
                disabled={isDeleting}
                data-testid="btn-confirm-delete-reseller"
              >
                {isDeleting ? t("common.deleting", "Terminating...") : t("common.delete", "Terminate Reseller")}
              </Button>
            </DialogFooter>
          </DialogContent>
        </Dialog>
      )}
    </div>
  );
}
