import React, { useEffect, useState } from "react";
import { api } from "@builtpanel/api";
import { Card, CardHeader, CardTitle, CardContent, Badge } from "@builtpanel/ui";
import { useTranslation } from "@builtpanel/i18n";
import { Activity, CheckCircle2, AlertCircle } from "lucide-react";

interface HealthState {
  status: string;
  version: string;
  timestamp?: string;
}

export function HealthCard() {
  const { t } = useTranslation();
  const [health, setHealth] = useState<HealthState | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    // Typed API call via openapi-fetch
    api.GET("/api/v1/public/health")
      .then(({ data, error: apiErr }) => {
        if (data) {
          setHealth(data);
        } else if (apiErr) {
          setError((apiErr as any).message || "Degraded service");
        }
        setLoading(false);
      })
      .catch((err) => {
        setError(err.message || "Failed to check health");
        setLoading(false);
      });
  }, []);

  return (
    <Card className="max-w-md">
      <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
        <CardTitle className="text-sm font-medium flex items-center gap-2">
          <Activity className="h-4 w-4 text-primary" />
          {t("common.healthStatus")}
        </CardTitle>
        {health?.status === "ok" ? (
          <Badge variant="success" className="gap-1">
            <CheckCircle2 className="h-3 w-3" />
            {t("common.ok")}
          </Badge>
        ) : (
          <Badge variant="destructive" className="gap-1">
            <AlertCircle className="h-3 w-3" />
            {error || "Degraded"}
          </Badge>
        )}
      </CardHeader>
      <CardContent>
        {loading ? (
          <p className="text-sm text-muted-foreground">{t("common.loading")}</p>
        ) : (
          <div className="space-y-1 text-sm">
            <div className="flex justify-between">
              <span className="text-muted-foreground">{t("common.status")}:</span>
              <span className="font-medium uppercase">{health?.status || "Unknown"}</span>
            </div>
            <div className="flex justify-between">
              <span className="text-muted-foreground">{t("common.version")}:</span>
              <span className="font-mono text-xs">{health?.version || "N/A"}</span>
            </div>
          </div>
        )}
      </CardContent>
    </Card>
  );
}
