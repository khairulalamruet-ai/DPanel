import React from "react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { ClientFirewallPage } from "./ClientFirewallPage";
import { AdminFirewallPage } from "./AdminFirewallPage";
import { AddIpRuleDialog } from "./AddIpRuleDialog";
import { AddPortDialog } from "./AddPortDialog";
import { AddGlobalBanDialog } from "./AddGlobalBanDialog";
import { ClientIpRule, AdminFirewallOverview } from "./types";

// Mock @builtpanel/api
const mockApiFetch = vi.fn();
vi.mock("@builtpanel/api", () => ({
  apiFetch: (...args: any[]) => mockApiFetch(...args),
}));

// Mock @builtpanel/i18n
vi.mock("@builtpanel/i18n", () => ({
  useTranslation: () => ({
    t: (key: string, params?: Record<string, any>) => {
      if (params) {
        let str = key;
        for (const [k, v] of Object.entries(params)) {
          str += ` ${k}=${v}`;
        }
        return str;
      }
      return key;
    },
  }),
}));

// Mock @builtpanel/ui toast
const mockToast = vi.fn();
vi.mock("@builtpanel/ui", async () => {
  const actual = await vi.importActual<any>("@builtpanel/ui");
  return {
    ...actual,
    useToast: () => ({ toast: mockToast }),
  };
});

describe("Firewall & IP Access Control UI (T11.08)", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  const mockClientRules: ClientIpRule[] = [
    {
      id: "ip-rule-1",
      domain: "example.com",
      ip_cidr: "198.51.100.42",
      action: "block",
      notes: "Aggressive scraper / brute force",
      created_at: "2026-11-12 14:00:00",
    },
    {
      id: "ip-rule-2",
      domain: "example.com",
      ip_cidr: "192.0.2.10",
      action: "allow",
      notes: "Office static IP",
      created_at: "2026-11-10 10:00:00",
    },
  ];

  const mockAdminOverview: AdminFirewallOverview = {
    node_id: "node-1",
    status: "active",
    nftables_version: "nftables v1.0.9 (Kernel Packet Filter)",
    active_ports: [
      { id: "port-1", port: 22, protocol: "tcp", service_name: "SSH Management", is_system_protected: true, status: "open" },
      { id: "port-2", port: 8080, protocol: "tcp", service_name: "Dev Webhook Port", is_system_protected: false, status: "open" },
    ],
    active_bans: [
      {
        id: "ban-1",
        ip: "198.51.100.22",
        reason: "Exceeded 10 failed panel login attempts",
        source: "bruteforce_auto",
        service: "panel_login",
        banned_at: "2026-11-13 10:15:00",
        expires_at: "2026-11-13 11:15:00",
      },
    ],
    rate_limit_enabled: true,
    rate_limit_rps: 50,
  };

  it("ClientFirewallPage: renders rules and verifies DoD: Client can block IP for their site", async () => {
    mockApiFetch.mockImplementation((url: string) => {
      if (url.includes("/api/v1/client/firewall/rules")) return Promise.resolve(mockClientRules);
      return Promise.resolve({});
    });

    render(<ClientFirewallPage />);

    await waitFor(() => {
      expect(screen.getByTestId("firewall-page-title")).toBeInTheDocument();
      expect(screen.getByTestId("card-blocked-ips-count")).toHaveTextContent("1");
      expect(screen.getByTestId("card-allowed-ips-count")).toHaveTextContent("1");
      expect(screen.getByTestId("ip-rule-row-ip-rule-1")).toBeInTheDocument();
      expect(screen.getByText("198.51.100.42")).toBeInTheDocument();
    });

    // DoD: Open Block IP modal
    const blockBtn = screen.getByTestId("btn-open-add-ip-rule");
    fireEvent.click(blockBtn);

    await waitFor(() => {
      expect(screen.getByTestId("add-ip-rule-title")).toBeInTheDocument();
    });

    // Enter invalid IP: submit button remains disabled
    const ipInput = screen.getByTestId("input-ip-cidr");
    fireEvent.change(ipInput, { target: { value: "invalid-ip" } });
    const submitBtn = screen.getByTestId("btn-confirm-add-ip-rule");
    expect(submitBtn).toBeDisabled();

    // Enter valid IP and notes
    fireEvent.change(ipInput, { target: { value: "203.0.113.55" } });
    const notesInput = screen.getByTestId("input-rule-notes");
    fireEvent.change(notesInput, { target: { value: "Spam bot identified in access logs" } });

    expect(submitBtn).not.toBeDisabled();

    mockApiFetch.mockResolvedValueOnce({ success: true, rule: { id: "ip-rule-99" } });
    fireEvent.click(submitBtn);

    await waitFor(() => {
      expect(mockApiFetch).toHaveBeenCalledWith("/api/v1/client/firewall/rules", expect.objectContaining({
        method: "POST",
        body: JSON.stringify({
          domain: "example.com",
          ip_cidr: "203.0.113.55",
          action: "block",
          notes: "Spam bot identified in access logs",
        }),
      }));
    });
  });

  it("ClientFirewallPage: deletes / unblocks IP rule", async () => {
    mockApiFetch.mockImplementation((url: string) => {
      if (url.includes("/api/v1/client/firewall/rules")) return Promise.resolve(mockClientRules);
      return Promise.resolve({});
    });

    render(<ClientFirewallPage />);

    await waitFor(() => {
      expect(screen.getByTestId("ip-rule-row-ip-rule-1")).toBeInTheDocument();
    });

    mockApiFetch.mockResolvedValueOnce({ success: true, id: "ip-rule-1" });
    const deleteBtn = screen.getByTestId("btn-delete-rule-ip-rule-1");
    fireEvent.click(deleteBtn);

    await waitFor(() => {
      expect(mockApiFetch).toHaveBeenCalledWith("/api/v1/client/firewall/rules/ip-rule-1", expect.objectContaining({
        method: "DELETE",
      }));
    });
  });

  it("AdminFirewallPage: edits the node's firewall policy (open ports, blocked addresses)", async () => {
    let policy: any = {
      open_ports: [
        { port: 80, proto: "tcp" },
        { port: 8080, proto: "tcp", from: ["203.0.113.5"] },
      ],
      blocked: ["198.51.100.22"],
    };
    mockApiFetch.mockImplementation((url: string, opts?: any) => {
      if (url === "/api/v1/admin/nodes") {
        return Promise.resolve({ items: [{ id: "node-1", name: "node-1", status: "online" }] });
      }
      if (url === "/api/v1/admin/nodes/node-1/firewall") {
        if (opts?.method === "PUT") policy = JSON.parse(opts.body);
        return Promise.resolve({ policy });
      }
      return Promise.resolve({});
    });

    render(<AdminFirewallPage />);

    await waitFor(() => {
      expect(screen.getByTestId("admin-firewall-title")).toBeInTheDocument();
      expect(screen.getByTestId("card-admin-firewall-stats")).toBeInTheDocument();
      expect(screen.getByTestId("port-row-80")).toBeInTheDocument();
      expect(screen.getByTestId("port-row-8080")).toBeInTheDocument();
    });

    // Close a port: the whole policy goes back without it
    fireEvent.click(screen.getByTestId("btn-close-port-8080"));
    await waitFor(() => {
      expect(policy.open_ports).toEqual([{ port: 80, proto: "tcp" }]);
    });

    // The blocked addresses
    fireEvent.click(screen.getByTestId("tab-firewall-bans"));
    await waitFor(() => {
      expect(screen.getByTestId("title-global-bans")).toBeInTheDocument();
      expect(screen.getByText("198.51.100.22")).toBeInTheDocument();
    });

    fireEvent.click(screen.getByTestId("btn-unban-198.51.100.22"));
    await waitFor(() => {
      expect(policy.blocked).toEqual([]);
    });
  });
});
