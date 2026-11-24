import React, { useState } from "react";
import {
  Card,
  CardHeader,
  CardTitle,
  CardDescription,
  CardContent,
  Button,
  Badge,
  useToast,
} from "@builtpanel/ui";
import { apiFetch } from "@builtpanel/api";
import { useTranslation } from "@builtpanel/i18n";
import {
  RotateCcw,
  CheckCircle2,
  AlertTriangle,
  Clock,
  HardDrive,
  Cpu,
  Server,
  Zap,
} from "lucide-react";
import type { ServiceItem, RestartResponse, ServiceRateLimitError } from "./types";

interface ServiceCardProps {
  service: ServiceItem;
  onRestarted: () => void;
}

export function ServiceCard({ service, onRestarted }: ServiceCardProps) {
  const { t } = useTranslation();
  const { toast } = useToast();

  const [isRestarting, setIsRestarting] = useState(false);
  const [rateLimitError, setRateLimitError] = useState<ServiceRateLimitError | null>(null);

  const formatUptime = (seconds: number) => {
    if (seconds < 60) return `${seconds}s`;
    const mins = Math.floor(seconds / 60);
    if (mins < 60) return `${mins}m`;
    const hours = Math.floor(mins / 60);
    const remMins = mins % 60;
    return `${hours}h ${remMins}m`;
  };

  const handleRestart = async () => {
    try {
      setIsRestarting(true);
      setRateLimitError(null);
      const res = await apiFetch<RestartResponse>(
        `/api/v1/client/services/${service.name}/restart`,
        { method: "POST" }
      );
      toast({
        title: t("common.success", "Success"),
        description: res.message,
      });
      onRestarted();
    } catch (err: any) {
      if (err?.status === 429 || err?.error === "rate_limit_exceeded") {
        setRateLimitError({
          error: "rate_limit_exceeded",
          message: err?.message || t("services.errors.rateLimited", "Too many restart requests. You can restart this service at most 3 times per minute."),
          retry_after_seconds: err?.retry_after_seconds || 60,
        });
      }
      toast({
        title: t("common.error", "Error"),
        description: err?.message || t("services.errors.restartFailed", "Failed to restart service"),
        variant: "destructive",
      });
    } finally {
      setIsRestarting(false);
    }
  };

  const remainingRestarts = Math.max(0, service.restarts_limit_per_minute - service.restarts_count_this_minute);

  return (
    <Card className="border shadow-sm" data-testid={`service-card-${service.name}`}>
      <CardHeader className="pb-3">
        <div className="flex items-start justify-between gap-3">
          <div className="space-y-1">
            <CardTitle className="text-base flex items-center gap-2">
              <Server className="h-5 w-5 text-primary" />
              {service.display_name}
            </CardTitle>
            <CardDescription className="text-xs">
              {service.description}
            </CardDescription>
          </div>

          <Badge className="bg-emerald-600 text-white gap-1 text-[11px] font-semibold" data-testid={`badge-status-${service.name}`}>
            <CheckCircle2 className="h-3 w-3" />
            {service.status.toUpperCase()}
          </Badge>
        </div>
      </CardHeader>

      <CardContent className="space-y-4">
        {/* Real-time Metrics Grid */}
        <div className="grid grid-cols-2 sm:grid-cols-4 gap-2.5 p-3 bg-muted/40 rounded-lg border text-xs">
          <div>
            <span className="text-[10px] text-muted-foreground uppercase font-semibold flex items-center gap-1">
              <Clock className="h-3 w-3 text-primary" />
              Uptime
            </span>
            <p className="font-mono font-bold mt-0.5 text-foreground">
              {formatUptime(service.uptime_seconds)}
            </p>
          </div>

          <div>
            <span className="text-[10px] text-muted-foreground uppercase font-semibold flex items-center gap-1">
              <HardDrive className="h-3 w-3 text-emerald-500" />
              Memory
            </span>
            <p className="font-mono font-bold mt-0.5 text-foreground">
              {service.memory_usage_mb} MB
            </p>
          </div>

          <div>
            <span className="text-[10px] text-muted-foreground uppercase font-semibold flex items-center gap-1">
              <Cpu className="h-3 w-3 text-primary" />
              CPU Load
            </span>
            <p className="font-mono font-bold mt-0.5 text-foreground">
              {service.cpu_percent}%
            </p>
          </div>

          <div>
            <span className="text-[10px] text-muted-foreground uppercase font-semibold flex items-center gap-1">
              <Zap className="h-3 w-3 text-amber-500" />
              Workers
            </span>
            <p className="font-mono font-bold mt-0.5 text-foreground">
              {service.workers_count} procs
            </p>
          </div>
        </div>

        {/* Rate limit status & warning banner */}
        {rateLimitError ? (
          <div
            className="p-3 bg-destructive/10 border border-destructive/20 text-destructive text-xs rounded-lg flex items-center gap-2"
            data-testid={`banner-rate-limit-${service.name}`}
          >
            <AlertTriangle className="h-4 w-4 shrink-0" />
            <span>
              {rateLimitError.message} (Cooldown: {rateLimitError.retry_after_seconds}s)
            </span>
          </div>
        ) : (
          <div className="flex items-center justify-between text-[11px] text-muted-foreground px-1">
            <span>Rate limit quota:</span>
            <span
              className={`font-semibold font-mono ${
                remainingRestarts === 0 ? "text-amber-600 dark:text-amber-400" : "text-foreground"
              }`}
              data-testid={`quota-restarts-${service.name}`}
            >
              {service.restarts_count_this_minute} / {service.restarts_limit_per_minute} restarts this minute
            </span>
          </div>
        )}

        {/* Action Button */}
        <div className="flex items-center justify-between pt-2 border-t">
          <span className="text-[11px] text-muted-foreground font-mono">
            Last restarted: {new Date(service.last_restarted_at).toLocaleTimeString()}
          </span>

          <Button
            size="sm"
            variant="outline"
            onClick={handleRestart}
            disabled={isRestarting}
            className="h-8 text-xs gap-1.5"
            data-testid={`btn-restart-${service.name}`}
            title={`Restart ${service.display_name}`}
          >
            <RotateCcw className={`h-3.5 w-3.5 ${isRestarting ? "animate-spin text-primary" : ""}`} />
            {isRestarting ? t("common.processing", "Restarting...") : t("services.actions.restart", "Restart Service")}
          </Button>
        </div>
      </CardContent>
    </Card>
  );
}
