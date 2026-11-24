import { describe, it, expect, vi, beforeEach } from "vitest";
import React from "react";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { AuditViewerPage } from "./AuditViewerPage";
import { api } from "@builtpanel/api";

let mockHasPermission = true;

vi.mock("@builtpanel/auth", () => ({
  useSession: () => ({
    data: {
      user: { id: "00000000-0000-0000-0000-000000000001", username: "admin" },
      roles: ["super_admin"],
      permissions: ["*"],
    },
    isLoading: false,
    hasPermission: (perm: string) => mockHasPermission,
    hasRole: (role: string) => true,
  }),
}));

vi.mock("@builtpanel/i18n", () => ({
  useTranslation: () => ({
    t: (key: string) => {
      const dict: Record<string, string> = {
        "audit.title": "Audit Logs",
        "audit.desc": "Track and inspect administrative actions, security events, and system changes across the cluster.",
        "audit.filters.title": "Filter Logs",
        "audit.filters.action": "Action",
        "audit.filters.actionPlaceholder": "e.g. auth.login, user.create",
        "audit.filters.actor": "Actor ID",
        "audit.filters.actorPlaceholder": "User UUID or system",
        "audit.filters.outcome": "Outcome",
        "audit.filters.outcomeAll": "All Outcomes",
        "audit.filters.outcomeSuccess": "Success",
        "audit.filters.outcomeFailure": "Failure",
        "audit.filters.outcomeDenied": "Denied",
        "audit.filters.targetType": "Target Type",
        "audit.filters.targetTypePlaceholder": "e.g. user, vhost, backup",
        "audit.filters.apply": "Apply Filters",
        "audit.filters.reset": "Reset",
        "audit.table.timestamp": "Timestamp",
        "audit.table.action": "Action",
        "audit.table.outcome": "Outcome",
        "audit.table.actor": "Actor",
        "audit.table.target": "Target",
        "audit.table.ip": "IP Address",
        "audit.table.details": "Details",
        "audit.table.viewMetadata": "View Details",
        "audit.pagination.limit": "Rows per page",
        "audit.pagination.page": "Page",
        "audit.pagination.previous": "Previous",
        "audit.pagination.next": "Next",
        "audit.metadataModal.title": "Audit Event Details",
        "audit.metadataModal.actorIp": "Actor IP",
        "audit.metadataModal.correlationId": "Correlation ID",
        "audit.metadataModal.nodeId": "Node ID",
        "audit.metadataModal.metadata": "Structured Metadata (JSON)",
        "audit.metadataModal.copyJson": "Copy JSON",
        "audit.metadataModal.copied": "Copied!",
        "audit.metadataModal.close": "Close",
        "audit.empty": "No audit records found matching the criteria.",
        "common.loading": "Loading...",
        "common.error": "Failed to load audit logs",
        "common.noPermission": "Permission denied",
      };
      return dict[key] || key;
    },
  }),
  useFormatters: () => ({
    formatDate: (d: Date) => d.toISOString(),
  }),
}));

const mockAuditLogs = [
  {
    id: "aud_01",
    at: "2026-11-11T12:00:00Z",
    action: "auth.login",
    outcome: "success",
    actor_id: "usr_admin_01",
    actor_ip: "192.168.1.100",
    correlation_id: "corr_01",
    node_id: "node_master",
    target_type: "user",
    target_id: "usr_admin_01",
    tenant_id: "tenant_main",
    metadata: { method: "password", mfa: true, browser: "Chrome" },
  },
  {
    id: "aud_02",
    at: "2026-11-11T12:05:00Z",
    action: "vhost.create",
    outcome: "denied",
    actor_id: "usr_client_02",
    actor_ip: "192.168.1.105",
    correlation_id: "corr_02",
    node_id: "node_worker_1",
    target_type: "vhost",
    target_id: "example.com",
    tenant_id: "tenant_02",
    metadata: { reason: "quota_exceeded", max_domains: 5 },
  },
];

describe("AuditViewerPage", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    mockHasPermission = true;
  });

  it("renders permission denied message when user lacks admin.audit.read", () => {
    mockHasPermission = false;
    render(<AuditViewerPage />);
    expect(screen.getByText("Permission denied")).toBeInTheDocument();
  });

  it("renders audit logs table and filters", async () => {
    vi.spyOn(api, "GET").mockResolvedValue({
      data: {
        items: mockAuditLogs,
        next_cursor: "cursor_page_2",
      },
      error: undefined,
      response: new Response(),
    } as any);

    render(<AuditViewerPage />);

    expect(screen.getByText("Audit Logs")).toBeInTheDocument();
    expect(screen.getByText("Filter Logs")).toBeInTheDocument();

    await waitFor(() => {
      expect(screen.getByText("auth.login")).toBeInTheDocument();
      expect(screen.getByText("vhost.create")).toBeInTheDocument();
      expect(screen.getAllByText("Success").length).toBeGreaterThanOrEqual(1);
      expect(screen.getAllByText("Denied").length).toBeGreaterThanOrEqual(1);
    });
  });

  it("opens metadata dialog when clicking View Details and copies JSON", async () => {
    vi.spyOn(api, "GET").mockResolvedValue({
      data: {
        items: mockAuditLogs,
        next_cursor: null,
      },
      error: undefined,
      response: new Response(),
    } as any);

    // Mock clipboard
    const writeTextMock = vi.fn().mockResolvedValue(undefined);
    Object.assign(navigator, {
      clipboard: {
        writeText: writeTextMock,
      },
    });

    render(<AuditViewerPage />);

    await waitFor(() => {
      expect(screen.getByText("auth.login")).toBeInTheDocument();
    });

    const viewButtons = screen.getAllByRole("button", { name: /View Details/i });
    fireEvent.click(viewButtons[0]);

    await waitFor(() => {
      expect(screen.getByText("Audit Event Details")).toBeInTheDocument();
      expect(screen.getByText(/ID: aud_01/)).toBeInTheDocument();
      expect(screen.getByText(/corr_01/)).toBeInTheDocument();
    });

    const copyBtn = screen.getByRole("button", { name: /Copy JSON/i });
    fireEvent.click(copyBtn);

    expect(writeTextMock).toHaveBeenCalled();
  });

  it("applies filters and submits form", async () => {
    const getSpy = vi.spyOn(api, "GET").mockResolvedValue({
      data: {
        items: [mockAuditLogs[1]],
        next_cursor: null,
      },
      error: undefined,
      response: new Response(),
    } as any);

    render(<AuditViewerPage />);

    await waitFor(() => {
      expect(screen.getByPlaceholderText(/auth\.login/i)).toBeInTheDocument();
    });

    const actionInput = screen.getByPlaceholderText(/auth\.login/i);
    fireEvent.change(actionInput, { target: { value: "vhost" } });

    const applyBtn = screen.getByRole("button", { name: /Apply Filters/i });
    fireEvent.click(applyBtn);

    await waitFor(() => {
      expect(getSpy).toHaveBeenCalledWith(
        "/api/v1/admin/audit-logs",
        expect.objectContaining({
          params: expect.objectContaining({
            query: expect.objectContaining({
              action: "vhost",
            }),
          }),
        })
      );
    });
  });

  it("handles next and previous pagination", async () => {
    const getSpy = vi.spyOn(api, "GET").mockResolvedValue({
      data: {
        items: mockAuditLogs,
        next_cursor: "cursor_page_2",
      },
      error: undefined,
      response: new Response(),
    } as any);

    render(<AuditViewerPage />);

    await waitFor(() => {
      expect(screen.getByText("auth.login")).toBeInTheDocument();
    });

    const nextBtn = screen.getByRole("button", { name: /Next/i });
    expect(nextBtn).toBeEnabled();

    fireEvent.click(nextBtn);

    await waitFor(() => {
      expect(getSpy).toHaveBeenCalledWith(
        "/api/v1/admin/audit-logs",
        expect.objectContaining({
          params: expect.objectContaining({
            query: expect.objectContaining({
              cursor: "cursor_page_2",
            }),
          }),
        })
      );
    });
  });
});
