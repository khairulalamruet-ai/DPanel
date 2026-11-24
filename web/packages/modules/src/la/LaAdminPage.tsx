import React, { useState, useEffect } from "react";
import {
  Button,
  Card,
  CardHeader,
  CardTitle,
  CardContent,
  Badge,
  Table,
  TableHeader,
  TableBody,
  TableHead,
  TableRow,
  TableCell,
  useToast,
} from "@builtpanel/ui";
import { useTranslation, useFormatters } from "@builtpanel/i18n";
import { apiFetch } from "@builtpanel/api";
import { useSession } from "@builtpanel/auth";
import {
  KeyRound,
  ShieldCheck,
  ShieldAlert,
  Users,
  Server,
  Plus,
  RotateCcw,
  AlertOctagon,
  CheckCircle2,
  Clock,
  Building,
} from "lucide-react";
import type {
  LaLicenseItem,
  LaActivation,
  LaCustomer,
  LaOverviewMetrics,
  LaLicenseStatus,
} from "./types";
import { IssueLicenseDialog } from "./IssueLicenseDialog";
import { RevokeLicenseDialog } from "./RevokeLicenseDialog";

export function LaAdminPage() {
  const { t } = useTranslation();
  const { formatDate } = useFormatters();
  const { toast } = useToast();
  const { hasPermission, isLoading: isSessionLoading } = useSession();

  const [activeTab, setActiveTab] = useState<"licenses" | "activations" | "customers">("licenses");
  const [metrics, setMetrics] = useState<LaOverviewMetrics | null>(null);
  const [licenses, setLicenses] = useState<LaLicenseItem[]>([]);
  const [activations, setActivations] = useState<LaActivation[]>([]);
  const [customers, setCustomers] = useState<LaCustomer[]>([]);
  const [isLoading, setIsLoading] = useState<boolean>(true);
  const [error, setError] = useState<string | null>(null);

  // Dialog states
  const [isIssueOpen, setIsIssueOpen] = useState<boolean>(false);
  const [revokingLicense, setRevokingLicense] = useState<LaLicenseItem | null>(null);

  const fetchOverview = async () => {
    try {
      setIsLoading(true);
      setError(null);

      const [resMetrics, resLicenses, resActivations, resCustomers] = await Promise.all([
        apiFetch<LaOverviewMetrics>("/api/v1/la/overview").catch(() => null),
        apiFetch<{ items: LaLicenseItem[] }>("/api/v1/la/licenses"),
        apiFetch<{ items: LaActivation[] }>("/api/v1/la/activations").catch(() => ({ items: [] })),
        apiFetch<{ items: LaCustomer[] }>("/api/v1/la/customers").catch(() => ({ items: [] })),
      ]);

      setLicenses(resLicenses.items || []);
      setActivations(resActivations.items || []);
      setCustomers(resCustomers.items || []);

      if (resMetrics) {
        setMetrics(resMetrics);
      } else {
        const total = resLicenses.items?.length || 0;
        const active = resLicenses.items?.filter((l) => l.status === "active").length || 0;
        const revoked = resLicenses.items?.filter((l) => l.status === "revoked").length || 0;
        setMetrics({
          total_licenses: total,
          active_licenses: active,
          revoked_licenses: revoked,
          total_customers: resCustomers.items?.length || 0,
          active_activations: resActivations.items?.length || 0,
        });
      }
    } catch {
      setError(t("common.error"));
    } finally {
      setIsLoading(false);
    }
  };

  useEffect(() => {
    fetchOverview();
  }, []);

  const handleLicenseIssued = (newLicense: LaLicenseItem) => {
    setLicenses((prev) => [newLicense, ...prev]);
    fetchOverview();
  };

  const handleLicenseRevoked = (updated: LaLicenseItem) => {
    setLicenses((prev) =>
      prev.map((l) => (l.id === updated.id ? { ...l, ...updated, status: "revoked" } : l))
    );
    fetchOverview();
  };

  const renderLicenseStatusBadge = (status: LaLicenseStatus) => {
    switch (status) {
      case "active":
        return (
          <Badge variant="success" className="text-[11px] gap-1 font-medium">
            <CheckCircle2 className="h-3 w-3" />
            {t("la.status.active")}
          </Badge>
        );
      case "revoked":
        return (
          <Badge
            variant="destructive"
            data-testid="license-revoked-badge"
            className="text-[11px] gap-1 font-medium"
          >
            <AlertOctagon className="h-3 w-3" />
            {t("la.status.revoked")}
          </Badge>
        );
      case "suspended":
        return (
          <Badge variant="outline" className="text-[11px] text-amber-500 border-amber-500 gap-1 font-medium">
            <Clock className="h-3 w-3" />
            {t("la.status.suspended")}
          </Badge>
        );
      case "expired":
      default:
        return (
          <Badge variant="secondary" className="text-[11px] text-muted-foreground gap-1 font-medium">
            <Clock className="h-3 w-3" />
            {t("la.status.expired")}
          </Badge>
        );
    }
  };

  if (isSessionLoading) {
    return (
      <div className="py-12 text-center text-sm text-muted-foreground">
        {t("common.loading")}
      </div>
    );
  }

  // Permission Guard
  if (!hasPermission("admin.license.read") && !hasPermission("admin.settings.read")) {
    return (
      <div className="p-8 text-center border rounded-lg bg-card">
        <p className="text-sm text-muted-foreground">{t("common.noPermission")}</p>
      </div>
    );
  }

  return (
    <div className="space-y-6">
      {/* Header */}
      <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-4">
        <div>
          <div className="flex items-center gap-2">
            <KeyRound className="h-6 w-6 text-primary" />
            <h1 className="text-2xl font-bold tracking-tight">{t("la.title")}</h1>
          </div>
          <p className="text-sm text-muted-foreground mt-1">{t("la.desc")}</p>
        </div>

        <div className="flex items-center gap-2">
          <Button
            size="sm"
            data-testid="btn-open-issue-license"
            onClick={() => setIsIssueOpen(true)}
            className="gap-1.5 text-xs"
          >
            <Plus className="h-3.5 w-3.5" />
            {t("la.issueBtn")}
          </Button>
          <Button
            variant="outline"
            size="sm"
            onClick={fetchOverview}
            disabled={isLoading}
            className="gap-1.5 text-xs"
          >
            <RotateCcw className={`h-3.5 w-3.5 ${isLoading ? "animate-spin" : ""}`} />
            {t("common.reload")}
          </Button>
        </div>
      </div>

      {/* Metrics Cards */}
      {metrics && (
        <div className="grid grid-cols-2 md:grid-cols-4 gap-4">
          <Card className="p-4 flex items-center justify-between border bg-card">
            <div>
              <p className="text-xs font-medium text-muted-foreground">{t("la.metrics.totalLicenses")}</p>
              <p className="text-2xl font-bold text-foreground mt-1" data-testid="metric-total-licenses">
                {metrics.total_licenses}
              </p>
            </div>
            <div className="h-10 w-10 rounded-full bg-primary/10 flex items-center justify-center text-primary">
              <KeyRound className="h-5 w-5" />
            </div>
          </Card>

          <Card className="p-4 flex items-center justify-between border bg-card">
            <div>
              <p className="text-xs font-medium text-muted-foreground">{t("la.metrics.activeLicenses")}</p>
              <p className="text-2xl font-bold text-emerald-600 dark:text-emerald-400 mt-1" data-testid="metric-active-licenses">
                {metrics.active_licenses}
              </p>
            </div>
            <div className="h-10 w-10 rounded-full bg-emerald-500/10 flex items-center justify-center text-emerald-500">
              <ShieldCheck className="h-5 w-5" />
            </div>
          </Card>

          <Card className="p-4 flex items-center justify-between border bg-card">
            <div>
              <p className="text-xs font-medium text-muted-foreground">{t("la.metrics.revokedLicenses")}</p>
              <p className="text-2xl font-bold text-destructive mt-1" data-testid="metric-revoked-licenses">
                {metrics.revoked_licenses}
              </p>
            </div>
            <div className="h-10 w-10 rounded-full bg-destructive/10 flex items-center justify-center text-destructive">
              <ShieldAlert className="h-5 w-5" />
            </div>
          </Card>

          <Card className="p-4 flex items-center justify-between border bg-card">
            <div>
              <p className="text-xs font-medium text-muted-foreground">{t("la.metrics.activeActivations")}</p>
              <p className="text-2xl font-bold text-blue-600 dark:text-blue-400 mt-1" data-testid="metric-active-activations">
                {metrics.active_activations}
              </p>
            </div>
            <div className="h-10 w-10 rounded-full bg-blue-500/10 flex items-center justify-center text-blue-500">
              <Server className="h-5 w-5" />
            </div>
          </Card>
        </div>
      )}

      {/* Tabs Toolbar */}
      <div className="flex border-b space-x-4 text-xs font-medium">
        <button
          onClick={() => setActiveTab("licenses")}
          data-testid="tab-licenses"
          className={`pb-2 border-b-2 transition-colors ${
            activeTab === "licenses"
              ? "border-primary text-primary font-bold"
              : "border-transparent text-muted-foreground hover:text-foreground"
          }`}
        >
          {t("la.tabs.licenses")} ({licenses.length})
        </button>
        <button
          onClick={() => setActiveTab("activations")}
          data-testid="tab-activations"
          className={`pb-2 border-b-2 transition-colors ${
            activeTab === "activations"
              ? "border-primary text-primary font-bold"
              : "border-transparent text-muted-foreground hover:text-foreground"
          }`}
        >
          {t("la.tabs.activations")} ({activations.length})
        </button>
        <button
          onClick={() => setActiveTab("customers")}
          data-testid="tab-customers"
          className={`pb-2 border-b-2 transition-colors ${
            activeTab === "customers"
              ? "border-primary text-primary font-bold"
              : "border-transparent text-muted-foreground hover:text-foreground"
          }`}
        >
          {t("la.tabs.customers")} ({customers.length})
        </button>
      </div>

      {/* Tab 1: Licenses Table */}
      {activeTab === "licenses" && (
        <Card className="border shadow-sm">
          <CardContent className="p-0">
            <div className="overflow-x-auto">
              <Table>
                <TableHeader className="bg-muted/40">
                  <TableRow>
                    <TableHead className="w-[25%]">{t("la.table.licenseKey")}</TableHead>
                    <TableHead className="w-[20%]">{t("la.table.customer")}</TableHead>
                    <TableHead className="w-[12%]">{t("la.table.tier")}</TableHead>
                    <TableHead className="w-[12%]">{t("la.table.limits")}</TableHead>
                    <TableHead className="w-[11%]">{t("la.table.status")}</TableHead>
                    <TableHead className="w-[10%]">{t("la.table.activations")}</TableHead>
                    <TableHead className="w-[10%] text-right pr-4">{t("la.table.actions")}</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {isLoading ? (
                    <TableRow>
                      <TableCell colSpan={7} className="h-32 text-center text-xs text-muted-foreground">
                        {t("common.loading")}
                      </TableCell>
                    </TableRow>
                  ) : licenses.length === 0 ? (
                    <TableRow>
                      <TableCell colSpan={7} className="h-32 text-center text-xs text-muted-foreground">
                        {t("la.empty")}
                      </TableCell>
                    </TableRow>
                  ) : (
                    licenses.map((license) => (
                      <TableRow key={license.id} data-testid={`license-row-${license.id}`}>
                        <TableCell>
                          <span className="font-mono text-xs font-bold text-foreground">
                            {license.license_key}
                          </span>
                        </TableCell>
                        <TableCell>
                          <div>
                            <p className="font-medium text-xs text-foreground truncate">{license.customer_name}</p>
                            <p className="text-[11px] text-muted-foreground font-mono truncate">{license.customer_email}</p>
                          </div>
                        </TableCell>
                        <TableCell>
                          <Badge variant="outline" className="uppercase font-mono text-[10px]">
                            {license.plan_tier}
                          </Badge>
                        </TableCell>
                        <TableCell>
                          <span className="text-xs text-muted-foreground">
                            {license.max_nodes} Nodes / {license.max_tenants} Users
                          </span>
                        </TableCell>
                        <TableCell>{renderLicenseStatusBadge(license.status)}</TableCell>
                        <TableCell>
                          <span className="text-xs font-mono">
                            {license.activations_count} / {license.max_activations}
                          </span>
                        </TableCell>
                        <TableCell className="text-right pr-4">
                          {license.status === "active" && (
                            <Button
                              variant="outline"
                              size="sm"
                              data-testid={`btn-revoke-license-${license.id}`}
                              onClick={() => setRevokingLicense(license)}
                              className="h-7 px-2 text-xs text-destructive hover:bg-destructive/10 border-destructive/30"
                            >
                              {t("la.revokeBtn")}
                            </Button>
                          )}
                        </TableCell>
                      </TableRow>
                    ))
                  )}
                </TableBody>
              </Table>
            </div>
          </CardContent>
        </Card>
      )}

      {/* Tab 2: Activations Table */}
      {activeTab === "activations" && (
        <Card className="border shadow-sm">
          <CardContent className="p-0">
            <div className="overflow-x-auto">
              <Table>
                <TableHeader className="bg-muted/40">
                  <TableRow>
                    <TableHead>{t("la.table.installationId")}</TableHead>
                    <TableHead>{t("la.table.licenseKey")}</TableHead>
                    <TableHead>{t("la.table.serverIp")}</TableHead>
                    <TableHead>{t("la.table.hostname")}</TableHead>
                    <TableHead>{t("la.table.lastHeartbeat")}</TableHead>
                    <TableHead>{t("la.table.status")}</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {activations.length === 0 ? (
                    <TableRow>
                      <TableCell colSpan={6} className="h-28 text-center text-xs text-muted-foreground">
                        {t("la.activations.empty")}
                      </TableCell>
                    </TableRow>
                  ) : (
                    activations.map((act) => (
                      <TableRow key={act.id}>
                        <TableCell className="font-mono text-xs">{act.installation_id}</TableCell>
                        <TableCell className="font-mono text-xs">{act.license_key}</TableCell>
                        <TableCell className="font-mono text-xs">{act.server_ip}</TableCell>
                        <TableCell className="text-xs">{act.hostname}</TableCell>
                        <TableCell className="text-xs text-muted-foreground">
                          {formatDate(new Date(act.last_heartbeat_at))}
                        </TableCell>
                        <TableCell>
                          <Badge variant="outline" className="text-[10px] text-emerald-600 border-emerald-500/30">
                            {act.status}
                          </Badge>
                        </TableCell>
                      </TableRow>
                    ))
                  )}
                </TableBody>
              </Table>
            </div>
          </CardContent>
        </Card>
      )}

      {/* Tab 3: Customers Directory */}
      {activeTab === "customers" && (
        <Card className="border shadow-sm">
          <CardContent className="p-0">
            <div className="overflow-x-auto">
              <Table>
                <TableHeader className="bg-muted/40">
                  <TableRow>
                    <TableHead>{t("la.customers.name")}</TableHead>
                    <TableHead>{t("la.customers.email")}</TableHead>
                    <TableHead>{t("la.customers.company")}</TableHead>
                    <TableHead>{t("la.customers.licensesCount")}</TableHead>
                    <TableHead>{t("la.customers.registered")}</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {customers.length === 0 ? (
                    <TableRow>
                      <TableCell colSpan={5} className="h-28 text-center text-xs text-muted-foreground">
                        {t("la.customers.empty")}
                      </TableCell>
                    </TableRow>
                  ) : (
                    customers.map((cust) => (
                      <TableRow key={cust.id}>
                        <TableCell className="font-semibold text-xs text-foreground">{cust.name}</TableCell>
                        <TableCell className="font-mono text-xs text-muted-foreground">{cust.email}</TableCell>
                        <TableCell className="text-xs">{cust.company || "—"}</TableCell>
                        <TableCell className="font-mono text-xs">{cust.licenses_count}</TableCell>
                        <TableCell className="text-xs text-muted-foreground">
                          {formatDate(new Date(cust.created_at))}
                        </TableCell>
                      </TableRow>
                    ))
                  )}
                </TableBody>
              </Table>
            </div>
          </CardContent>
        </Card>
      )}

      {/* Issue License Dialog (DoD T17.06) */}
      <IssueLicenseDialog
        open={isIssueOpen}
        onOpenChange={setIsIssueOpen}
        onLicenseIssued={handleLicenseIssued}
      />

      {/* Revoke License Dialog (DoD T17.06) */}
      <RevokeLicenseDialog
        license={revokingLicense}
        open={!!revokingLicense}
        onOpenChange={(open) => !open && setRevokingLicense(null)}
        onLicenseRevoked={handleLicenseRevoked}
      />
    </div>
  );
}
