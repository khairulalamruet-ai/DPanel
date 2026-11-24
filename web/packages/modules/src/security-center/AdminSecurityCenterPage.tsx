import React, { useCallback, useEffect, useState } from "react";
import { Card, CardHeader, CardTitle, CardContent, Button, Badge, Input, useToast } from "@builtpanel/ui";
import { useTranslation } from "@builtpanel/i18n";
import * as sec from "./api";

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

function useRun(reload: () => Promise<void>) {
  const { toast } = useToast();
  return async (fn: () => Promise<unknown>, ok: string) => {
    try {
      await fn();
      toast({ title: ok });
      await reload();
    } catch (e) {
      toast({ title: e instanceof Error ? e.message : String(e), variant: "destructive" });
    }
  };
}

function ImagesCard() {
  const { t } = useTranslation();
  const { data, error, reload } = useLoad(sec.listImages);
  const run = useRun(reload);
  const [open, setOpen] = useState<string | null>(null);
  const [findings, setFindings] = useState<sec.ImageFinding[]>([]);

  const show = async (reference: string) => {
    setOpen(reference);
    try {
      setFindings(await sec.imageFindings(reference));
    } catch {
      setFindings([]);
    }
  };

  return (
    <Card>
      <CardHeader>
        <CardTitle>{t("securityCenter.images", "Container images")}</CardTitle>
      </CardHeader>
      <CardContent className="space-y-2">
        {error && <p className="text-sm text-destructive">{error}</p>}
        {data?.map((i) => (
          <div key={i.reference} className="space-y-1 rounded border p-3">
            <div className="flex flex-wrap items-center justify-between gap-2">
              <div className="font-mono text-sm">
                {i.reference} {i.update_available && <Badge variant="outline">{t("securityCenter.newer", "newer version")}</Badge>}
              </div>
              <div className="flex gap-2">
                <Button size="sm" onClick={() => run(() => sec.scanImage(i.reference), t("securityCenter.asked", "Scan asked for"))}>
                  {t("securityCenter.scan", "Scan")}
                </Button>
                <Button size="sm" variant="outline" onClick={() => void show(i.reference)}>
                  {t("securityCenter.findings", "Findings")}
                </Button>
              </div>
            </div>
            <div className="text-xs text-muted-foreground">
              {i.role} · critical {i.critical ?? "-"} · high {i.high ?? "-"} · medium {i.medium ?? "-"} · low {i.low ?? "-"}
              {i.scanned_at ? ` · ${new Date(i.scanned_at * 1000).toLocaleString()}` : ""}
            </div>
            {open === i.reference &&
              (findings.length === 0 ? (
                <p className="text-xs text-muted-foreground">{t("securityCenter.noFindings", "No findings.")}</p>
              ) : (
                findings.map((f) => (
                  <div key={f.id + f.package} className="text-xs">
                    {f.severity}: {f.id} {f.package} {f.installed}{f.fixed ? ` → ${f.fixed}` : ""} — {f.title}
                  </div>
                ))
              ))}
          </div>
        ))}
      </CardContent>
    </Card>
  );
}

function UserAgentsCard() {
  const { t } = useTranslation();
  const { data, error, reload } = useLoad(sec.listUserAgents);
  const run = useRun(reload);
  const [pattern, setPattern] = useState("");
  const [note, setNote] = useState("");
  return (
    <Card>
      <CardHeader>
        <CardTitle>{t("securityCenter.userAgents", "Refused User-Agents")}</CardTitle>
      </CardHeader>
      <CardContent className="space-y-2">
        {error && <p className="text-sm text-destructive">{error}</p>}
        {data?.map((u) => (
          <div key={u.id} className="flex items-center justify-between gap-2 border-b py-1 text-sm">
            <span className="font-mono">{u.pattern} {u.note && <span className="text-xs text-muted-foreground">{u.note}</span>}</span>
            <Button size="sm" variant="outline" onClick={() => run(() => sec.removeUserAgent(u.id), t("securityCenter.removed", "Removed"))}>
              {t("common.delete", "Delete")}
            </Button>
          </div>
        ))}
        <form
          className="flex flex-wrap gap-2"
          onSubmit={(e) => {
            e.preventDefault();
            void run(() => sec.addUserAgent(pattern, note), t("securityCenter.added", "Added")).then(() => {
              setPattern("");
              setNote("");
            });
          }}
        >
          <Input placeholder="BadBot" value={pattern} onChange={(e) => setPattern(e.target.value)} required />
          <Input placeholder={t("securityCenter.note", "note")} value={note} onChange={(e) => setNote(e.target.value)} />
          <Button type="submit">{t("securityCenter.add", "Add")}</Button>
        </form>
      </CardContent>
    </Card>
  );
}

function AdvisoriesCard() {
  const { t } = useTranslation();
  const { data, error, reload } = useLoad(sec.listAdvisories);
  const run = useRun(reload);
  const [form, setForm] = useState({ component: "plugin", slug: "", fixed_in: "", severity: "high", title: "", reference: "" });
  return (
    <Card>
      <CardHeader>
        <CardTitle>{t("securityCenter.advisories", "WordPress advisories")}</CardTitle>
      </CardHeader>
      <CardContent className="space-y-2">
        {error && <p className="text-sm text-destructive">{error}</p>}
        {data?.map((a) => (
          <div key={a.id} className="flex items-center justify-between gap-2 border-b py-1 text-sm">
            <span>
              <Badge variant="outline">{a.severity}</Badge> {a.component}/{a.slug} &lt; {a.fixed_in} — {a.title}
            </span>
            <Button size="sm" variant="outline" onClick={() => run(() => sec.removeAdvisory(a.id), t("securityCenter.removed", "Removed"))}>
              {t("common.delete", "Delete")}
            </Button>
          </div>
        ))}
        <form
          className="grid gap-2 sm:grid-cols-3"
          onSubmit={(e) => {
            e.preventDefault();
            void run(() => sec.addAdvisory(form), t("securityCenter.added", "Added"));
          }}
        >
          <select className="rounded border bg-background px-2 text-sm" value={form.component} onChange={(e) => setForm({ ...form, component: e.target.value })}>
            <option value="core">core</option>
            <option value="plugin">plugin</option>
            <option value="theme">theme</option>
          </select>
          <Input placeholder="slug" value={form.slug} onChange={(e) => setForm({ ...form, slug: e.target.value })} required />
          <Input placeholder="fixed in (6.4.2)" value={form.fixed_in} onChange={(e) => setForm({ ...form, fixed_in: e.target.value })} required />
          <select className="rounded border bg-background px-2 text-sm" value={form.severity} onChange={(e) => setForm({ ...form, severity: e.target.value })}>
            {["low", "medium", "high", "critical"].map((s) => (
              <option key={s} value={s}>{s}</option>
            ))}
          </select>
          <Input placeholder={t("securityCenter.title", "title")} value={form.title} onChange={(e) => setForm({ ...form, title: e.target.value })} required />
          <Input placeholder="reference URL" value={form.reference} onChange={(e) => setForm({ ...form, reference: e.target.value })} />
          <Button type="submit">{t("securityCenter.add", "Add")}</Button>
        </form>
      </CardContent>
    </Card>
  );
}

export function AdminSecurityCenterPage() {
  const { t } = useTranslation();
  return (
    <div className="space-y-4" data-testid="admin-security-center-page">
      <h1 className="text-2xl font-bold">{t("securityCenter.title", "Security center")}</h1>
      <ImagesCard />
      <UserAgentsCard />
      <AdvisoriesCard />
    </div>
  );
}
