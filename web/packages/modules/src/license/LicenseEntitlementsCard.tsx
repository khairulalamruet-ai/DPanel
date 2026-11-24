import React from "react";
import { useTranslation } from "@builtpanel/i18n";
import { Card, CardHeader, CardTitle, CardDescription, CardContent, Badge } from "@builtpanel/ui";
import {
  CheckCircle2,
  XCircle,
  Server,
  Users,
  Globe,
  Layers,
  ShieldCheck,
  HardDrive,
  Cpu,
  Sparkles,
} from "lucide-react";
import { LicenseFeatures, LicenseUsage } from "./types";

interface LicenseEntitlementsCardProps {
  features: LicenseFeatures;
  usage: LicenseUsage;
}

export function LicenseEntitlementsCard({ features, usage }: LicenseEntitlementsCardProps) {
  const { t } = useTranslation();

  const renderLimitBar = (
    label: string,
    current: number,
    max: number,
    icon: React.ReactNode,
    testId: string
  ) => {
    const isUnlimited = max <= 0 || max >= 99999;
    const percent = isUnlimited ? 0 : Math.min(100, Math.round((current / max) * 100));

    return (
      <div className="p-3.5 rounded-lg border bg-card space-y-2" data-testid={testId}>
        <div className="flex items-center justify-between text-xs font-semibold">
          <span className="flex items-center gap-1.5 text-foreground">
            {icon}
            {label}
          </span>
          <span className="font-mono text-xs">
            <span className="font-bold text-primary">{current}</span>
            <span className="text-muted-foreground"> / {isUnlimited ? "Unlimited" : max}</span>
          </span>
        </div>

        {!isUnlimited && (
          <div className="w-full bg-muted rounded-full h-1.5 overflow-hidden">
            <div
              className={`h-full rounded-full transition-all duration-300 ${
                percent >= 90 ? "bg-destructive" : percent >= 75 ? "bg-amber-500" : "bg-primary"
              }`}
              style={{ width: `${percent}%` }}
            />
          </div>
        )}
      </div>
    );
  };

  const featureList = [
    {
      key: "reseller_enabled",
      title: t("license.featReseller", "Reseller Hosting System"),
      enabled: features.reseller_enabled,
      desc: t("license.featResellerDesc", "Multi-tenant reseller accounts, sub-packages, and custom quotas."),
    },
    {
      key: "clustering_enabled",
      title: t("license.featClustering", "Multi-Node Clustering"),
      enabled: features.clustering_enabled,
      desc: t("license.featClusteringDesc", "Distributed worker nodes, placement scheduler, and PowerDNS cluster."),
    },
    {
      key: "advanced_backups",
      title: t("license.featBackups", "Off-site & S3 Cloud Backups"),
      enabled: features.advanced_backups,
      desc: t("license.featBackupsDesc", "Automated remote storage to AWS S3, Wasabi, and SFTP endpoints."),
    },
    {
      key: "commercial_waf",
      title: t("license.featWaf", "Commercial ModSecurity WAF"),
      enabled: features.commercial_waf,
      desc: t("license.featWafDesc", "OWASP CRS rule updates and virtual patching engine."),
    },
    {
      key: "whitelabel_enabled",
      title: t("license.featWhitelabel", "White-Label Branding"),
      enabled: features.whitelabel_enabled,
      desc: t("license.featWhitelabelDesc", "Custom logos, accent colors, custom login titles, and client subdomains."),
    },
    {
      key: "api_access",
      title: t("license.featApi", "REST API & Automation Webhooks"),
      enabled: features.api_access,
      desc: t("license.featApiDesc", "WHMCS / Blesta billing integration and API tokens."),
    },
  ];

  return (
    <Card className="border shadow-xs" data-testid="license-entitlements-card">
      <CardHeader className="pb-3">
        <CardTitle className="text-base font-bold flex items-center gap-2">
          <Layers className="h-4 w-4 text-primary" />
          <span>{t("license.entitlementsTitle", "Licensed Entitlements & Features")}</span>
        </CardTitle>
        <CardDescription className="text-xs">
          {t(
            "license.entitlementsSubtitle",
            "Allocated capacity quotas and signed module feature gates for this installation."
          )}
        </CardDescription>
      </CardHeader>

      <CardContent className="space-y-4">
        {/* Resource Usage & Quota Bars */}
        <div className="grid grid-cols-1 sm:grid-cols-3 gap-3">
          {renderLimitBar(
            t("license.managedNodes", "Managed Nodes"),
            usage.nodes_used,
            features.max_nodes,
            <Server className="h-4 w-4 text-primary" />,
            "quota-bar-nodes"
          )}

          {renderLimitBar(
            t("license.tenantAccounts", "Tenant Accounts"),
            usage.tenants_used,
            features.max_tenants,
            <Users className="h-4 w-4 text-primary" />,
            "quota-bar-tenants"
          )}

          {renderLimitBar(
            t("license.hostedDomains", "Hosted Domains"),
            usage.domains_used,
            features.max_domains,
            <Globe className="h-4 w-4 text-primary" />,
            "quota-bar-domains"
          )}
        </div>

        {/* Feature Gates Grid */}
        <div className="pt-2 border-t">
          <h4 className="text-xs font-semibold text-muted-foreground uppercase tracking-wider mb-3">
            {t("license.licensedModules", "Signed Module Capabilities")}
          </h4>

          <div className="grid grid-cols-1 md:grid-cols-2 gap-3" data-testid="license-features-grid">
            {featureList.map((feat) => (
              <div
                key={feat.key}
                className="p-3 rounded-lg border bg-muted/15 flex items-start gap-3"
                data-testid={`feature-item-${feat.key}`}
              >
                <div className="mt-0.5">
                  {feat.enabled ? (
                    <CheckCircle2 className="h-4 w-4 text-emerald-500 shrink-0" />
                  ) : (
                    <XCircle className="h-4 w-4 text-muted-foreground shrink-0" />
                  )}
                </div>
                <div className="space-y-0.5 flex-1 min-w-0">
                  <div className="flex items-center justify-between">
                    <span className="text-xs font-bold text-foreground truncate">{feat.title}</span>
                    <Badge
                      variant={feat.enabled ? "success" : "secondary"}
                      className="text-[10px] font-mono uppercase"
                    >
                      {feat.enabled ? t("common.enabled", "Active") : t("common.locked", "Locked")}
                    </Badge>
                  </div>
                  <p className="text-[11px] text-muted-foreground leading-relaxed">{feat.desc}</p>
                </div>
              </div>
            ))}
          </div>
        </div>
      </CardContent>
    </Card>
  );
}
