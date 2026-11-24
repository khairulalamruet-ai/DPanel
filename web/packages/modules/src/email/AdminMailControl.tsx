import React, { useCallback, useEffect, useState } from "react";
import { apiFetch } from "@builtpanel/api";
import { Card, CardHeader, CardTitle, CardContent, Button, Badge, Input, useToast } from "@builtpanel/ui";
import { useTranslation } from "@builtpanel/i18n";

interface MailDomain {
  id: string;
  domain: string;
  state: string;
  held: boolean;
  held_reason?: string | null;
  error?: string | null;
}
interface MailServer {
  node_id: string;
  hostname: string;
  applied_hostname?: string | null;
  error?: string | null;
}

/** Mail domains (hold or release their outgoing mail) and the name of each node's mail server. */
export function AdminMailControl() {
  const { t } = useTranslation();
  const { toast } = useToast();
  const [domains, setDomains] = useState<MailDomain[]>([]);
  const [servers, setServers] = useState<MailServer[]>([]);
  const [names, setNames] = useState<Record<string, string>>({});

  const load = useCallback(async () => {
    try {
      const [d, s] = await Promise.all([
        apiFetch<MailDomain[]>("/api/v1/admin/email/domains"),
        apiFetch<MailServer[]>("/api/v1/admin/email/servers"),
      ]);
      setDomains(d);
      setServers(s);
    } catch (e) {
      toast({ title: e instanceof Error ? e.message : String(e), variant: "destructive" });
    }
  }, [toast]);

  useEffect(() => {
    void load();
  }, [load]);

  const run = async (fn: () => Promise<unknown>) => {
    try {
      await fn();
      await load();
    } catch (e) {
      toast({ title: e instanceof Error ? e.message : String(e), variant: "destructive" });
    }
  };

  const hold = (d: MailDomain) => {
    const reason = window.prompt(t("email.holdReason", "Why is this domain held?"));
    if (!reason) return;
    void run(() =>
      apiFetch(`/api/v1/admin/email/domains/${d.id}/hold`, { method: "POST", body: JSON.stringify({ reason }) }),
    );
  };

  return (
    <div className="grid gap-4 lg:grid-cols-2" data-testid="admin-mail-control">
      <Card>
        <CardHeader>
          <CardTitle>{t("email.mailDomains", "Mail domains")}</CardTitle>
        </CardHeader>
        <CardContent className="space-y-1">
          {domains.map((d) => (
            <div key={d.id} className="flex items-center justify-between gap-2 border-b py-1 text-sm">
              <span>
                {d.domain} <Badge variant="outline">{d.held ? t("email.held", "held") : d.state}</Badge>
                {d.held_reason && <span className="ml-1 text-xs text-muted-foreground">{d.held_reason}</span>}
              </span>
              {d.held ? (
                <Button size="sm" onClick={() => run(() => apiFetch(`/api/v1/admin/email/domains/${d.id}/release`, { method: "POST" }))}>
                  {t("email.release", "Release")}
                </Button>
              ) : (
                <Button size="sm" variant="outline" onClick={() => hold(d)}>
                  {t("email.hold", "Hold")}
                </Button>
              )}
            </div>
          ))}
        </CardContent>
      </Card>
      <Card>
        <CardHeader>
          <CardTitle>{t("email.mailServers", "Mail servers")}</CardTitle>
        </CardHeader>
        <CardContent className="space-y-2">
          {servers.map((s) => (
            <div key={s.node_id} className="space-y-1 border-b pb-2 text-sm">
              <div className="text-xs text-muted-foreground">
                {s.node_id} · {s.applied_hostname ?? "-"}
                {s.error && <span className="text-destructive"> · {s.error}</span>}
              </div>
              <form
                className="flex gap-2"
                onSubmit={(e) => {
                  e.preventDefault();
                  const hostname = names[s.node_id] ?? s.hostname;
                  void run(() =>
                    apiFetch(`/api/v1/admin/email/servers/${s.node_id}`, { method: "PUT", body: JSON.stringify({ hostname }) }),
                  );
                }}
              >
                <Input value={names[s.node_id] ?? s.hostname} onChange={(e) => setNames({ ...names, [s.node_id]: e.target.value })} />
                <Button type="submit" size="sm">{t("common.save", "Save")}</Button>
              </form>
            </div>
          ))}
        </CardContent>
      </Card>
    </div>
  );
}
