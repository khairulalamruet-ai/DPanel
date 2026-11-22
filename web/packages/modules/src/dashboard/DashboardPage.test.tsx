import React from "react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { DashboardPage } from "./DashboardPage";

const STORAGE_KEY = "builtpanel-admin-dashboard-layout";

vi.mock("@builtpanel/auth", () => ({
  useSession: () => ({
    data: {
      user: { username: "admin", name: "Administrator" },
      roles: ["super_admin"],
      branding: { companyName: "Built Panel" },
    },
  }),
}));

vi.mock("@builtpanel/api", () => ({
  api: {
    GET: vi.fn(async () => ({
      data: { status: "ok", version: "0.1.0" },
    })),
  },
  apiFetch: vi.fn(async () => ({}))
}));

vi.mock("./api", () => ({
  loadAdminSummary: vi.fn(async () => ({
    nodes: { total: 3, online: 3, avg_cpu_percent: 18, avg_ram_percent: 42 },
    tenants: { total: 24, active: 22, domains_count: 58, databases_count: 36 },
    resources: {
      cpu_percent: 24,
      memory_used_mb: 6144,
      memory_total_mb: 16384,
      disk_used_gb: 142,
      disk_total_gb: 512,
      network_rx_mb: 4.8,
      network_tx_mb: 8.2,
    },
    operations: [
      { id: "op-vhost-842", type: "VhostTemplateApply", status: "completed", duration_ms: 120, created_at: new Date().toISOString() },
    ],
    security: { blocked_threats_24h: 142, active_waf_rules: 942, ssh_bans: 12 },
    license: { tier: "Built Panel Enterprise Edition", status: "active", signature_valid: true, expires_in_days: 305 },
  })),
}));

let mockStorage: Record<string, string> = {};

describe("Admin Dashboard & Layout Customization (T18.01)", () => {
  beforeEach(() => {
    mockStorage = {};
    const storageMock = {
      getItem: vi.fn((key: string) => mockStorage[key] ?? null),
      setItem: vi.fn((key: string, value: string) => {
        mockStorage[key] = String(value);
      }),
      removeItem: vi.fn((key: string) => {
        delete mockStorage[key];
      }),
      clear: vi.fn(() => {
        mockStorage = {};
      }),
    };
    Object.defineProperty(window, "localStorage", {
      value: storageMock,
      writable: true,
      configurable: true,
    });
    Object.defineProperty(globalThis, "localStorage", {
      value: storageMock,
      writable: true,
      configurable: true,
    });
    vi.clearAllMocks();
  });

  it("renders all 6 default dashboard widgets", async () => {
    render(<DashboardPage />);

    await waitFor(() => {
      expect(screen.getByTestId("widget-nodes_status")).toBeDefined();
      expect(screen.getByTestId("widget-tenants_count")).toBeDefined();
      expect(screen.getByTestId("widget-resource_usage")).toBeDefined();
      expect(screen.getByTestId("widget-recent_operations")).toBeDefined();
      expect(screen.getByTestId("widget-security_alerts")).toBeDefined();
      expect(screen.getByTestId("widget-license_status")).toBeDefined();
    });
  });

  it("opens customize layout dialog and toggles widget visibility", async () => {
    render(<DashboardPage />);

    const customizeBtn = screen.getByTestId("btn-customize-dashboard");
    fireEvent.click(customizeBtn);

    await waitFor(() => {
      expect(screen.getByTestId("dashboard-layout-dialog")).toBeDefined();
    });

    // Disable security alerts widget
    const secCheckbox = screen.getByTestId("checkbox-widget-security_alerts");
    fireEvent.click(secCheckbox);

    // Save layout
    const saveBtn = screen.getByTestId("btn-save-dashboard-layout");
    fireEvent.click(saveBtn);

    await waitFor(() => {
      expect(screen.queryByTestId("widget-security_alerts")).toBeNull();
      // Other widgets remain visible
      expect(screen.getByTestId("widget-nodes_status")).toBeDefined();
    });

    // Verify localStorage has persisted layout
    const stored = JSON.parse(localStorage.getItem(STORAGE_KEY) || "[]");
    const secItem = stored.find((item: any) => item.id === "security_alerts");
    expect(secItem?.enabled).toBe(false);
  });

  it("reorders widgets and restores defaults on reset", async () => {
    render(<DashboardPage />);

    fireEvent.click(screen.getByTestId("btn-customize-dashboard"));

    await waitFor(() => {
      expect(screen.getByTestId("btn-move-down-nodes_status")).toBeDefined();
    });

    // Move nodes_status down
    fireEvent.click(screen.getByTestId("btn-move-down-nodes_status"));

    // Save
    fireEvent.click(screen.getByTestId("btn-save-dashboard-layout"));

    await waitFor(() => {
      const stored = JSON.parse(localStorage.getItem(STORAGE_KEY) || "[]");
      expect(stored[0].id).toBe("tenants_count");
      expect(stored[1].id).toBe("nodes_status");
    });

    // Reopen and reset
    fireEvent.click(screen.getByTestId("btn-customize-dashboard"));
    await waitFor(() => {
      expect(screen.getByTestId("btn-reset-dashboard-layout")).toBeDefined();
    });

    fireEvent.click(screen.getByTestId("btn-reset-dashboard-layout"));

    await waitFor(() => {
      expect(screen.getByTestId("widget-nodes_status")).toBeDefined();
      expect(localStorage.getItem(STORAGE_KEY)).toBeNull();
    });
  });
});
