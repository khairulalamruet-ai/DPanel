import React from "react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { AdminServerToolsPage } from "./AdminServerToolsPage";
import { RebootDialog } from "./RebootDialog";
import { TimezoneCard } from "./TimezoneCard";
import { SwapCard } from "./SwapCard";
import { DropCachesCard } from "./DropCachesCard";
import { SshConfigCard } from "./SshConfigCard";

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

describe("Server Tools & Maintenance UI (T10.15)", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  const mockOverview = {
    node_id: "node-1",
    hostname: "node-1.builtpanel.internal",
    os: "Ubuntu 24.04.1 LTS",
    kernel: "6.8.0-45-generic",
    uptime_seconds: 432000,
    uptime_human: "5 days, 4 hours",
    current_time: "2026-11-13 11:45:00 UTC",
    timezone: "UTC",
    memory: {
      total_mb: 8192,
      free_mb: 1840,
      available_mb: 4620,
      buffers_mb: 320,
      cached_mb: 2460,
    },
    swap: {
      total_mb: 4096,
      used_mb: 256,
      free_mb: 3840,
      swappiness: 60,
    },
  };

  const mockSshConfig = {
    port: 22,
    permit_root_login: "no" as const,
    password_authentication: false,
    pubkey_authentication: true,
    max_auth_tries: 3,
    authorized_keys_count: 2,
    ciphers: ["chacha20-poly1305@openssh.com"],
    security_rating: "hardened" as const,
  };

  const mockAuditEvents = [
    {
      id: "srv-aud-01",
      action: "server.set_timezone",
      actor: "admin",
      details: "Timezone set to UTC",
      created_at: "2026-11-12 10:15:22 UTC",
      status: "success" as const,
    },
  ];

  it("renders AdminServerToolsPage and displays server health overview and cards", async () => {
    mockApiFetch.mockImplementation((url: string) => {
      if (url === "/api/v1/admin/nodes") {
        return Promise.resolve({ items: [{ id: "node-1", name: "node-1", status: "online" }] });
      }
      if (url === "/api/v1/admin/nodes/node-1/server") {
        return Promise.resolve({
          hostname: mockOverview.hostname,
          os: mockOverview.os,
          kernel: mockOverview.kernel,
          uptime_seconds: mockOverview.uptime_seconds,
          timezone: mockOverview.timezone,
          memory_total_bytes: mockOverview.memory.total_mb * 1048576,
          memory_available_bytes: mockOverview.memory.available_mb * 1048576,
          swap_total_bytes: mockOverview.swap.total_mb * 1048576,
          swap_used_bytes: mockOverview.swap.used_mb * 1048576,
          ssh: [
            { name: "Port", value: "22" },
            { name: "PermitRootLogin", value: "no" },
            { name: "PasswordAuthentication", value: "no" },
          ],
        });
      }
      if (url.startsWith("/api/v1/admin/audit-logs")) {
        return Promise.resolve({
          items: mockAuditEvents.map((e) => ({ id: e.id, action: e.action, at: e.created_at, outcome: "success", metadata: {} })),
        });
      }
      return Promise.resolve({});
    });

    render(<AdminServerToolsPage />);

    await waitFor(() => {
      expect(screen.getByTestId("server-tools-title")).toBeInTheDocument();
      expect(screen.getByTestId("text-node-hostname")).toHaveTextContent("node-1.builtpanel.internal");
      expect(screen.getByTestId("title-timezone-config")).toBeInTheDocument();
      expect(screen.getByTestId("title-swap-management")).toBeInTheDocument();
      expect(screen.getByTestId("title-drop-caches")).toBeInTheDocument();
      expect(screen.getByTestId("title-ssh-config")).toBeInTheDocument();
      expect(screen.getByTestId("title-server-audit-trail")).toBeInTheDocument();
    });
  });

  it("TimezoneCard: validates selection and sudo password before update", async () => {
    const onUpdate = vi.fn().mockResolvedValue(undefined);
    render(
      <TimezoneCard
        currentTimezone="UTC"
        currentTimeStr="2026-11-13 11:45:00 UTC"
        onUpdateTimezone={onUpdate}
      />
    );

    const submitBtn = screen.getByTestId("btn-save-timezone");
    expect(submitBtn).toBeDisabled();

    // Select different timezone
    const select = screen.getByTestId("select-timezone");
    fireEvent.change(select, { target: { value: "Asia/Dhaka" } });

    // Still disabled without sudo password
    expect(submitBtn).toBeDisabled();

    // Enter sudo password
    const sudoInput = screen.getByTestId("input-tz-sudo");
    fireEvent.change(sudoInput, { target: { value: "SecretSudo123!" } });

    expect(submitBtn).not.toBeDisabled();
    fireEvent.click(submitBtn);

    await waitFor(() => {
      expect(onUpdate).toHaveBeenCalledWith("Asia/Dhaka", "SecretSudo123!");
    });
  });

  it("SwapCard: supports quick preset buttons and applies swap configuration", async () => {
    const onUpdate = vi.fn().mockResolvedValue(undefined);
    render(
      <SwapCard
        swap={mockOverview.swap}
        onUpdateSwap={onUpdate}
      />
    );

    const submitBtn = screen.getByTestId("btn-save-swap");
    expect(submitBtn).toBeDisabled();

    // Click 8GB preset
    const preset8gb = screen.getByTestId("btn-swap-preset-8192");
    fireEvent.click(preset8gb);

    // Enter sudo password
    const sudoInput = screen.getByTestId("input-swap-sudo");
    fireEvent.change(sudoInput, { target: { value: "AdminSudoPass!" } });

    expect(submitBtn).not.toBeDisabled();
    fireEvent.click(submitBtn);

    await waitFor(() => {
      expect(onUpdate).toHaveBeenCalledWith(8192, 60, "AdminSudoPass!");
    });
  });

  it("DropCachesCard: executes drop caches with sudo password", async () => {
    const onDrop = vi.fn().mockResolvedValue(undefined);
    render(
      <DropCachesCard
        memory={mockOverview.memory}
        onDropCaches={onDrop}
      />
    );

    const dropBtn = screen.getByTestId("btn-execute-drop-caches");
    expect(dropBtn).toBeDisabled();

    const sudoInput = screen.getByTestId("input-drop-caches-sudo");
    fireEvent.change(sudoInput, { target: { value: "SudoSecret!" } });

    expect(dropBtn).not.toBeDisabled();
    fireEvent.click(dropBtn);

    await waitFor(() => {
      expect(onDrop).toHaveBeenCalledWith("SudoSecret!");
    });
  });

  it("SshConfigCard: renders security rating and hardened parameters", () => {
    render(<SshConfigCard sshConfig={mockSshConfig} />);

    expect(screen.getByTestId("title-ssh-config")).toBeInTheDocument();
    expect(screen.getByTestId("badge-ssh-rating")).toHaveTextContent("serverTools.rating.hardened");
    expect(screen.getByTestId("text-ssh-port")).toHaveTextContent("22");
    expect(screen.getByTestId("text-ssh-root-login")).toHaveTextContent("no");
    expect(screen.getByTestId("text-ssh-password-auth")).toHaveTextContent("disabled");
  });

  it("RebootDialog: gates execution until typed confirmation is REBOOT and sudo password provided", async () => {
    const onConfirm = vi.fn().mockResolvedValue(undefined);
    render(
      <RebootDialog
        open={true}
        onOpenChange={vi.fn()}
        nodeHostname="node-1.builtpanel.internal"
        onConfirm={onConfirm}
      />
    );

    const rebootBtn = screen.getByTestId("btn-confirm-reboot");
    expect(rebootBtn).toBeDisabled();

    // Wrong text
    const textInput = screen.getByTestId("input-reboot-confirm");
    fireEvent.change(textInput, { target: { value: "restart" } });
    const sudoInput = screen.getByTestId("input-reboot-sudo");
    fireEvent.change(sudoInput, { target: { value: "RootPass!" } });
    expect(rebootBtn).toBeDisabled();

    // Correct text "REBOOT"
    fireEvent.change(textInput, { target: { value: "REBOOT" } });
    expect(rebootBtn).not.toBeDisabled();

    fireEvent.click(rebootBtn);
    await waitFor(() => {
      expect(onConfirm).toHaveBeenCalledWith("RootPass!");
    });
  });
});
