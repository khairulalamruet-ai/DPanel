import React, { useState, useEffect, useMemo } from "react";
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
  Input,
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogFooter,
  useToast,
} from "@builtpanel/ui";
import { useTranslation } from "@builtpanel/i18n";
import { loadClientDomains, createDomain, updateDocroot, setRedirect, deleteDomain } from "./api";
import {
  Globe,
  Plus,
  RefreshCw,
  Search,
  ExternalLink,
  Folder,
  ArrowRightLeft,
  ShieldCheck,
  ShieldAlert,
  ShieldOff,
  AlertCircle,
  Clock,
  Trash2,
  Edit2,
  AlertTriangle,
  Radio,
} from "lucide-react";
import {
  ClientDomainItem,
  ClientDomainsResponse,
  CreateDomainPayload,
  UpdateDocrootPayload,
  UpdateRedirectPayload,
  DomainType,
  SslStatus,
  DnsStatus,
} from "./types";
import { AddDomainDialog } from "./AddDomainDialog";
import { EditDocrootDialog } from "./EditDocrootDialog";
import { ManageRedirectDialog } from "./ManageRedirectDialog";
import { PreviewLinkDialog } from "./PreviewLinkDialog";

export function ClientDomainsPage() {
  const { t } = useTranslation();
  const { toast } = useToast();

  const [domains, setDomains] = useState<ClientDomainItem[]>([]);
  const [limits, setLimits] = useState<{ max_domains: number; used_domains: number }>({
    max_domains: -1,
    used_domains: 0,
  });
  const [isLoading, setIsLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  // Search & Filter
  const [searchQuery, setSearchQuery] = useState("");
  const [typeFilter, setTypeFilter] = useState<string>("all");

  // Dialog states
  const [isAddOpen, setIsAddOpen] = useState(false);
  const [editingDocrootDomain, setEditingDocrootDomain] = useState<ClientDomainItem | null>(null);
  const [managingRedirectDomain, setManagingRedirectDomain] = useState<ClientDomainItem | null>(null);
  const [previewLinkDomain, setPreviewLinkDomain] = useState<ClientDomainItem | null>(null);
  const [deletingDomain, setDeletingDomain] = useState<ClientDomainItem | null>(null);
  const [isDeleting, setIsDeleting] = useState(false);

  const fetchDomains = async () => {
    try {
      setIsLoading(true);
      setError(null);
      const res = await loadClientDomains();
      setDomains(res.items);
      setLimits((prev) => ({ ...prev, used_domains: res.total }));
    } catch (err: any) {
      setError(err?.message || t("domains.errors.loadFailed", "Failed to load domains"));
    } finally {
      setIsLoading(false);
    }
  };

  useEffect(() => {
    fetchDomains();
  }, []);

  const handleCreateDomain = async (payload: CreateDomainPayload) => {
    await createDomain(payload);
    toast({
      title: t("common.success", "Success"),
      description: t("domains.toast.createdSuccess", "Domain {{name}} added successfully", {
        name: payload.name,
      }),
    });
    await fetchDomains();
  };

  const handleUpdateDocroot = async (domainId: string, docroot: string) => {
    await updateDocroot(domainId, docroot);
    toast({
      title: t("common.success", "Success"),
      description: t("domains.toast.docrootUpdated", "Document root updated successfully"),
    });
    await fetchDomains();
  };

  const handleUpdateRedirect = async (domainId: string, payload: UpdateRedirectPayload) => {
    await setRedirect(domainId, payload);
    toast({
      title: t("common.success", "Success"),
      description: t("domains.toast.redirectUpdated", "Redirection updated successfully"),
    });
    await fetchDomains();
  };

  const handleDeleteDomain = async () => {
    if (!deletingDomain) return;
    try {
      setIsDeleting(true);
      await deleteDomain(deletingDomain.id);
      toast({
        title: t("common.success", "Success"),
        description: t("domains.toast.deletedSuccess", "Domain deleted successfully"),
      });
      setDeletingDomain(null);
      await fetchDomains();
    } catch (err: any) {
      toast({
        title: t("common.error", "Error"),
        description: err?.message || t("domains.toast.deleteFailed", "Failed to delete domain"),
        variant: "destructive",
      });
    } finally {
      setIsDeleting(false);
    }
  };

  const filteredDomains = useMemo(() => {
    return domains.filter((d) => {
      const matchSearch =
        d.name.toLowerCase().includes(searchQuery.toLowerCase()) ||
        d.docroot.toLowerCase().includes(searchQuery.toLowerCase());
      const matchType = typeFilter === "all" || d.type === typeFilter;
      return matchSearch && matchType;
    });
  }, [domains, searchQuery, typeFilter]);

  const isLimitReached =
    limits.max_domains !== -1 && limits.used_domains >= limits.max_domains;

  const renderTypeBadge = (type: DomainType, isPrimary: boolean) => {
    if (isPrimary) {
      return (
        <Badge variant="default" className="bg-primary/90 text-primary-foreground font-semibold">
          {t("domains.types.primary", "Primary")}
        </Badge>
      );
    }
    switch (type) {
      case "addon":
        return <Badge variant="secondary">{t("domains.types.addon", "Addon")}</Badge>;
      case "subdomain":
        return <Badge variant="outline">{t("domains.types.subdomain", "Subdomain")}</Badge>;
      case "alias":
        return <Badge variant="outline" className="border-dashed">{t("domains.types.alias", "Alias")}</Badge>;
      default:
        return <Badge variant="outline">{type}</Badge>;
    }
  };

  const renderSslBadge = (status: SslStatus) => {
    switch (status) {
      case "active":
        return (
          <span className="inline-flex items-center gap-1 text-xs font-medium text-emerald-600 dark:text-emerald-400">
            <ShieldCheck className="h-4 w-4" />
            {t("domains.ssl.active", "Active")}
          </span>
        );
      case "pending":
        return (
          <span className="inline-flex items-center gap-1 text-xs font-medium text-amber-600 dark:text-amber-400">
            <Clock className="h-4 w-4 animate-spin" />
            {t("domains.ssl.pending", "Issuing")}
          </span>
        );
      case "expired":
        return (
          <span className="inline-flex items-center gap-1 text-xs font-medium text-rose-600 dark:text-rose-400">
            <AlertCircle className="h-4 w-4" />
            {t("domains.ssl.expired", "Expired")}
          </span>
        );
      case "self_signed":
        return (
          <span className="inline-flex items-center gap-1 text-xs font-medium text-orange-600 dark:text-orange-400">
            <ShieldAlert className="h-4 w-4" />
            {t("domains.ssl.selfSigned", "Self-Signed")}
          </span>
        );
      case "none":
      default:
        return (
          <span className="inline-flex items-center gap-1 text-xs font-medium text-muted-foreground">
            <ShieldOff className="h-4 w-4" />
            {t("domains.ssl.none", "No SSL")}
          </span>
        );
    }
  };

  const renderDnsBadge = (status: DnsStatus) => {
    switch (status) {
      case "active":
        return (
          <span className="inline-flex items-center gap-1.5 text-xs text-muted-foreground">
            <span className="h-2 w-2 rounded-full bg-emerald-500" />
            {t("domains.dns.resolved", "Connected")}
          </span>
        );
      case "pending":
        return (
          <span className="inline-flex items-center gap-1.5 text-xs text-muted-foreground">
            <span className="h-2 w-2 rounded-full bg-amber-500 animate-pulse" />
            {t("domains.dns.pending", "Pending DNS")}
          </span>
        );
      case "proxied":
        return (
          <span className="inline-flex items-center gap-1.5 text-xs text-muted-foreground">
            <span className="h-2 w-2 rounded-full bg-blue-500" />
            {t("domains.dns.proxied", "Cloudflare CDN")}
          </span>
        );
      case "error":
      default:
        return (
          <span className="inline-flex items-center gap-1.5 text-xs text-rose-500">
            <span className="h-2 w-2 rounded-full bg-rose-500" />
            {t("domains.dns.unresolved", "Not Pointing")}
          </span>
        );
    }
  };

  return (
    <div className="space-y-6">
      {/* Page Header */}
      <div className="flex flex-col sm:flex-row sm:items-center sm:justify-between gap-4">
        <div>
          <h1 className="text-2xl font-bold tracking-tight flex items-center gap-2">
            <Globe className="h-6 w-6 text-primary" />
            {t("domains.title", "Domains")}
          </h1>
          <p className="text-sm text-muted-foreground mt-1">
            {t(
              "domains.subtitle",
              "Manage domains, subdomains, document roots, URL redirections, and SSL certificates."
            )}
          </p>
        </div>

        <div className="flex items-center gap-2">
          <Button
            variant="outline"
            size="sm"
            onClick={fetchDomains}
            disabled={isLoading}
            className="h-9 gap-1.5"
            data-testid="btn-refresh-domains"
          >
            <RefreshCw className={`h-4 w-4 ${isLoading ? "animate-spin" : ""}`} />
            <span className="hidden sm:inline">{t("common.refresh", "Refresh")}</span>
          </Button>

          <Button
            size="sm"
            onClick={() => setIsAddOpen(true)}
            disabled={isLoading || isLimitReached}
            className="h-9 gap-1.5"
            data-testid="btn-add-domain"
          >
            <Plus className="h-4 w-4" />
            {t("domains.addDomain", "Add Domain")}
          </Button>
        </div>
      </div>

      {/* Quota Limit Notice */}
      {limits.max_domains !== -1 && (
        <div
          className={`p-4 rounded-xl border flex flex-col sm:flex-row sm:items-center justify-between gap-3 ${
            isLimitReached
              ? "bg-amber-500/10 border-amber-500/30 text-amber-900 dark:text-amber-200"
              : "bg-card text-card-foreground border-border"
          }`}
          data-testid="domain-quota-banner"
        >
          <div className="flex items-center gap-3">
            <Radio className="h-5 w-5 text-primary shrink-0" />
            <div>
              <p className="text-sm font-semibold">
                {t("domains.quota.title", "Domain Allocation Usage")}
              </p>
              <p className="text-xs text-muted-foreground mt-0.5">
                {t("domains.quota.description", "{{used}} of {{max}} domains configured", {
                  used: limits.used_domains,
                  max: limits.max_domains,
                })}
              </p>
            </div>
          </div>
          {isLimitReached && (
            <Badge variant="destructive" className="self-start sm:self-auto font-medium">
              {t("domains.quota.limitReachedBadge", "Quota Reached")}
            </Badge>
          )}
        </div>
      )}

      {/* Filter / Search Bar */}
      <div className="flex flex-col sm:flex-row gap-3">
        <div className="relative flex-1">
          <Search className="absolute left-3 top-1/2 -translate-y-1/2 h-4 w-4 text-muted-foreground" />
          <Input
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
            placeholder={t("domains.searchPlaceholder", "Search domain or docroot...")}
            className="pl-9 h-9"
            data-testid="input-search-domains"
          />
        </div>

        <select
          value={typeFilter}
          onChange={(e) => setTypeFilter(e.target.value)}
          data-testid="select-filter-type"
          className="h-9 px-3 rounded-md border border-input bg-background text-sm shadow-sm transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
        >
          <option value="all">{t("domains.filter.allTypes", "All Types")}</option>
          <option value="addon">{t("domains.types.addon", "Addon Domains")}</option>
          <option value="subdomain">{t("domains.types.subdomain", "Subdomains")}</option>
          <option value="alias">{t("domains.types.alias", "Aliases")}</option>
        </select>
      </div>

      {/* Domains Table Card */}
      <Card>
        <CardHeader className="pb-3">
          <CardTitle className="text-base font-medium">
            {t("domains.configuredDomains", "Configured Domains")}
          </CardTitle>
          <CardDescription>
            {t("domains.tableDescription", "Domains served by web server with routing configuration")}
          </CardDescription>
        </CardHeader>
        <CardContent className="p-0">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>{t("domains.table.domain", "Domain")}</TableHead>
                <TableHead>{t("domains.table.type", "Type")}</TableHead>
                <TableHead>{t("domains.table.routing", "Document Root / Redirect")}</TableHead>
                <TableHead>{t("domains.table.ssl", "SSL Security")}</TableHead>
                <TableHead>{t("domains.table.dns", "DNS Status")}</TableHead>
                <TableHead className="text-right">{t("common.actions", "Actions")}</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {isLoading ? (
                <TableRow>
                  <TableCell colSpan={6} className="h-32 text-center text-muted-foreground">
                    <RefreshCw className="h-5 w-5 animate-spin mx-auto mb-2 text-primary" />
                    {t("common.loading", "Loading domains...")}
                  </TableCell>
                </TableRow>
              ) : error ? (
                <TableRow>
                  <TableCell colSpan={6} className="h-32 text-center text-destructive">
                    <AlertTriangle className="h-5 w-5 mx-auto mb-2" />
                    {error}
                  </TableCell>
                </TableRow>
              ) : filteredDomains.length === 0 ? (
                <TableRow>
                  <TableCell colSpan={6} className="h-32 text-center text-muted-foreground">
                    <Globe className="h-6 w-6 mx-auto mb-2 opacity-40" />
                    {searchQuery
                      ? t("domains.noResults", "No domains match your search query.")
                      : t("domains.noDomains", "No domains configured yet.")}
                  </TableCell>
                </TableRow>
              ) : (
                filteredDomains.map((domain) => {
                  const hasRedirect = domain.redirect && domain.redirect.enabled;

                  return (
                    <TableRow key={domain.id} data-testid={`domain-row-${domain.id}`}>
                      {/* Domain Name */}
                      <TableCell className="font-medium">
                        <div className="flex items-center gap-2">
                          <span className="font-mono font-semibold text-foreground">
                            {domain.name}
                          </span>
                          <a
                            href={`https://${domain.name}`}
                            target="_blank"
                            rel="noopener noreferrer"
                            className="text-muted-foreground hover:text-foreground"
                            title={t("domains.openSite", "Open website in new tab")}
                          >
                            <ExternalLink className="h-3.5 w-3.5" />
                          </a>
                        </div>
                      </TableCell>

                      {/* Type Badge */}
                      <TableCell>{renderTypeBadge(domain.type, domain.is_primary)}</TableCell>

                      {/* Routing: Docroot or Redirect */}
                      <TableCell>
                        {hasRedirect ? (
                          <div className="flex items-center gap-1.5 text-xs font-mono">
                            <ArrowRightLeft className="h-3.5 w-3.5 text-primary shrink-0" />
                            <Badge variant="outline" className="text-[10px] px-1 py-0">
                              {domain.redirect?.status_code}
                            </Badge>
                            <span className="truncate max-w-[220px]" title={domain.redirect?.target_url}>
                              {domain.redirect?.target_url}
                            </span>
                          </div>
                        ) : (
                          <div className="flex items-center gap-1.5 text-xs font-mono text-muted-foreground">
                            <Folder className="h-3.5 w-3.5 shrink-0" />
                            <span className="truncate max-w-[240px]" title={domain.docroot}>
                              {domain.docroot}
                            </span>
                          </div>
                        )}
                      </TableCell>

                      {/* SSL Status */}
                      <TableCell>{renderSslBadge(domain.ssl_status)}</TableCell>

                      {/* DNS Status */}
                      <TableCell>{renderDnsBadge(domain.dns_status)}</TableCell>

                      {/* Actions */}
                      <TableCell className="text-right">
                        <div className="flex items-center justify-end gap-1">
                          {/* Edit Docroot Button */}
                          <Button
                            variant="ghost"
                            size="sm"
                            className="h-8 w-8 p-0"
                            title={t("domains.editDocroot.title", "Edit Document Root")}
                            onClick={() => setEditingDocrootDomain(domain)}
                            data-testid={`btn-edit-docroot-${domain.id}`}
                          >
                            <Folder className="h-4 w-4" />
                          </Button>

                          {/* Manage Redirect Button */}
                          <Button
                            variant="ghost"
                            size="sm"
                            className="h-8 w-8 p-0"
                            title={t("domains.redirect.title", "Manage Redirection")}
                            onClick={() => setManagingRedirectDomain(domain)}
                            data-testid={`btn-manage-redirect-${domain.id}`}
                          >
                            <ArrowRightLeft className="h-4 w-4" />
                          </Button>

                          {/* Temporary Preview Link Button (T06.14) */}
                          <Button
                            variant="ghost"
                            size="sm"
                            className="h-8 w-8 p-0 text-primary hover:text-primary hover:bg-primary/10"
                            title={t("domains.preview.actionTooltip", "Temporary Preview Link")}
                            onClick={() => setPreviewLinkDomain(domain)}
                            data-testid={`btn-preview-link-${domain.id}`}
                          >
                            <Globe className="h-4 w-4" />
                          </Button>

                          {/* Delete Button (disabled for primary domain) */}
                          <Button
                            variant="ghost"
                            size="sm"
                            className="h-8 w-8 p-0 text-destructive hover:text-destructive hover:bg-destructive/10 disabled:opacity-30"
                            disabled={domain.is_primary}
                            title={
                              domain.is_primary
                                ? t("domains.cannotDeletePrimary", "Primary domain cannot be deleted")
                                : t("common.delete", "Delete domain")
                            }
                            onClick={() => setDeletingDomain(domain)}
                            data-testid={`btn-delete-domain-${domain.id}`}
                          >
                            <Trash2 className="h-4 w-4" />
                          </Button>
                        </div>
                      </TableCell>
                    </TableRow>
                  );
                })
              )}
            </TableBody>
          </Table>
        </CardContent>
      </Card>

      {/* Add Domain Dialog */}
      <AddDomainDialog
        open={isAddOpen}
        onOpenChange={setIsAddOpen}
        onSubmit={handleCreateDomain}
        isLimitReached={isLimitReached}
      />

      {/* Edit Docroot Dialog */}
      <EditDocrootDialog
        open={!!editingDocrootDomain}
        onOpenChange={(open) => !open && setEditingDocrootDomain(null)}
        domain={editingDocrootDomain}
        onSubmit={handleUpdateDocroot}
      />

      {/* Manage Redirect Dialog */}
      <ManageRedirectDialog
        open={!!managingRedirectDomain}
        onOpenChange={(open) => !open && setManagingRedirectDomain(null)}
        domain={managingRedirectDomain}
        onSubmit={handleUpdateRedirect}
      />

      {/* Delete Confirmation Dialog */}
      <Dialog open={!!deletingDomain} onOpenChange={(open) => !open && setDeletingDomain(null)}>
        <DialogContent className="max-w-md">
          <DialogHeader>
            <DialogTitle className="flex items-center gap-2 text-destructive">
              <AlertTriangle className="h-5 w-5" />
              {t("domains.deleteDialog.title", "Delete Domain")}
            </DialogTitle>
          </DialogHeader>

          <div className="py-2 space-y-3 text-sm">
            <p>
              {t(
                "domains.deleteDialog.confirm",
                "Are you sure you want to remove domain {{name}}? Web server virtual host configurations will be removed.",
                { name: deletingDomain?.name }
              )}
            </p>
            <p className="text-xs text-muted-foreground">
              {t(
                "domains.deleteDialog.warning",
                "Note: Directory files will remain on disk for safety unless manually deleted."
              )}
            </p>
          </div>

          <DialogFooter className="pt-2">
            <Button
              type="button"
              variant="outline"
              onClick={() => setDeletingDomain(null)}
              disabled={isDeleting}
            >
              {t("common.cancel", "Cancel")}
            </Button>
            <Button
              variant="destructive"
              onClick={handleDeleteDomain}
              disabled={isDeleting}
              data-testid="btn-confirm-delete-domain"
            >
              {isDeleting
                ? t("common.deleting", "Deleting...")
                : t("common.delete", "Delete")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* Temporary Preview Link Dialog (T06.14) */}
      <PreviewLinkDialog
        open={!!previewLinkDomain}
        onOpenChange={(open) => !open && setPreviewLinkDomain(null)}
        domain={previewLinkDomain}
      />
    </div>
  );
}
