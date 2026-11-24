import React, { useCallback, useEffect, useState } from "react";
import { apiFetch, untilAnswered } from "@builtpanel/api";
import { Badge, Button, Card, CardContent, CardHeader, CardTitle } from "@builtpanel/ui";
import { useTranslation } from "@builtpanel/i18n";

interface WafEvent {
  at_ms: number;
  method: string;
  host: string;
  uri: string;
  rule_id: number;
  message: string;
  blocked: boolean;
}
interface WafEventList {
  range: string;
  available: boolean;
  items: WafEvent[];
  partial: boolean;
}

/** The requests the firewall refused for this account's sites, newest first. */
export function WafEventsCard() {
  const { t } = useTranslation();
  const [range, setRange] = useState("24h");
  const [data, setData] = useState<WafEventList | null>(null);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      setError(null);
      const answer = await untilAnswered(() => apiFetch<WafEventList>("/api/v1/client/waf/events", { params: { range, limit: 100 } }));
      setData({ ...answer, items: answer.items ?? [] });
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, [range]);

  useEffect(() => {
    void load();
  }, [load]);

  return (
    <Card data-testid="waf-events-card">
      <CardHeader className="flex flex-row items-center justify-between space-y-0">
        <CardTitle>{t("waf.events", "Refused requests")}</CardTitle>
        <div className="flex items-center gap-2">
          <select
            className="rounded border bg-background px-2 py-1 text-xs"
            value={range}
            onChange={(e) => setRange(e.target.value)}
            aria-label={t("waf.range", "Time range")}
          >
            <option value="24h">24h</option>
            <option value="7d">7d</option>
            <option value="30d">30d</option>
          </select>
          <Button size="sm" variant="outline" onClick={() => void load()}>
            {t("common.refresh", "Refresh")}
          </Button>
        </div>
      </CardHeader>
      <CardContent className="space-y-1">
        {error && <p className="text-sm text-destructive">{error}</p>}
        {data && !data.available && (
          <p className="text-sm text-muted-foreground">
            {t("waf.eventsUnavailable", "This server keeps no record of refused requests (yet): the firewall's log is not there.")}
          </p>
        )}
        {data?.available && data.items.length === 0 && (
          <p className="text-sm text-muted-foreground">{t("waf.noEvents", "Nothing was refused in this time.")}</p>
        )}
        {data?.items.map((e, i) => (
          <div key={`${e.at_ms}-${i}`} className="flex flex-wrap items-center justify-between gap-2 border-b py-1 text-sm">
            <span className="min-w-0 break-all">
              <Badge variant={e.blocked ? "destructive" : "outline"} className="mr-1">
                {e.blocked ? t("waf.refused", "refused") : t("waf.noticed", "noticed")}
              </Badge>
              <span className="font-mono text-xs">{e.method} {e.host}{e.uri}</span>
              <span className="ml-2 text-xs text-muted-foreground">
                {e.rule_id ? `#${e.rule_id} ` : ""}
                {e.message}
              </span>
            </span>
            <span className="text-xs text-muted-foreground">{new Date(e.at_ms).toLocaleString()}</span>
          </div>
        ))}
        {data?.partial && (
          <p className="text-xs text-muted-foreground">{t("waf.eventsPartial", "Only the newest are shown.")}</p>
        )}
      </CardContent>
    </Card>
  );
}
