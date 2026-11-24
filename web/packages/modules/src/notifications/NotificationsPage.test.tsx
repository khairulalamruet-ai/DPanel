import React from "react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { ToastProvider } from "@builtpanel/ui";
import { NotificationItem } from "./types";
import { NotificationBell } from "./NotificationBell";
import { NotificationPreferencesPage } from "./NotificationPreferencesPage";

const mockNotifications: NotificationItem[] = [
  {
    id: "notif-1",
    title: "SSL Certificate Expiring Soon",
    message: "Certificate for mysite.com will expire in 7 days.",
    severity: "warning",
    category: "ssl",
    read: false,
    created_at: new Date(Date.now() - 300000).toISOString(),
    action_url: "/ssl",
    action_label: "Renew SSL",
  },
  {
    id: "notif-2",
    title: "Nightly S3 Backup Succeeded",
    message: "420 MB archive transferred to wasabi-us-east-1.",
    severity: "success",
    category: "backup",
    read: false,
    created_at: new Date(Date.now() - 3600000).toISOString(),
  },
  {
    id: "notif-3",
    title: "Cluster Node node-lon-02 Offline",
    message: "Agent connection dropped 4 minutes ago.",
    severity: "error",
    category: "system",
    read: true,
    created_at: new Date(Date.now() - 7200000).toISOString(),
  },
];

const view = (n: NotificationItem) => ({
  id: n.id,
  event_type: n.category === "ssl" ? "ssl.expiring" : n.category === "backup" ? "backup.finished" : "system.node",
  severity: n.severity === "error" ? "critical" : n.severity,
  subject: n.title,
  body: n.message,
  read: n.read,
  created_at: Math.floor(new Date(n.created_at).getTime() / 1000),
});

let quiet: { paused: boolean; snoozed_until: number | null } = { paused: false, snoozed_until: null };

vi.mock("@builtpanel/api", () => ({
  apiFetch: vi.fn(async (url: string, opts?: any) => {
    if (url === "/api/v1/client/notifications") {
      return { items: mockNotifications.map(view), unread: mockNotifications.filter((n) => !n.read).length };
    }
    if (url === "/api/v1/client/notifications/read") {
      return { updated: 1 };
    }
    if (url === "/api/v1/client/notifications/events") {
      return { events: ["ssl.expiring", "backup.finished"] };
    }
    if (url === "/api/v1/client/notifications/channels") {
      if (opts?.method === "POST") {
        const body = JSON.parse(opts.body);
        return { channel: { id: "ch_new", enabled: true, ...body }, secret: null };
      }
      return [{ id: "ch_01", kind: "email", name: "Me", target: "admin@enterprise.io", events: [], enabled: true }];
    }
    if (url === "/api/v1/client/notifications/quiet") {
      if (opts?.method === "PUT") {
        const body = JSON.parse(opts.body);
        quiet = { paused: body.paused, snoozed_until: body.snooze_minutes ? Math.floor(Date.now() / 1000) + body.snooze_minutes * 60 : null };
      }
      return quiet;
    }
    return {};
  }),
}));

describe("Notification UI Module (T18.05)", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    quiet = { paused: false, snoozed_until: null };
  });

  describe("NotificationBell", () => {
    it("renders trigger button with unread count badge", async () => {
      render(<NotificationBell />);

      await waitFor(() => {
        expect(screen.getByTestId("notification-bell-btn")).toBeDefined();
        // 2 unread out of 3
        expect(screen.getByTestId("unread-count-badge").textContent).toBe("2");
      });
    });

    it("opens flyout dropdown and lists notifications", async () => {
      render(<NotificationBell />);

      await waitFor(() => {
        expect(screen.getByTestId("notification-bell-btn")).toBeDefined();
      });

      fireEvent.click(screen.getByTestId("notification-bell-btn"));

      await waitFor(() => {
        expect(screen.getByTestId("notification-dropdown")).toBeDefined();
        expect(screen.getByText("SSL Certificate Expiring Soon")).toBeDefined();
        expect(screen.getByText("Nightly S3 Backup Succeeded")).toBeDefined();
      });
    });

    it("filters notifications by unread and critical tabs", async () => {
      render(<NotificationBell />);

      fireEvent.click(screen.getByTestId("notification-bell-btn"));

      await waitFor(() => {
        expect(screen.getByTestId("notification-dropdown")).toBeDefined();
      });

      // Filter Unread
      fireEvent.click(screen.getByTestId("tab-notif-unread"));
      expect(screen.getByText("SSL Certificate Expiring Soon")).toBeDefined();
      expect(screen.queryByText("Cluster Node node-lon-02 Offline")).toBeNull();

      // Filter Critical
      fireEvent.click(screen.getByTestId("tab-notif-critical"));
      expect(screen.getByText("SSL Certificate Expiring Soon")).toBeDefined();
      expect(screen.getByText("Cluster Node node-lon-02 Offline")).toBeDefined();
      expect(screen.queryByText("Nightly S3 Backup Succeeded")).toBeNull();
    });

    it("marks all notifications as read", async () => {
      render(<NotificationBell />);

      fireEvent.click(screen.getByTestId("notification-bell-btn"));

      await waitFor(() => {
        expect(screen.getByTestId("btn-mark-all-read")).toBeDefined();
      });

      fireEvent.click(screen.getByTestId("btn-mark-all-read"));

      await waitFor(() => {
        expect(screen.queryByTestId("unread-count-badge")).toBeNull();
      });
    });
  });

  describe("NotificationPreferencesPage", () => {
    it("loads and renders channels, alert triggers, and snooze controls", async () => {
      render(
        <ToastProvider>
          <NotificationPreferencesPage />
        </ToastProvider>
      );

      await waitFor(() => {
        expect(screen.getByTestId("notification-settings-title")).toBeDefined();
        expect(screen.getByTestId("notification-channels-card")).toBeDefined();
        expect(screen.getByTestId("notification-triggers-card")).toBeDefined();
        expect(screen.getByTestId("snooze-control-card")).toBeDefined();
      });
    });

    it("allows toggling snooze window", async () => {
      render(
        <ToastProvider>
          <NotificationPreferencesPage />
        </ToastProvider>
      );

      await waitFor(() => {
        expect(screen.getByTestId("btn-snooze-1h")).toBeDefined();
      });

      fireEvent.click(screen.getByTestId("btn-snooze-1h"));

      await waitFor(() => {
        expect(screen.getByTestId("btn-clear-snooze")).toBeDefined();
      });

      fireEvent.click(screen.getByTestId("btn-clear-snooze"));

      await waitFor(() => {
        expect(screen.getByTestId("btn-snooze-1h")).toBeDefined();
      });
    });

    it("adds an e-mail channel", async () => {
      render(
        <ToastProvider>
          <NotificationPreferencesPage />
        </ToastProvider>
      );

      await waitFor(() => {
        expect(screen.getByTestId("channel-ch_01")).toBeDefined();
      });

      fireEvent.change(screen.getByTestId("input-channel-name"), { target: { value: "Ops" } });
      fireEvent.change(screen.getByTestId("input-channel-target"), { target: { value: "ops@example.com" } });
      fireEvent.click(screen.getByTestId("btn-add-channel"));

      await waitFor(() => {
        expect(screen.getByTestId("channel-ch_new")).toBeDefined();
      });
    });
  });
});
