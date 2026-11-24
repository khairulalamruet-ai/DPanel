import React, { useState, useEffect, useCallback } from "react";
import { Card, Button, useToast } from "@builtpanel/ui";
import { apiFetch } from "@builtpanel/api";
import { useTranslation } from "@builtpanel/i18n";
import { Server, RefreshCw, ShieldAlert, Activity } from "lucide-react";
import type { ServiceItem } from "./types";
import { ServiceCard } from "./ServiceCard";

export function ServicesPage() {
  const { t } = useTranslation();
  const { toast } = useToast();

  const [services, setServices] = useState<ServiceItem[]>([]);
  const [isLoading, setIsLoading] = useState(true);

  const fetchServices = useCallback(async () => {
    try {
      setIsLoading(true);
      const res = await apiFetch<{ items: ServiceItem[]; total: number }>(
        "/api/v1/client/services"
      );
      setServices(res.items);
    } catch {
      toast({
        title: t("common.error", "Error"),
        description: t("services.errors.fetchFailed", "Failed to load web services status"),
        variant: "destructive",
      });
    } finally {
      setIsLoading(false);
    }
  }, [t, toast]);

  useEffect(() => {
    fetchServices();
  }, [fetchServices]);

  return (
    <div className="space-y-6" data-testid="services-page">
      {/* Page Header */}
      <div className="flex flex-col sm:flex-row sm:items-center sm:justify-between gap-4">
        <div>
          <h1 className="text-2xl font-bold tracking-tight flex items-center gap-2">
            <Activity className="h-6 w-6 text-primary" />
            {t("services.title", "Web Services Management")}
          </h1>
          <p className="text-sm text-muted-foreground mt-1">
            {t(
              "services.subtitle",
              "Monitor daemon process health, inspect runtime metrics, and safely trigger zero-downtime restarts."
            )}
          </p>
        </div>

        <div className="flex items-center gap-2">
          <Button
            size="sm"
            variant="outline"
            onClick={fetchServices}
            disabled={isLoading}
            className="text-xs h-9 gap-1.5"
            data-testid="btn-refresh-services"
          >
            <RefreshCw className={`h-4 w-4 ${isLoading ? "animate-spin" : ""}`} />
            {t("common.refresh", "Refresh")}
          </Button>
        </div>
      </div>

      {/* Security & Rate Limiting Policy Banner */}
      <div className="p-3.5 bg-muted/40 border rounded-lg text-xs flex items-start gap-3">
        <ShieldAlert className="h-4 w-4 text-primary shrink-0 mt-0.5" />
        <div className="space-y-0.5">
          <p className="font-semibold text-foreground">
            {t("services.rateLimitTitle", "Service Restart Rate Limit Policy")}
          </p>
          <p className="text-muted-foreground leading-relaxed">
            {t(
              "services.rateLimitDesc",
              "Each web service (Nginx, PHP-FPM) can be restarted at most 3 times per minute. Further attempts will be blocked with HTTP 429 Too Many Requests until the cooldown period elapses."
            )}
          </p>
        </div>
      </div>

      {/* Services Grid */}
      <div className="grid grid-cols-1 md:grid-cols-2 gap-5">
        {isLoading ? (
          <Card className="col-span-2 p-12 text-center text-xs text-muted-foreground">
            <RefreshCw className="h-6 w-6 animate-spin mx-auto mb-2 text-primary" />
            {t("common.loading", "Loading web services status...")}
          </Card>
        ) : services.length === 0 ? (
          <Card className="col-span-2 p-12 text-center text-xs text-muted-foreground">
            <Server className="h-8 w-8 mx-auto mb-2 text-muted-foreground/40" />
            No active web services found.
          </Card>
        ) : (
          services.map((srv) => (
            <ServiceCard key={srv.id} service={srv} onRestarted={fetchServices} />
          ))
        )}
      </div>
    </div>
  );
}
