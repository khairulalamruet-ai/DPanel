import React, { useCallback, useEffect, useState } from "react";
import { Card, CardHeader, CardTitle, CardContent, Button, Badge, Input, useToast } from "@builtpanel/ui";
import { useTranslation } from "@builtpanel/i18n";
import * as tools from "./api";

type Tab = "cache" | "deploys" | "scans" | "traffic";

function fmtTime(sec?: number | null): string {
  return sec ? new Date(sec * 1000).toLocaleString() : "-";
}

function useLoad<T>(load: () => Promise<T>) {
  const [data, setData] = useState<T | null>(null);
  const [error, setError] = useState<string | null>(null);
  const reload = useCallback(async () => {
    try {
      setData(await load());
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, [load]);
  useEffect(() => {
    void reload();
  }, [reload]);
  return { data, error, reload };
}

function CacheTab() {
  const { t } = useTranslation();
  const { toast } = useToast();
  const { data, error, reload } = useLoad(tools.listCache);
  const [memory, setMemory] = useState("64");

  const run = async (fn: () => Promise<unknown>, ok: string) => {
    try {
      await fn();
      toast({ title: ok });
      await reload();
    } catch (e) {
      toast({ title: e instanceof Error ? e.message : String(e), variant: "destructive" });
    }
  };

  return (
    <Card>
      <CardHeader>
        <CardTitle>{t("tools.cache", "Cache servers")}</CardTitle>
      </CardHeader>
      <CardContent className="space-y-3">
        {error && <p className="text-sm text-destructive">{error}</p>}
        {data?.items.map((c) => (
          <div key={c.kind} className="flex flex-wrap items-center justify-between gap-2 rounded border p-3">
            <div>
              <div className="font-medium capitalize">
                {c.kind} <Badge variant="outline">{c.status}</Badge>
              </div>
              <div className="text-xs text-muted-foreground">
                {c.host ? `${c.host}:${c.port ?? ""}` : ""} · {c.memory_mb} MB
                {c.password ? ` · ${t("tools.password", "password")}: ${c.password}` : ""}
              </div>
              {c.error && <div className="text-xs text-destructive">{c.error}</div>}
            </div>
            <Button size="sm" variant="outline" onClick={() => run(() => tools.deleteCache(c.kind), t("tools.removed", "Removed"))}>
              {t("common.delete", "Delete")}
            </Button>
          </div>
        ))}
        {data && data.items.length === 0 && <p className="text-sm text-muted-foreground">{t("tools.noCache", "No cache server yet.")}</p>}
        <div className="flex flex-wrap items-end gap-2">
          <Input className="w-28" type="number" value={memory} onChange={(e) => setMemory(e.target.value)} aria-label="MB" />
          {["redis", "memcached"].map((k) => (
            <Button key={k} size="sm" onClick={() => run(() => tools.createCache(k, Number(memory) || 64), t("tools.queued", "Queued"))}>
              + {k}
            </Button>
          ))}
        </div>
        {data?.limit_mb != null && (
          <p className="text-xs text-muted-foreground">{t("tools.limit", "Plan limit")}: {data.limit_mb} MB</p>
        )}
      </CardContent>
    </Card>
  );
}

function DeploysTab() {
  const { t } = useTranslation();
  const { toast } = useToast();
  const { data, error, reload } = useLoad(tools.listDeploys);
  const [form, setForm] = useState({ name: "", repo_url: "", branch: "main", dest: "public_html", token: "" });

  const run = async (fn: () => Promise<unknown>, ok: string) => {
    try {
      await fn();
      toast({ title: ok });
      await reload();
    } catch (e) {
      toast({ title: e instanceof Error ? e.message : String(e), variant: "destructive" });
    }
  };

  return (
    <Card>
      <CardHeader>
        <CardTitle>{t("tools.deploys", "Git deploys")}</CardTitle>
      </CardHeader>
      <CardContent className="space-y-3">
        {error && <p className="text-sm text-destructive">{error}</p>}
        {data?.map((d) => (
          <div key={d.id} className="space-y-1 rounded border p-3">
            <div className="flex flex-wrap items-center justify-between gap-2">
              <div className="font-medium">
                {d.name} <Badge variant="outline">{d.status}</Badge>
              </div>
              <div className="flex gap-2">
                <Button size="sm" onClick={() => run(() => tools.deployNow(d.id), t("tools.queued", "Queued"))}>
                  {t("tools.deployNow", "Deploy now")}
                </Button>
                <Button size="sm" variant="outline" onClick={() => run(() => tools.newWebhookSecret(d.id), t("tools.secretNew", "New webhook secret made"))}>
                  {t("tools.newSecret", "New secret")}
                </Button>
                <Button size="sm" variant="outline" onClick={() => run(() => tools.deleteDeploy(d.id), t("tools.removed", "Removed"))}>
                  {t("common.delete", "Delete")}
                </Button>
              </div>
            </div>
            <div className="text-xs text-muted-foreground">
              {d.repo_url} @ {d.branch} → {d.dest} · {fmtTime(d.last_deployed_at)}
            </div>
            <div className="text-xs text-muted-foreground">Webhook: {d.webhook_path}</div>
            {d.webhook_secret && <div className="break-all text-xs">Secret: {d.webhook_secret}</div>}
            {d.last_error && <div className="text-xs text-destructive">{d.last_error}</div>}
          </div>
        ))}
        <form
          className="grid gap-2 sm:grid-cols-2"
          onSubmit={(e) => {
            e.preventDefault();
            void run(() => tools.createDeploy(form), t("tools.queued", "Queued"));
          }}
        >
          <Input placeholder="name" value={form.name} onChange={(e) => setForm({ ...form, name: e.target.value })} required />
          <Input placeholder="https://github.com/you/repo.git" value={form.repo_url} onChange={(e) => setForm({ ...form, repo_url: e.target.value })} required />
          <Input placeholder="branch" value={form.branch} onChange={(e) => setForm({ ...form, branch: e.target.value })} />
          <Input placeholder="public_html" value={form.dest} onChange={(e) => setForm({ ...form, dest: e.target.value })} required />
          <Input type="password" placeholder={t("tools.token", "access token (private repo)")} value={form.token} onChange={(e) => setForm({ ...form, token: e.target.value })} />
          <Button type="submit">{t("tools.addDeploy", "Add deploy")}</Button>
        </form>
      </CardContent>
    </Card>
  );
}

function ScansTab() {
  const { t } = useTranslation();
  const { toast } = useToast();
  const malware = useLoad(tools.listMalwareScans);
  const vuln = useLoad(tools.listVulnScans);

  const start = async (fn: () => Promise<unknown>, reload: () => Promise<void>) => {
    try {
      await fn();
      toast({ title: t("tools.queued", "Queued") });
      await reload();
    } catch (e) {
      toast({ title: e instanceof Error ? e.message : String(e), variant: "destructive" });
    }
  };

  return (
    <div className="grid gap-4 lg:grid-cols-2">
      <Card>
        <CardHeader>
          <CardTitle>{t("tools.malware", "Malware scan")}</CardTitle>
        </CardHeader>
        <CardContent className="space-y-2">
          <div className="flex gap-2">
            <Button size="sm" onClick={() => start(() => tools.startMalwareScan(false), malware.reload)}>{t("tools.scan", "Scan")}</Button>
            <Button size="sm" variant="outline" onClick={() => start(() => tools.startMalwareScan(true), malware.reload)}>{t("tools.scanQuarantine", "Scan and quarantine")}</Button>
            <Button size="sm" variant="ghost" onClick={() => void malware.reload()}>{t("common.refresh", "Refresh")}</Button>
          </div>
          {malware.error && <p className="text-sm text-destructive">{malware.error}</p>}
          {malware.data?.slice(0, 5).map((s) => (
            <div key={s.id} className="rounded border p-2 text-sm">
              <Badge variant="outline">{s.state}</Badge> {fmtTime(s.requested_at)} · {s.scanned ?? 0} files
              {s.error && <div className="text-xs text-destructive">{s.error}</div>}
              {s.findings.map((f) => (
                <div key={f.path} className="text-xs">{f.path} — {f.signature}{f.quarantined ? " (quarantined)" : ""}</div>
              ))}
            </div>
          ))}
        </CardContent>
      </Card>
      <Card>
        <CardHeader>
          <CardTitle>{t("tools.vuln", "WordPress vulnerabilities")}</CardTitle>
        </CardHeader>
        <CardContent className="space-y-2">
          <div className="flex gap-2">
            <Button size="sm" onClick={() => start(() => tools.startVulnScan(), vuln.reload)}>{t("tools.scan", "Scan")}</Button>
            <Button size="sm" variant="ghost" onClick={() => void vuln.reload()}>{t("common.refresh", "Refresh")}</Button>
          </div>
          {vuln.error && <p className="text-sm text-destructive">{vuln.error}</p>}
          {vuln.data?.slice(0, 5).map((s) => (
            <div key={s.id} className="rounded border p-2 text-sm">
              <Badge variant="outline">{s.state}</Badge> {fmtTime(s.requested_at)}
              {s.error && <div className="text-xs text-destructive">{s.error}</div>}
              {s.issues.map((i, n) => (
                <div key={n} className="text-xs">{i.severity}: {i.title} ({i.component} {i.version} → {i.fixed_in})</div>
              ))}
            </div>
          ))}
        </CardContent>
      </Card>
    </div>
  );
}

function TrafficTab() {
  const { t } = useTranslation();
  const { data, error } = useLoad(tools.getTraffic);
  const cells: [string, number | undefined][] = [
    ["Requests", data?.requests],
    ["2xx", data?.status_2xx],
    ["3xx", data?.status_3xx],
    ["4xx", data?.status_4xx],
    ["5xx", data?.status_5xx],
    ["Bots", data?.bots],
  ];
  return (
    <Card>
      <CardHeader>
        <CardTitle>{t("tools.traffic", "Website traffic")}</CardTitle>
      </CardHeader>
      <CardContent className="space-y-3">
        {error && <p className="text-sm text-destructive">{error}</p>}
        <div className="grid grid-cols-2 gap-2 sm:grid-cols-6">
          {cells.map(([k, v]) => (
            <div key={k} className="rounded border p-2">
              <div className="text-xs text-muted-foreground">{k}</div>
              <div className="text-lg font-semibold">{v ?? "-"}</div>
            </div>
          ))}
        </div>
        {data?.top_paths.map((p) => (
          <div key={p.path} className="flex justify-between text-sm">
            <span className="truncate">{p.path}</span>
            <span>{p.requests}</span>
          </div>
        ))}
      </CardContent>
    </Card>
  );
}

export function ClientToolsPage() {
  const { t } = useTranslation();
  const [tab, setTab] = useState<Tab>("cache");
  const tabs: [Tab, string][] = [
    ["cache", t("tools.cache", "Cache servers")],
    ["deploys", t("tools.deploys", "Git deploys")],
    ["scans", t("tools.scans", "Security scans")],
    ["traffic", t("tools.traffic", "Website traffic")],
  ];
  return (
    <div className="space-y-4" data-testid="client-tools-page">
      <h1 className="text-2xl font-bold">{t("tools.title", "Developer tools")}</h1>
      <div className="flex flex-wrap gap-2">
        {tabs.map(([id, label]) => (
          <Button key={id} size="sm" variant={tab === id ? "default" : "outline"} onClick={() => setTab(id)}>
            {label}
          </Button>
        ))}
      </div>
      {tab === "cache" && <CacheTab />}
      {tab === "deploys" && <DeploysTab />}
      {tab === "scans" && <ScansTab />}
      {tab === "traffic" && <TrafficTab />}
    </div>
  );
}
