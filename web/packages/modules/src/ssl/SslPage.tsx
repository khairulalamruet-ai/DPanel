import React, { useState, useEffect, useCallback, useMemo } from "react";
import {
  Card,
  CardContent,
  Button,
  Badge,
  Input,
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
  useToast,
} from "@builtpanel/ui";
import { loadCertificates, loadCertificate, removeCustomCertificate } from "./api";
import { useTranslation } from "@builtpanel/i18n";
import {
  Lock,
  Shield,
  ShieldCheck,
  ShieldAlert,
  AlertTriangle,
  RefreshCw,
  UploadCloud,
  Settings,
  Trash2,
  Search,
  ExternalLink,
} from "lucide-react";
import type { SslCertificateItem, SslStatus } from "./types";
import { CustomCertUploadDialog } from "./CustomCertUploadDialog";
import { SslSettingsDialog } from "./SslSettingsDialog";

export function SslPage() {
  const { t } = useTranslation();
  const { toast } = useToast();

  const [certificates, setCertificates] = useState<SslCertificateItem[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [searchQuery, setSearchQuery] = useState("");
  const [statusFilter, setStatusFilter] = useState<string>("ALL");

  // Reissuing state
  const [reissuingDomainId, setReissuingDomainId] = useState<string | null>(null);

  // Dialogs
  const [customUploadTarget, setCustomUploadTarget] = useState<SslCertificateItem | null>(null);
  const [settingsTarget, setSettingsTarget] = useState<SslCertificateItem | null>(null);
  const [revokingTarget, setRevokingTarget] = useState<SslCertificateItem | null>(null);

  const fetchCertificates = useCallback(async () => {
    try {
      setIsLoading(true);
      setCertificates(await loadCertificates());
    } catch {
      toast({
        title: t("common.error", "Error"),
        description: t("ssl.errors.fetchFailed", "Failed to load SSL certificates"),
        variant: "destructive",
      });
    } finally {
      setIsLoading(false);
    }
  }, [t, toast]);

  useEffect(() => {
    fetchCertificates();
  }, [fetchCertificates]);

  // Metrics summary
  const metrics = useMemo(() => {
    const total = certificates.length;
    const active = certificates.filter((c) => c.status === "active").length;
    const expiringSoon = certificates.filter(
      (c) => c.status === "active" && c.days_remaining > 0 && c.days_remaining <= 30
    ).length;
    const expiredOrNone = certificates.filter(
      (c) => c.status === "expired" || c.status === "none"
    ).length;

    return { total, active, expiringSoon, expiredOrNone };
  }, [certificates]);

  // Filtered certificates
  const filteredCertificates = useMemo(() => {
    let result = certificates;

    if (statusFilter !== "ALL") {
      result = result.filter((c) => c.status === statusFilter);
    }

    if (searchQuery.trim()) {
      const q = searchQuery.toLowerCase().trim();
      result = result.filter(
        (c) =>
          c.domain_name.toLowerCase().includes(q) ||
          (c.issuer && c.issuer.toLowerCase().includes(q)) ||
          c.subject.toLowerCase().includes(q)
      );
    }

    return result;
  }, [certificates, statusFilter, searchQuery]);

  // Re-issue certificate (DoD)
  const handleReissue = async (cert: SslCertificateItem) => {
    try {
      setReissuingDomainId(cert.domain_id);
      // The web server gets and renews certificates on its own; this reads the state again.
      const fresh = await loadCertificate(cert.domain_id);
      setCertificates((prev) => prev.map((c) => (c.domain_id === cert.domain_id ? fresh : c)));

      toast({
        title: t("ssl.toast.refreshedTitle", "Certificate state refreshed"),
        description: t("ssl.toast.refreshed", "Certificates are renewed by the web server on its own; the state was read again."),
      });
    } catch (err: any) {
      toast({
        title: t("common.error", "Error"),
        description: err?.message || t("ssl.errors.reissueFailed", "Failed to re-issue SSL certificate"),
        variant: "destructive",
      });
    } finally {
      setReissuingDomainId(null);
    }
  };

  // Revoke certificate
  const handleRevoke = async () => {
    if (!revokingTarget) return;
    try {
      const fresh = await removeCustomCertificate(revokingTarget.domain_id);

      setCertificates((prev) => prev.map((c) => (c.domain_id === revokingTarget.domain_id ? fresh : c)));

      toast({
        title: t("common.success", "Success"),
        description: t("ssl.toast.revoked", "Certificate removed and reverted to unencrypted state"),
      });
      setRevokingTarget(null);
    } catch (err: any) {
      toast({
        title: t("common.error", "Error"),
        description: err?.message || t("ssl.errors.revokeFailed", "Failed to revoke certificate"),
        variant: "destructive",
      });
    }
  };

  const renderStatusBadge = (status: SslStatus) => {
    switch (status) {
      case "active":
        return (
          <Badge className="bg-emerald-600 text-white gap-1 text-[11px] font-semibold" data-testid="badge-ssl-status-active">
            <ShieldCheck className="h-3 w-3" />
            {t("ssl.status.active", "Active & Encrypted")}
          </Badge>
        );
      case "expired":
        return (
          <Badge variant="destructive" className="gap-1 text-[11px] font-semibold" data-testid="badge-ssl-status-expired">
            <AlertTriangle className="h-3 w-3" />
            {t("ssl.status.expired", "Expired")}
          </Badge>
        );
      case "self_signed":
        return (
          <Badge variant="secondary" className="bg-amber-500/10 text-amber-600 border-amber-500/20 gap-1 text-[11px]" data-testid="badge-ssl-status-self-signed">
            <ShieldAlert className="h-3 w-3" />
            {t("ssl.status.selfSigned", "Self-Signed")}
          </Badge>
        );
      case "pending":
        return (
          <Badge variant="secondary" className="gap-1 text-[11px] animate-pulse">
            <RefreshCw className="h-3 w-3 animate-spin text-primary" />
            {t("ssl.status.pending", "Issuing...")}
          </Badge>
        );
      default:
        return (
          <Badge variant="outline" className="text-muted-foreground text-[11px]">
            {t("ssl.status.none", "No Certificate")}
          </Badge>
        );
    }
  };

  return (
    <div className="space-y-6" data-testid="ssl-page">
      {/* Page Header */}
      <div className="flex flex-col sm:flex-row sm:items-center sm:justify-between gap-4">
        <div>
          <h1 className="text-2xl font-bold tracking-tight flex items-center gap-2">
            <Lock className="h-6 w-6 text-primary" />
            {t("ssl.title", "SSL / TLS Certificates")}
          </h1>
          <p className="text-sm text-muted-foreground mt-1">
            {t(
              "ssl.subtitle",
              "Manage Let's Encrypt automated issuance, custom commercial certificates, and HTTPS transport policies."
            )}
          </p>
        </div>

        <div className="flex items-center gap-2">
          <Button
            size="sm"
            variant="outline"
            onClick={fetchCertificates}
            disabled={isLoading}
            className="text-xs h-9 gap-1.5"
            data-testid="btn-refresh-ssl"
          >
            <RefreshCw className={`h-4 w-4 ${isLoading ? "animate-spin" : ""}`} />
            {t("common.refresh", "Refresh")}
          </Button>
        </div>
      </div>

      {/* Metric Summary Cards */}
      <div className="grid grid-cols-2 lg:grid-cols-4 gap-3 sm:gap-4">
        <Card className="p-4 flex items-center justify-between">
          <div>
            <p className="text-xs text-muted-foreground uppercase font-semibold">
              {t("ssl.metrics.total", "Total Managed Domains")}
            </p>
            <p className="text-2xl font-bold mt-1">{metrics.total}</p>
          </div>
          <Lock className="h-8 w-8 text-primary/40 shrink-0" />
        </Card>

        <Card className="p-4 flex items-center justify-between">
          <div>
            <p className="text-xs text-muted-foreground uppercase font-semibold">
              {t("ssl.metrics.active", "Active & Encrypted")}
            </p>
            <p className="text-2xl font-bold mt-1 text-emerald-600 dark:text-emerald-400">
              {metrics.active}
            </p>
          </div>
          <ShieldCheck className="h-8 w-8 text-emerald-500/40 shrink-0" />
        </Card>

        <Card className="p-4 flex items-center justify-between">
          <div>
            <p className="text-xs text-muted-foreground uppercase font-semibold">
              {t("ssl.metrics.expiringSoon", "Expiring Soon (< 30d)")}
            </p>
            <p className="text-2xl font-bold mt-1 text-amber-600 dark:text-amber-400">
              {metrics.expiringSoon}
            </p>
          </div>
          <AlertTriangle className="h-8 w-8 text-amber-500/40 shrink-0" />
        </Card>

        <Card className="p-4 flex items-center justify-between">
          <div>
            <p className="text-xs text-muted-foreground uppercase font-semibold">
              {t("ssl.metrics.expiredOrNone", "Expired / Action Needed")}
            </p>
            <p className="text-2xl font-bold mt-1 text-destructive">
              {metrics.expiredOrNone}
            </p>
          </div>
          <ShieldAlert className="h-8 w-8 text-destructive/40 shrink-0" />
        </Card>
      </div>

      {/* Filters Toolbar */}
      <div className="flex flex-col sm:flex-row items-stretch sm:items-center justify-between gap-3">
        {/* Search */}
        <div className="relative w-full sm:w-72">
          <Search className="h-4 w-4 absolute left-3 top-1/2 -translate-y-1/2 text-muted-foreground" />
          <Input
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
            placeholder={t("ssl.searchPlaceholder", "Search domains or issuers...")}
            className="pl-9 h-9 text-xs"
            data-testid="input-search-ssl"
          />
        </div>

        {/* Status Filter */}
        <div className="flex items-center gap-1.5 overflow-x-auto pb-1 sm:pb-0">
          {["ALL", "active", "expired", "self_signed", "none"].map((st) => (
            <Button
              key={st}
              size="sm"
              variant={statusFilter === st ? "default" : "outline"}
              className="h-8 text-xs px-2.5 rounded-full capitalize"
              onClick={() => setStatusFilter(st)}
              data-testid={`btn-filter-ssl-${st}`}
            >
              {st === "ALL" ? t("common.all", "All") : st.replace("_", " ")}
            </Button>
          ))}
        </div>
      </div>

      {/* Domain SSL Cards List */}
      <div className="space-y-4">
        {isLoading ? (
          <Card className="p-10 text-center text-xs text-muted-foreground">
            <RefreshCw className="h-6 w-6 animate-spin mx-auto mb-2 text-primary" />
            {t("common.loading", "Loading SSL certificates...")}
          </Card>
        ) : filteredCertificates.length === 0 ? (
          <Card className="p-10 text-center text-xs text-muted-foreground">
            {t("ssl.empty", "No domain SSL certificates found")}
          </Card>
        ) : (
          filteredCertificates.map((cert) => (
            <Card
              key={cert.id}
              className="border hover:shadow-sm transition-shadow"
              data-testid={`ssl-card-${cert.domain_id}`}
            >
              <CardContent className="p-5 flex flex-col lg:flex-row lg:items-center lg:justify-between gap-4">
                {/* Domain info & Issuer */}
                <div className="space-y-2 max-w-xl">
                  <div className="flex flex-wrap items-center gap-2">
                    <h3 className="font-mono text-base font-bold text-foreground">
                      {cert.domain_name}
                    </h3>
                    <a
                      href={`https://${cert.domain_name}`}
                      target="_blank"
                      rel="noopener noreferrer"
                      className="text-muted-foreground hover:text-foreground"
                      title="Open site"
                    >
                      <ExternalLink className="h-3.5 w-3.5" />
                    </a>
                    {renderStatusBadge(cert.status)}

                    {cert.force_https && (
                      <Badge variant="outline" className="text-[10px] text-emerald-600 border-emerald-500/30 gap-1 h-5">
                        <Lock className="h-3 w-3" />
                        HTTPS Enforced
                      </Badge>
                    )}

                    {cert.hsts_enabled && (
                      <Badge variant="outline" className="text-[10px] text-primary border-primary/30 gap-1 h-5">
                        <Shield className="h-3 w-3" />
                        HSTS Active
                      </Badge>
                    )}
                  </div>

                  <div className="text-xs text-muted-foreground space-y-1">
                    <p>
                      <strong>{t("ssl.fields.issuer", "Issuer")}:</strong>{" "}
                      <span className="font-mono text-foreground">{cert.issuer || "—"}</span>
                    </p>
                    {cert.sans.length > 0 && (
                      <p className="flex items-center gap-1 flex-wrap">
                        <strong>SANs:</strong>
                        {cert.sans.map((san) => (
                          <span
                            key={san}
                            className="inline-block px-1.5 py-0.2 bg-muted rounded font-mono text-[10px]"
                          >
                            {san}
                          </span>
                        ))}
                      </p>
                    )}
                  </div>
                </div>

                {/* Expiry Countdown & Progress */}
                <div className="w-full lg:w-64 space-y-1.5">
                  <div className="flex items-center justify-between text-xs">
                    <span className="text-muted-foreground font-medium">
                      {t("ssl.fields.expires", "Validity")}:
                    </span>
                    <span
                      className={`font-semibold ${
                        cert.days_remaining <= 0
                          ? "text-destructive"
                          : cert.days_remaining <= 30
                          ? "text-amber-600 dark:text-amber-400"
                          : "text-emerald-600 dark:text-emerald-400"
                      }`}
                    >
                      {cert.days_remaining <= 0
                        ? t("ssl.days.expired", "Expired")
                        : t("ssl.days.remaining", "{{days}} days left", { days: cert.days_remaining })}
                    </span>
                  </div>

                  <div className="h-2 w-full bg-secondary rounded-full overflow-hidden">
                    <div
                      className={`h-full transition-all ${
                        cert.days_remaining <= 0
                          ? "bg-destructive w-full"
                          : cert.days_remaining <= 30
                          ? "bg-amber-500 w-1/3"
                          : "bg-emerald-500 w-4/5"
                      }`}
                    />
                  </div>

                  {cert.expires_at && (
                    <p className="text-[11px] text-muted-foreground text-right font-mono">
                      {new Date(cert.expires_at).toLocaleDateString()}
                    </p>
                  )}
                </div>

                {/* Action Buttons */}
                <div className="flex flex-wrap items-center gap-2 pt-2 lg:pt-0 border-t lg:border-t-0">
                  <Button
                    size="sm"
                    variant="outline"
                    onClick={() => handleReissue(cert)}
                    disabled={reissuingDomainId === cert.domain_id}
                    className="h-8 text-xs gap-1.5 text-primary"
                    data-testid={`btn-reissue-ssl-${cert.domain_id}`}
                    title={t("ssl.actions.refresh", "Read the certificate state again")}
                  >
                    <RefreshCw
                      className={`h-3.5 w-3.5 ${
                        reissuingDomainId === cert.domain_id ? "animate-spin" : ""
                      }`}
                    />
                    {reissuingDomainId === cert.domain_id
                      ? t("common.processing", "Checking...")
                      : t("ssl.actions.refresh", "Refresh status")}
                  </Button>

                  <Button
                    size="sm"
                    variant="outline"
                    onClick={() => setCustomUploadTarget(cert)}
                    className="h-8 text-xs gap-1.5"
                    data-testid={`btn-upload-custom-ssl-${cert.domain_id}`}
                    title={t("ssl.actions.uploadCustom", "Upload Custom Certificate")}
                  >
                    <UploadCloud className="h-3.5 w-3.5" />
                    <span className="hidden xl:inline">{t("ssl.actions.custom", "Custom Cert")}</span>
                  </Button>

                  <Button
                    size="sm"
                    variant="outline"
                    onClick={() => setSettingsTarget(cert)}
                    className="h-8 text-xs gap-1.5"
                    data-testid={`btn-settings-ssl-${cert.domain_id}`}
                    title={t("ssl.actions.securitySettings", "HTTPS & HSTS Settings")}
                  >
                    <Settings className="h-3.5 w-3.5" />
                    <span className="hidden xl:inline">{t("ssl.actions.settings", "Settings")}</span>
                  </Button>

                  {cert.type === "custom" && (
                    <Button
                      size="sm"
                      variant="ghost"
                      onClick={() => setRevokingTarget(cert)}
                      className="h-8 w-8 p-0 text-destructive/80 hover:text-destructive hover:bg-destructive/10"
                      data-testid={`btn-revoke-ssl-${cert.domain_id}`}
                      title={t("ssl.actions.revoke", "Use the web server's own certificate")}
                    >
                      <Trash2 className="h-3.5 w-3.5" />
                    </Button>
                  )}
                </div>
              </CardContent>
            </Card>
          ))
        )}
      </div>

      {/* Custom Upload Dialog */}
      {customUploadTarget && (
        <CustomCertUploadDialog
          open={!!customUploadTarget}
          onOpenChange={(open) => !open && setCustomUploadTarget(null)}
          domainId={customUploadTarget.domain_id}
          domainName={customUploadTarget.domain_name}
          onSuccess={(updatedCert) => {
            setCertificates((prev) =>
              prev.map((c) => (c.domain_id === updatedCert.domain_id ? updatedCert : c))
            );
            toast({
              title: t("common.success", "Success"),
              description: t("ssl.toast.customInstalled", "Custom SSL certificate installed successfully"),
            });
          }}
        />
      )}

      {/* Security & HSTS Settings Dialog */}
      {settingsTarget && (
        <SslSettingsDialog
          open={!!settingsTarget}
          onOpenChange={(open) => !open && setSettingsTarget(null)}
          certificate={settingsTarget}
          onSuccess={(updatedCert) => {
            setCertificates((prev) =>
              prev.map((c) => (c.domain_id === updatedCert.domain_id ? updatedCert : c))
            );
            toast({
              title: t("common.success", "Success"),
              description: t("ssl.toast.settingsUpdated", "Security settings saved successfully"),
            });
          }}
        />
      )}

      {/* Revoke Confirmation Dialog */}
      <Dialog open={!!revokingTarget} onOpenChange={(open) => !open && setRevokingTarget(null)}>
        <DialogContent className="max-w-md">
          <DialogHeader>
            <DialogTitle className="text-destructive flex items-center gap-2">
              <AlertTriangle className="h-5 w-5" />
              {t("ssl.revokeDialog.title", "Revoke SSL Certificate")}
            </DialogTitle>
            <DialogDescription className="text-xs pt-1">
              {t(
                "ssl.revokeDialog.warning",
                "Are you sure you want to remove the SSL certificate for {{domain}}? Visitors will see browser security warnings over HTTPS.",
                { domain: revokingTarget?.domain_name }
              )}
            </DialogDescription>
          </DialogHeader>

          <DialogFooter>
            <Button variant="outline" onClick={() => setRevokingTarget(null)}>
              {t("common.cancel", "Cancel")}
            </Button>
            <Button
              variant="destructive"
              onClick={handleRevoke}
              data-testid="btn-confirm-revoke-ssl"
            >
              {t("ssl.actions.revokeConfirm", "Revoke Certificate")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
