import React, { useState, useEffect, useCallback } from "react";
import { useTranslation } from "@builtpanel/i18n";
import { apiFetch } from "@builtpanel/api";
import { UpdateSystemCard } from "./UpdateSystemCard";
import { UpdateHistoryTable } from "./UpdateHistoryTable";
import { PanelUpdateStatus } from "./types";

export function UpdatePage() {
  const { t } = useTranslation();
  const [status, setStatus] = useState<PanelUpdateStatus | null>(null);
  const [error, setError] = useState<string | null>(null);

  const loadStatus = useCallback(async () => {
    try {
      const res = await apiFetch<PanelUpdateStatus>("/api/v1/admin/update/status");
      setStatus(res);
      setError(null);
    } catch (err: any) {
      // While the panel restarts into a new version the request fails for a few seconds.
      setError(err?.message || t("update.loadError", "The update status could not be loaded."));
    }
  }, [t]);

  useEffect(() => {
    void loadStatus();
  }, [loadStatus]);

  return (
    <div className="space-y-6" data-testid="panel-update-page">
      <div>
        <h1 className="text-2xl font-bold tracking-tight">{t("update.pageTitle", "Panel Update & Releases")}</h1>
        <p className="text-sm text-muted-foreground">
          {t("update.pageSubtitle", "See which version runs, and install a newer release with one click.")}
        </p>
      </div>

      {status ? (
        <>
          <UpdateSystemCard status={status} onRefresh={loadStatus} />
          <UpdateHistoryTable history={status.history} />
        </>
      ) : (
        <p className="text-sm text-muted-foreground" data-testid="update-loading">
          {error ?? t("common.loading", "Loading...")}
        </p>
      )}
    </div>
  );
}
