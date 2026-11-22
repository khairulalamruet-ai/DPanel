import React, { useState, useEffect, useRef, useCallback } from "react";
import { useTranslation } from "@builtpanel/i18n";
import { useOptionalSession } from "@builtpanel/auth";
import { listNotifications, markRead, type Audience } from "./api";
import { Badge, Button } from "@builtpanel/ui";
import {
  Bell,
  CheckCheck,
  Check,
  Settings,
  ShieldAlert,
  Lock,
  Archive,
  Server,
  AlertTriangle,
  Info,
  X,
  ExternalLink,
} from "lucide-react";
import { NotificationItem, NotificationSeverity, NotificationCategory } from "./types";

interface NotificationBellProps {
  settingsHref?: string;
}

export function NotificationBell({ settingsHref = "/settings/notifications" }: NotificationBellProps) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [filter, setFilter] = useState<"all" | "unread" | "critical">("all");
  const [notifications, setNotifications] = useState<NotificationItem[]>([]);
  const [loading, setLoading] = useState(false);
  const dropdownRef = useRef<HTMLDivElement>(null);
  const who: Audience = useOptionalSession()?.data?.user.kind === "admin" ? "admin" : "client";

  const fetchNotifications = useCallback(async () => {
    try {
      setLoading(true);
      const res = await listNotifications(who);
      setNotifications(res.items);
    } catch {
      // Ignore background fetch failure
    } finally {
      setLoading(false);
    }
  }, [who]);

  useEffect(() => {
    fetchNotifications();
    // Refresh periodically
    const timer = setInterval(fetchNotifications, 60000);
    return () => clearInterval(timer);
  }, [fetchNotifications]);

  // Close dropdown on outside click
  useEffect(() => {
    function handleClickOutside(event: MouseEvent) {
      if (dropdownRef.current && !dropdownRef.current.contains(event.target as Node)) {
        setOpen(false);
      }
    }
    if (open) {
      document.addEventListener("mousedown", handleClickOutside);
    }
    return () => {
      document.removeEventListener("mousedown", handleClickOutside);
    };
  }, [open]);

  const unreadCount = notifications.filter((n) => !n.read).length;

  const handleMarkAsRead = async (id: string, e?: React.MouseEvent) => {
    e?.stopPropagation();
    try {
      await markRead(who, [id]);
      setNotifications((prev) =>
        prev.map((n) => (n.id === id ? { ...n, read: true } : n))
      );
    } catch {
      // Revert if error
    }
  };

  const handleMarkAllRead = async () => {
    try {
      await markRead(who);
      setNotifications((prev) => prev.map((n) => ({ ...n, read: true })));
    } catch {
      // Revert if error
    }
  };

  const filteredNotifications = notifications.filter((n) => {
    if (filter === "unread") return !n.read;
    if (filter === "critical") return n.severity === "error" || n.severity === "warning";
    return true;
  });

  const getCategoryIcon = (category: NotificationCategory, severity: NotificationSeverity) => {
    if (severity === "error") {
      return <ShieldAlert className="h-4 w-4 text-destructive shrink-0" />;
    }
    switch (category) {
      case "security":
        return <ShieldAlert className="h-4 w-4 text-amber-500 shrink-0" />;
      case "ssl":
        return <Lock className="h-4 w-4 text-primary shrink-0" />;
      case "backup":
        return <Archive className="h-4 w-4 text-emerald-500 shrink-0" />;
      case "system":
        return <Server className="h-4 w-4 text-blue-500 shrink-0" />;
      default:
        return <Info className="h-4 w-4 text-muted-foreground shrink-0" />;
    }
  };

  const formatRelativeTime = (isoString: string) => {
    const diff = Date.now() - new Date(isoString).getTime();
    const minutes = Math.floor(diff / 60000);
    if (minutes < 1) return t("notifications.justNow", "Just now");
    if (minutes < 60) return `${minutes}m ago`;
    const hours = Math.floor(minutes / 60);
    if (hours < 24) return `${hours}h ago`;
    const days = Math.floor(hours / 24);
    return `${days}d ago`;
  };

  return (
    <div className="relative" ref={dropdownRef} data-testid="notification-bell-container">
      {/* Bell Trigger Button */}
      <button
        type="button"
        onClick={() => setOpen(!open)}
        className="relative rounded-md p-1.5 text-muted-foreground hover:bg-accent hover:text-accent-foreground transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
        aria-label={t("notifications.bellLabel", "Notifications")}
        title={t("notifications.bellLabel", "Notifications")}
        data-testid="notification-bell-btn"
      >
        <Bell className="h-4 w-4" />
        {unreadCount > 0 && (
          <span
            className="absolute -top-1 -right-1 flex h-4 min-w-[16px] items-center justify-center rounded-full bg-destructive px-1 text-[10px] font-bold text-destructive-foreground shadow-sm animate-in zoom-in-75"
            data-testid="unread-count-badge"
          >
            {unreadCount > 9 ? "9+" : unreadCount}
          </span>
        )}
      </button>

      {/* Dropdown Flyout */}
      {open && (
        <div
          className="absolute right-0 mt-2 w-80 sm:w-96 rounded-xl border bg-card text-card-foreground shadow-xl z-50 overflow-hidden flex flex-col max-h-[85vh] animate-in fade-in-0 zoom-in-95"
          data-testid="notification-dropdown"
        >
          {/* Header */}
          <div className="p-3 border-b flex items-center justify-between bg-muted/20">
            <div className="flex items-center gap-2">
              <span className="font-bold text-sm" data-testid="notification-dropdown-title">
                {t("notifications.title", "Notifications")}
              </span>
              {unreadCount > 0 && (
                <Badge variant="destructive" className="text-[10px] h-4.5 px-1.5 font-mono">
                  {unreadCount} {t("notifications.unread", "unread")}
                </Badge>
              )}
            </div>

            <div className="flex items-center gap-1">
              {unreadCount > 0 && (
                <button
                  type="button"
                  onClick={handleMarkAllRead}
                  className="p-1 rounded text-muted-foreground hover:text-foreground text-xs flex items-center gap-1 px-1.5 hover:bg-muted"
                  title={t("notifications.markAllRead", "Mark all read")}
                  data-testid="btn-mark-all-read"
                >
                  <CheckCheck className="h-3.5 w-3.5 text-primary" />
                  <span className="text-[11px] hidden sm:inline">
                    {t("notifications.markAllRead", "Mark all read")}
                  </span>
                </button>
              )}

              <a
                href={settingsHref}
                onClick={() => setOpen(false)}
                className="p-1 rounded text-muted-foreground hover:text-foreground hover:bg-muted"
                title={t("notifications.settings", "Settings")}
                data-testid="btn-notification-settings"
              >
                <Settings className="h-3.5 w-3.5" />
              </a>
            </div>
          </div>

          {/* Filter Pills */}
          <div className="flex items-center gap-1 p-2 border-b bg-muted/10 text-xs">
            <button
              type="button"
              onClick={() => setFilter("all")}
              className={`px-2.5 py-1 rounded-md transition-colors ${
                filter === "all"
                  ? "bg-primary text-primary-foreground font-semibold"
                  : "text-muted-foreground hover:bg-muted"
              }`}
              data-testid="tab-notif-all"
            >
              {t("notifications.allFilter", "All")} ({notifications.length})
            </button>

            <button
              type="button"
              onClick={() => setFilter("unread")}
              className={`px-2.5 py-1 rounded-md transition-colors ${
                filter === "unread"
                  ? "bg-primary text-primary-foreground font-semibold"
                  : "text-muted-foreground hover:bg-muted"
              }`}
              data-testid="tab-notif-unread"
            >
              {t("notifications.unreadFilter", "Unread")} ({unreadCount})
            </button>

            <button
              type="button"
              onClick={() => setFilter("critical")}
              className={`px-2.5 py-1 rounded-md transition-colors ${
                filter === "critical"
                  ? "bg-primary text-primary-foreground font-semibold"
                  : "text-muted-foreground hover:bg-muted"
              }`}
              data-testid="tab-notif-critical"
            >
              {t("notifications.criticalFilter", "Critical")}
            </button>
          </div>

          {/* List */}
          <div className="flex-1 overflow-y-auto divide-y divide-border/50 max-h-[360px]">
            {filteredNotifications.length === 0 ? (
              <div
                className="py-10 px-4 text-center text-xs text-muted-foreground space-y-1.5"
                data-testid="empty-notifications"
              >
                <Bell className="h-8 w-8 mx-auto text-muted-foreground/40 stroke-1" />
                <p className="font-semibold text-foreground">
                  {t("notifications.noNotifications", "No notifications")}
                </p>
                <p className="text-[11px]">
                  {filter === "unread"
                    ? t("notifications.allCaughtUp", "You are all caught up!")
                    : t("notifications.emptyDescription", "System alerts and events will appear here.")}
                </p>
              </div>
            ) : (
              filteredNotifications.map((notif) => (
                <div
                  key={notif.id}
                  onClick={() => !notif.read && handleMarkAsRead(notif.id)}
                  className={`p-3 flex items-start gap-3 transition-colors cursor-pointer group ${
                    notif.read ? "bg-card hover:bg-muted/30 opacity-75" : "bg-primary/5 hover:bg-primary/10"
                  }`}
                  data-testid={`notification-item-${notif.id}`}
                >
                  <div className="p-1.5 rounded-md bg-muted/60 mt-0.5">
                    {getCategoryIcon(notif.category, notif.severity)}
                  </div>

                  <div className="flex-1 min-w-0 space-y-1">
                    <div className="flex items-start justify-between gap-1">
                      <h4 className="text-xs font-semibold text-foreground leading-snug">
                        {notif.title}
                      </h4>
                      <span className="text-[10px] text-muted-foreground font-mono shrink-0 whitespace-nowrap">
                        {formatRelativeTime(notif.created_at)}
                      </span>
                    </div>

                    <p className="text-[11px] text-muted-foreground leading-relaxed line-clamp-2">
                      {notif.message}
                    </p>

                    {notif.action_url && (
                      <div className="pt-0.5">
                        <a
                          href={notif.action_url}
                          onClick={(e) => {
                            e.stopPropagation();
                            if (!notif.read) handleMarkAsRead(notif.id);
                            setOpen(false);
                          }}
                          className="inline-flex items-center gap-1 text-[11px] text-primary hover:underline font-medium"
                          data-testid={`notif-action-link-${notif.id}`}
                        >
                          <span>{notif.action_label || t("common.viewDetails", "View")}</span>
                          <ExternalLink className="h-3 w-3" />
                        </a>
                      </div>
                    )}
                  </div>

                  {/* Actions & Read Indicator */}
                  <div className="flex flex-col items-center gap-2 self-stretch justify-between shrink-0">
                    {!notif.read && (
                      <span
                        className="h-2 w-2 rounded-full bg-primary"
                        title="Unread"
                        data-testid={`unread-dot-${notif.id}`}
                      />
                    )}
                  </div>
                </div>
              ))
            )}
          </div>

          {/* Footer */}
          <div className="p-2 border-t bg-muted/20 text-center">
            <a
              href={settingsHref}
              onClick={() => setOpen(false)}
              className="text-xs text-primary hover:underline font-semibold"
              data-testid="link-view-all-notifications"
            >
              {t("notifications.manageChannels", "Manage Notification Channels & Alerts →")}
            </a>
          </div>
        </div>
      )}
    </div>
  );
}
