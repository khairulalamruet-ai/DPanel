import React, { useCallback, useEffect, useState } from "react";
import { Card, CardHeader, CardTitle, CardContent, Button, Badge, Input, useToast } from "@builtpanel/ui";
import { useTranslation } from "@builtpanel/i18n";
import * as admin from "./api";

function useToastError() {
  const { toast } = useToast();
  return (e: unknown) => toast({ title: e instanceof Error ? e.message : String(e), variant: "destructive" });
}

function ImagesCard() {
  const { t } = useTranslation();
  const { toast } = useToast();
  const fail = useToastError();
  const [items, setItems] = useState<admin.ContainerImage[]>([]);
  const [form, setForm] = useState({ name: "", reference: "", default_port: "8080", default_memory_mb: "256", note: "" });

  const load = useCallback(async () => {
    try {
      setItems(await admin.listImages());
    } catch (e) {
      fail(e);
    }
  }, []);
  useEffect(() => {
    void load();
  }, [load]);

  const run = async (fn: () => Promise<unknown>) => {
    try {
      await fn();
      toast({ title: t("adminTools.saved", "Saved") });
      await load();
    } catch (e) {
      fail(e);
    }
  };

  return (
    <Card>
      <CardHeader>
        <CardTitle>{t("adminTools.images", "Container images clients may run")}</CardTitle>
      </CardHeader>
      <CardContent className="space-y-2">
        {items.map((i) => (
          <div key={i.id} className="flex flex-wrap items-center justify-between gap-2 rounded border p-3">
            <div>
              <div className="font-medium">
                {i.name} <Badge variant="outline">{i.enabled ? t("adminTools.on", "offered") : t("adminTools.off", "not offered")}</Badge>
              </div>
              <div className="font-mono text-xs text-muted-foreground">
                {i.reference ?? "-"} · port {i.default_port} · {i.default_memory_mb} MB
              </div>
              {i.note && <div className="text-xs text-muted-foreground">{i.note}</div>}
            </div>
            <div className="flex gap-2">
              <Button
                size="sm"
                variant="outline"
                onClick={() =>
                  run(() =>
                    admin.changeImage(i.id, {
                      name: i.name,
                      reference: i.reference,
                      note: i.note,
                      default_port: i.default_port,
                      default_memory_mb: i.default_memory_mb,
                      enabled: !i.enabled,
                    }),
                  )
                }
              >
                {i.enabled ? t("adminTools.stopOffering", "Stop offering") : t("adminTools.offer", "Offer")}
              </Button>
              <Button size="sm" variant="outline" onClick={() => run(() => admin.removeImage(i.id))}>
                {t("common.delete", "Delete")}
              </Button>
            </div>
          </div>
        ))}
        <form
          className="grid gap-2 sm:grid-cols-3"
          onSubmit={(e) => {
            e.preventDefault();
            void run(() =>
              admin.addImage({
                name: form.name,
                reference: form.reference || null,
                note: form.note || null,
                default_port: Number(form.default_port),
                default_memory_mb: Number(form.default_memory_mb),
                enabled: true,
              }),
            );
          }}
        >
          <Input placeholder="name" value={form.name} onChange={(e) => setForm({ ...form, name: e.target.value })} required />
          <Input placeholder="registry/image:tag" value={form.reference} onChange={(e) => setForm({ ...form, reference: e.target.value })} />
          <Input placeholder={t("adminTools.note", "note")} value={form.note} onChange={(e) => setForm({ ...form, note: e.target.value })} />
          <Input type="number" placeholder="port" value={form.default_port} onChange={(e) => setForm({ ...form, default_port: e.target.value })} required />
          <Input type="number" placeholder="MB" value={form.default_memory_mb} onChange={(e) => setForm({ ...form, default_memory_mb: e.target.value })} />
          <Button type="submit">{t("adminTools.approve", "Approve image")}</Button>
        </form>
      </CardContent>
    </Card>
  );
}

function SmtpCard() {
  const { t } = useTranslation();
  const { toast } = useToast();
  const fail = useToastError();
  const [hasPassword, setHasPassword] = useState(false);
  const [form, setForm] = useState({
    enabled: false,
    host: "",
    port: "587",
    security: "starttls",
    username: "",
    password: "",
    from_address: "",
    from_name: "",
  });
  const [to, setTo] = useState("");

  useEffect(() => {
    void (async () => {
      try {
        const s = await admin.getSmtp();
        if (s) {
          setHasPassword(s.has_password);
          setForm({
            enabled: s.enabled,
            host: s.host,
            port: String(s.port),
            security: s.security,
            username: s.username ?? "",
            password: "",
            from_address: s.from_address,
            from_name: s.from_name ?? "",
          });
        }
      } catch (e) {
        fail(e);
      }
    })();
  }, []);

  const save = async () => {
    try {
      await admin.putSmtp({
        enabled: form.enabled,
        host: form.host,
        port: Number(form.port),
        security: form.security,
        username: form.username || null,
        password: form.password || null,
        from_address: form.from_address,
        from_name: form.from_name || null,
      });
      toast({ title: t("adminTools.saved", "Saved") });
      setForm((f) => ({ ...f, password: "" }));
    } catch (e) {
      fail(e);
    }
  };

  const test = async () => {
    try {
      await admin.testSmtp(to);
      toast({ title: t("adminTools.testSent", "Test mail sent") });
    } catch (e) {
      fail(e);
    }
  };

  return (
    <Card>
      <CardHeader>
        <CardTitle>{t("adminTools.smtp", "Mail server of the panel (SMTP)")}</CardTitle>
      </CardHeader>
      <CardContent className="space-y-3">
        <label className="flex items-center gap-2 text-sm">
          <input type="checkbox" checked={form.enabled} onChange={(e) => setForm({ ...form, enabled: e.target.checked })} />
          {t("adminTools.enabled", "Send the panel's mail through this server")}
        </label>
        <div className="grid gap-2 sm:grid-cols-3">
          <Input placeholder="smtp.example.com" value={form.host} onChange={(e) => setForm({ ...form, host: e.target.value })} />
          <Input type="number" placeholder="587" value={form.port} onChange={(e) => setForm({ ...form, port: e.target.value })} />
          <select className="rounded border bg-background px-2 text-sm" value={form.security} onChange={(e) => setForm({ ...form, security: e.target.value })}>
            <option value="starttls">STARTTLS</option>
            <option value="tls">TLS</option>
            <option value="none">none</option>
          </select>
          <Input placeholder={t("adminTools.username", "username")} value={form.username} onChange={(e) => setForm({ ...form, username: e.target.value })} />
          <Input
            type="password"
            placeholder={hasPassword ? t("adminTools.passwordKept", "password (leave empty to keep)") : t("adminTools.password", "password")}
            value={form.password}
            onChange={(e) => setForm({ ...form, password: e.target.value })}
          />
          <span />
          <Input placeholder="from@example.com" value={form.from_address} onChange={(e) => setForm({ ...form, from_address: e.target.value })} />
          <Input placeholder={t("adminTools.fromName", "sender name")} value={form.from_name} onChange={(e) => setForm({ ...form, from_name: e.target.value })} />
        </div>
        <div className="flex flex-wrap gap-2">
          <Button onClick={() => void save()}>{t("common.save", "Save")}</Button>
          <Input className="w-64" placeholder="you@example.com" value={to} onChange={(e) => setTo(e.target.value)} />
          <Button variant="outline" onClick={() => void test()} disabled={!to}>
            {t("adminTools.sendTest", "Send a test mail")}
          </Button>
        </div>
      </CardContent>
    </Card>
  );
}

export function AdminSystemToolsPage() {
  const { t } = useTranslation();
  return (
    <div className="space-y-4" data-testid="admin-system-tools-page">
      <h1 className="text-2xl font-bold">{t("adminTools.title", "System tools")}</h1>
      <SmtpCard />
      <ImagesCard />
    </div>
  );
}
