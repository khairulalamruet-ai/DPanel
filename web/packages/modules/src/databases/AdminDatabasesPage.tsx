import React, { useState, useEffect, useCallback } from "react";
import {
  Card,
  CardHeader,
  CardTitle,
  CardContent,
  Button,
  Badge,
  Table,
  TableHeader,
  TableRow,
  TableHead,
  TableBody,
  TableCell,
  Input,
} from "@builtpanel/ui";
import { apiFetch } from "@builtpanel/api";
import {
  Database,
  Server,
  HardDrive,
  Users,
  Search,
  RefreshCw,
  ArrowUpDown,
  Filter,
  CheckCircle2,
  AlertTriangle,
} from "lucide-react";
import { AdminDbOverviewResponse, AdminDbNodeDistribution, AdminLargestDatabaseItem } from "./types";
import { useTranslation } from "@builtpanel/i18n";

export function AdminDatabasesPage() {
  const { t } = useTranslation();

  const [data, setData] = useState<AdminDbOverviewResponse | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [searchQuery, setSearchQuery] = useState("");
  const [selectedNode, setSelectedNode] = useState<string>("all");

  const fetchOverview = useCallback(async () => {
    try {
      setIsLoading(true);
      setError(null);
      const res = await apiFetch<AdminDbOverviewResponse>("/api/v1/admin/databases/overview");
      setData(res);
    } catch (err: any) {
      setError(err?.message || "Failed to load admin databases overview");
    } finally {
      setIsLoading(false);
    }
  }, []);

  useEffect(() => {
    fetchOverview();
  }, [fetchOverview]);

  if (isLoading) {
    return (
      <div className="flex h-96 items-center justify-center">
        <p className="text-sm text-muted-foreground">{t("common.loading")}</p>
      </div>
    );
  }

  if (error || !data) {
    return (
      <div className="rounded-lg border border-destructive/20 bg-destructive/10 p-6 text-center">
        <p className="text-sm text-destructive">{error || t("databases.errors.loadFailed")}</p>
      </div>
    );
  }

  const { summary, node_distributions, largest_databases } = data;

  // Filtered largest databases
  const filteredDbs = largest_databases.filter((db: AdminLargestDatabaseItem) => {
    const matchesSearch =
      searchQuery === "" ||
      db.name.toLowerCase().includes(searchQuery.toLowerCase()) ||
      db.tenant_username.toLowerCase().includes(searchQuery.toLowerCase());
    const matchesNode = selectedNode === "all" || db.node_id === selectedNode;
    return matchesSearch && matchesNode;
  });

  return (
    <div className="space-y-6">
      {/* Header */}
      <div className="flex flex-col gap-4 sm:flex-row sm:items-center sm:justify-between">
        <div>
          <h1 className="text-2xl font-bold tracking-tight text-foreground" data-testid="admin-db-title">
            {t("databases.admin.title", "Cluster Databases Overview")}
          </h1>
          <p className="text-sm text-muted-foreground mt-1">
            {t("databases.admin.subtitle", "Node-wise database allocations, capacity utilization, and high-consumption database metrics.")}
          </p>
        </div>

        <div className="flex items-center gap-2">
          <Button
            variant="outline"
            size="sm"
            onClick={fetchOverview}
            className="text-xs"
            data-testid="btn-refresh-admin-db"
          >
            <RefreshCw className="h-3.5 w-3.5 mr-1.5" />
            {t("common.refresh")}
          </Button>
        </div>
      </div>

      {/* Summary Metrics Cards */}
      <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-4 gap-4">
        <Card>
          <CardContent className="p-4 flex items-center justify-between">
            <div>
              <p className="text-xs font-medium text-muted-foreground">
                {t("databases.admin.totalDbs", "Total Databases")}
              </p>
              <div className="flex items-baseline gap-2 mt-1">
                <span className="text-2xl font-bold font-mono text-foreground" data-testid="metric-total-dbs">
                  {summary.total_databases}
                </span>
              </div>
            </div>
            <div className="rounded-full bg-primary/10 p-2.5 text-primary">
              <Database className="h-5 w-5" />
            </div>
          </CardContent>
        </Card>

        <Card>
          <CardContent className="p-4 flex items-center justify-between">
            <div>
              <p className="text-xs font-medium text-muted-foreground">
                {t("databases.admin.totalStorage", "Cluster DB Storage")}
              </p>
              <div className="flex items-baseline gap-2 mt-1">
                <span className="text-2xl font-bold font-mono text-foreground" data-testid="metric-total-storage">
                  {summary.total_size_formatted}
                </span>
              </div>
            </div>
            <div className="rounded-full bg-primary/10 p-2.5 text-primary">
              <HardDrive className="h-5 w-5" />
            </div>
          </CardContent>
        </Card>

        <Card>
          <CardContent className="p-4 flex items-center justify-between">
            <div>
              <p className="text-xs font-medium text-muted-foreground">
                {t("databases.admin.totalUsers", "Total Database Users")}
              </p>
              <div className="flex items-baseline gap-2 mt-1">
                <span className="text-2xl font-bold font-mono text-foreground" data-testid="metric-total-users">
                  {summary.total_users}
                </span>
              </div>
            </div>
            <div className="rounded-full bg-primary/10 p-2.5 text-primary">
              <Users className="h-5 w-5" />
            </div>
          </CardContent>
        </Card>

        <Card>
          <CardContent className="p-4 flex items-center justify-between">
            <div>
              <p className="text-xs font-medium text-muted-foreground">
                {t("databases.admin.activeNodes", "Active DB Nodes")}
              </p>
              <div className="flex items-baseline gap-2 mt-1">
                <span className="text-2xl font-bold font-mono text-foreground" data-testid="metric-active-nodes">
                  {summary.active_nodes}
                </span>
              </div>
            </div>
            <div className="rounded-full bg-primary/10 p-2.5 text-primary">
              <Server className="h-5 w-5" />
            </div>
          </CardContent>
        </Card>
      </div>

      {/* DoD Section 1: Node Distributions (Node অনুযায়ী DB সংখ্যা আর size) */}
      <div className="space-y-4">
        <h2 className="text-base font-semibold text-foreground">
          {t("databases.admin.nodeDistributionTitle", "Database Distribution by Node")}
        </h2>

        <div className="grid grid-cols-1 md:grid-cols-3 gap-4">
          {node_distributions.map((node: AdminDbNodeDistribution) => (
            <Card key={node.node_id} data-testid={`card-db-node-${node.node_id}`}>
              <CardContent className="p-4 space-y-3">
                <div className="flex items-center justify-between">
                  <div className="flex items-center gap-2">
                    <Server className="h-4 w-4 text-primary" />
                    <span className="font-semibold text-sm text-foreground">{node.node_name}</span>
                  </div>
                  <Badge
                    variant={node.status === "healthy" ? "success" : "secondary"}
                    className="text-xs capitalize"
                  >
                    {node.status}
                  </Badge>
                </div>

                <div className="grid grid-cols-2 gap-2 text-xs border-t border-border pt-2 font-mono">
                  <div>
                    <span className="text-muted-foreground block text-[11px]">
                      {t("databases.table.tables", "Databases")}
                    </span>
                    <span className="font-semibold text-foreground text-sm" data-testid={`node-count-${node.node_id}`}>
                      {node.database_count}
                    </span>
                  </div>
                  <div>
                    <span className="text-muted-foreground block text-[11px]">
                      {t("databases.table.size", "Total Size")}
                    </span>
                    <span className="font-semibold text-foreground text-sm" data-testid={`node-size-${node.node_id}`}>
                      {node.total_size_formatted}
                    </span>
                  </div>
                </div>

                <div className="text-[11px] text-muted-foreground font-mono flex items-center justify-between border-t border-border pt-2">
                  <span>Engine:</span>
                  <span className="text-foreground">{node.db_version}</span>
                </div>
              </CardContent>
            </Card>
          ))}
        </div>
      </div>

      {/* DoD Section 2: Largest Databases Table (বড় DB-র তালিকা) */}
      <div className="space-y-4">
        <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
          <h2 className="text-base font-semibold text-foreground">
            {t("databases.admin.largestDbsTitle", "Largest Databases (Top Consumption)")}
          </h2>

          <div className="flex flex-wrap items-center gap-2">
            {/* Search */}
            <div className="relative w-48 sm:w-64">
              <Search className="absolute left-2.5 top-2.5 h-3.5 w-3.5 text-muted-foreground" />
              <input
                type="text"
                placeholder={t("databases.admin.searchPlaceholder", "Search db or tenant...")}
                value={searchQuery}
                onChange={(e) => setSearchQuery(e.target.value)}
                className="w-full h-8 pl-8 pr-3 rounded-md border border-input bg-background text-xs font-mono focus:outline-none focus:ring-1 focus:ring-ring"
                data-testid="input-search-largest-dbs"
              />
            </div>

            {/* Node Filter */}
            <select
              value={selectedNode}
              onChange={(e) => setSelectedNode(e.target.value)}
              className="h-8 rounded-md border border-input bg-background px-2 text-xs font-mono"
              data-testid="select-filter-node"
            >
              <option value="all">All Nodes</option>
              {node_distributions.map((n) => (
                <option key={n.node_id} value={n.node_id}>
                  {n.node_name}
                </option>
              ))}
            </select>
          </div>
        </div>

        <div className="rounded-lg border border-border bg-card overflow-hidden">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>{t("databases.table.name", "Database Name")}</TableHead>
                <TableHead>{t("databases.admin.tenant", "Tenant Owner")}</TableHead>
                <TableHead>{t("databases.admin.node", "Cluster Node")}</TableHead>
                <TableHead>{t("databases.table.size", "Size")}</TableHead>
                <TableHead>{t("databases.table.tables", "Tables")}</TableHead>
                <TableHead>{t("databases.table.charset", "Charset")}</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {filteredDbs.length === 0 ? (
                <TableRow>
                  <TableCell colSpan={6} className="text-center py-8 text-muted-foreground">
                    {t("databases.admin.noMatches", "No databases match the filter.")}
                  </TableCell>
                </TableRow>
              ) : (
                filteredDbs.map((db: AdminLargestDatabaseItem) => (
                  <TableRow key={db.id} data-testid={`largest-db-row-${db.name}`}>
                    <TableCell className="font-mono text-xs font-semibold text-foreground">
                      <div className="flex items-center gap-2">
                        <Database className="h-3.5 w-3.5 text-primary" />
                        <span>{db.name}</span>
                      </div>
                    </TableCell>
                    <TableCell className="font-mono text-xs text-muted-foreground">
                      <Badge variant="secondary" className="text-[10px] py-0">
                        {db.tenant_username}
                      </Badge>
                    </TableCell>
                    <TableCell className="font-mono text-xs text-muted-foreground">
                      {db.node_name}
                    </TableCell>
                    <TableCell className="font-mono text-xs font-semibold text-foreground" data-testid={`db-size-${db.name}`}>
                      {db.size_formatted}
                    </TableCell>
                    <TableCell className="font-mono text-xs text-muted-foreground">
                      {db.tables_count}
                    </TableCell>
                    <TableCell className="font-mono text-[11px] text-muted-foreground">
                      {db.charset}
                    </TableCell>
                  </TableRow>
                ))
              )}
            </TableBody>
          </Table>
        </div>
      </div>
    </div>
  );
}
