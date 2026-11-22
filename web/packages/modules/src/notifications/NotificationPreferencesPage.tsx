import React, { useState, useEffect, useCallback } from "react";
import { useTranslation } from "@builtpanel/i18n";
import { useOptionalSession } from "@builtpanel/auth";
import { useToast, Card, CardHeader, CardTitle, CardDescription, CardContent, Button, Input, Badge } from "@builtpanel/ui";
import { Bell, Mail, Webhook, Clock, PauseCircle, PlayCircle, Trash2, Plus } from "lucide-react";
import {
  addChannel,
  changeChannel,
  getQuiet,
  listChannels,
  listEvents,
  removeChannel,
  setQuiet,
  type Audience,
  type Channel,
  type Quiet,
} from "./api";

/** Where notifications go (e-mail, webhook), which events go where, and when to keep quiet. */
export function NotificationPreferencesPage() {
  const { t } = useTranslation();
  const { toast } = useToast();
  const who: Audience = useOptionalSession()?.data?.user.kind === "admin" ? "admin" : "client";

  const [channels, setChannels] = useState<Channel[]>([]);
  const [events, setEvents] = useState<string[]>([]);
  const [quiet, setQuietState] = useState<Quiet>({ paused: false, snoozed_until: null });

  const [kind, setKind] = useState<"email" | "webhook">("email");
  const [name, setName] = useState("");
  const [target, setTarget] = useState("");
  const [picked, setPicked] = useState<string[]>([]);
  const [secret, setSecret] = useState<string | null>(null);

  const fail = useCallback(
    (err: any) =>
      toast({
        title: t("common.error", "Error"),
        description: err?.message || t("common.unknownError", "Something went wrong"),
        variant: "destructive",
      }),
    [t, toast],
  );

  const load = useCallback(async () => {
    try {
      const [c, e, q] = await Promise.all([listChannels(who), listEvents(who), getQuiet(who)]);
      setChannels(c);
      setEvents(e);
      setQuietState(q);
    } catch (err) {
      fail(err);
    }
  }, [who, fail]);

  useEffect(() => {
    void load();
  }, [load]);

  const handleAdd = async (e: React.FormEvent) => {
    e.preventDefault();
    try {
      const res = await addChannel(who, { kind, name: name.trim(), target: target.trim(), events: picked });
      setChannels((prev) => [...prev, res.channel]);
      setSecret(res.secret ?? null);
      setName("");
      setTarget("");
      setPicked([]);
      toast({ title: t("notifications.savedTitle", "Saved"), description: t("notifications.channelAdded", "Channel added"), variant: "success" });
    } catch (err) {
      fail(err);
    }
  };

  const handleToggle = async (c: Channel) => {
    try {
      await changeChannel(who, c.id, { enabled: !c.enabled });
      setChannels((prev) => prev.map((x) => (x.id === c.id ? { ...x, enabled: !c.enabled } : x)));
    } catch (err) {
      fail(err);
    }
  };

  const handleRemove = async (c: Channel) => {
    try {
      await removeChannel(who, c.id);
      setChannels((prev) => prev.filter((x) => x.id !== c.id));
    } catch (err) {
      fail(err);
    }
  };

  const handleSnooze = async (minutes: number) => {
    try {
      setQuietState(await setQuiet(who, false, minutes));
    } catch (err) {
      fail(err);
    }
  };

  const handlePause = async (paused: boolean) => {
    try {
      setQuietState(await setQuiet(who, paused, 0));
    } catch (err) {
      fail(err);
    }
  };

  const snoozedUntil = quiet.snoozed_until ? new Date(quiet.snoozed_until * 1000) : null;
  const isSnoozed = Boolean(snoozedUntil && snoozedUntil > new Date());

  return (
    <div className="space-y-6" data-testid="notification-preferences-page">
      <div>
        <h1 className="text-2xl font-bold tracking-tight" data-testid="notification-settings-title">
          {t("notifications.settingsPageTitle", "Notification Channels & Alert Rules")}
        </h1>
        <p className="text-muted-foreground text-sm">
          {t("notifications.settingsPageSubtitle", "Choose where notifications are sent, which events go where, and when to keep quiet.")}
        </p>
      </div>

      {/* Channels */}
      <Card data-testid="notification-channels-card">
        <CardHeader>
          <CardTitle className="text-base flex items-center gap-2">
            <Bell className="h-4 w-4 text-primary" />
            {t("notifications.channelsTitle", "Channels")}
          </CardTitle>
          <CardDescription>
            {t("notifications.channelsDesc", "Every notification also lands in the bell; a channel sends the chosen events elsewhere.")}
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          {channels.length === 0 ? (
            <p className="text-sm text-muted-foreground" data-testid="no-channels">
              {t("notifications.noChannels", "No channels yet.")}
            </p>
          ) : (
            <ul className="divide-y rounded-md border">
              {channels.map((c) => (
                <li key={c.id} className="flex items-center justify-between gap-3 p-3 text-sm" data-testid={`channel-${c.id}`}>
                  <div className="flex items-center gap-2 min-w-0">
                    {c.kind === "webhook" ? <Webhook className="h-4 w-4 shrink-0" /> : <Mail className="h-4 w-4 shrink-0" />}
                    <div className="min-w-0">
                      <div className="font-medium truncate">{c.name}</div>
                      <div className="text-xs text-muted-foreground font-mono truncate">{c.target}</div>
                      <div className="flex flex-wrap gap-1 mt-1">
                        {c.events.length === 0 ? (
                          <Badge variant="secondary">{t("notifications.allEvents", "all events")}</Badge>
                        ) : (
                          c.events.map((ev) => (
                            <Badge key={ev} variant="secondary" className="font-mono text-[10px]">
                              {ev}
                            </Badge>
                          ))
                        )}
                      </div>
                    </div>
                  </div>
                  <div className="flex items-center gap-2 shrink-0">
                    <label className="flex items-center gap-1.5 text-xs">
                      <input type="checkbox" checked={c.enabled} onChange={() => handleToggle(c)} data-testid={`toggle-channel-${c.id}`} />
                      {t("common.enabled", "On")}
                    </label>
                    <Button size="sm" variant="ghost" onClick={() => handleRemove(c)} data-testid={`remove-channel-${c.id}`}>
                      <Trash2 className="h-4 w-4 text-destructive" />
                    </Button>
                  </div>
                </li>
              ))}
            </ul>
          )}

          {secret && (
            <div className="rounded-md border border-amber-500/40 bg-amber-500/10 p-3 text-xs" data-testid="webhook-secret">
              {t("notifications.secretOnce", "The webhook's signing secret is shown only now:")} <code className="font-mono">{secret}</code>
            </div>
          )}

          <form onSubmit={handleAdd} className="grid grid-cols-1 md:grid-cols-4 gap-2 items-end border-t pt-4">
            <div className="space-y-1">
              <label className="text-xs font-semibold text-muted-foreground uppercase tracking-wider">{t("notifications.kind", "Kind")}</label>
              <select
                value={kind}
                onChange={(e) => setKind(e.target.value as "email" | "webhook")}
                className="w-full h-9 rounded-md border border-input bg-background px-2.5 text-sm"
                data-testid="select-channel-kind"
              >
                <option value="email">E-mail</option>
                <option value="webhook">Webhook</option>
              </select>
            </div>
            <div className="space-y-1">
              <label className="text-xs font-semibold text-muted-foreground uppercase tracking-wider">{t("common.name", "Name")}</label>
              <Input value={name} onChange={(e) => setName(e.target.value)} required data-testid="input-channel-name" />
            </div>
            <div className="space-y-1 md:col-span-1">
              <label className="text-xs font-semibold text-muted-foreground uppercase tracking-wider">
                {kind === "email" ? t("notifications.address", "Address") : "URL"}
              </label>
              <Input
                value={target}
                onChange={(e) => setTarget(e.target.value)}
                required
                placeholder={kind === "email" ? "me@example.com" : "https://example.com/hook"}
                data-testid="input-channel-target"
              />
            </div>
            <Button type="submit" size="sm" className="gap-1.5" data-testid="btn-add-channel">
              <Plus className="h-4 w-4" />
              {t("common.add", "Add")}
            </Button>
          </form>

          {events.length > 0 && (
            <div className="space-y-1.5" data-testid="notification-triggers-card">
              <div className="text-xs font-semibold text-muted-foreground uppercase tracking-wider">
                {t("notifications.eventsPick", "Events for the new channel (none picked: all)")}
              </div>
              <div className="flex flex-wrap gap-2">
                {events.map((ev) => (
                  <label key={ev} className="flex items-center gap-1.5 text-xs font-mono border rounded px-2 py-1">
                    <input
                      type="checkbox"
                      checked={picked.includes(ev)}
                      onChange={() => setPicked((p) => (p.includes(ev) ? p.filter((x) => x !== ev) : [...p, ev]))}
                    />
                    {ev}
                  </label>
                ))}
              </div>
            </div>
          )}
        </CardContent>
      </Card>

      {/* Quiet */}
      <Card data-testid="snooze-control-card">
        <CardHeader>
          <CardTitle className="text-base flex items-center gap-2">
            <Clock className="h-4 w-4 text-primary" />
            {t("notifications.quietTitle", "Quiet hours")}
          </CardTitle>
          <CardDescription>
            {quiet.paused
              ? t("notifications.pausedNow", "Notifications are paused.")
              : isSnoozed
                ? t("notifications.snoozedDesc", "Alerts paused until {{time}}.", {
                    time: snoozedUntil?.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" }),
                  })
                : t("notifications.activeNow", "Notifications are on.")}
          </CardDescription>
        </CardHeader>
        <CardContent className="flex flex-wrap gap-2">
          {isSnoozed || quiet.paused ? (
            <Button size="sm" variant="outline" className="gap-1.5" onClick={() => handlePause(false)} data-testid="btn-clear-snooze">
              <PlayCircle className="h-4 w-4" />
              {t("notifications.resume", "Resume")}
            </Button>
          ) : (
            <>
              <Button size="sm" variant="outline" onClick={() => handleSnooze(60)} data-testid="btn-snooze-1h">
                1 h
              </Button>
              <Button size="sm" variant="outline" onClick={() => handleSnooze(240)} data-testid="btn-snooze-4h">
                4 h
              </Button>
              <Button size="sm" variant="outline" onClick={() => handleSnooze(1440)} data-testid="btn-snooze-24h">
                24 h
              </Button>
              <Button size="sm" variant="outline" className="gap-1.5" onClick={() => handlePause(true)} data-testid="btn-pause-notifications">
                <PauseCircle className="h-4 w-4" />
                {t("notifications.pause", "Pause")}
              </Button>
            </>
          )}
        </CardContent>
      </Card>
    </div>
  );
}
