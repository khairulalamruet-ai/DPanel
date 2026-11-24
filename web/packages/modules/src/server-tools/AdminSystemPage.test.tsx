import React from "react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { AdminSystemPage } from "./AdminSystemPage";
import { ServerTimeTab } from "./tabs/ServerTimeTab";
import { SshAccessTab } from "./tabs/SshAccessTab";
import { ServerRebootTab } from "./tabs/ServerRebootTab";
import { SwapTab } from "./tabs/SwapTab";
import { ServerMigrateTab } from "./tabs/ServerMigrateTab";
import { MultiServerTab } from "./tabs/MultiServerTab";

// Mock @builtpanel/api
const mockApiFetch = vi.fn();
vi.mock("@builtpanel/api", () => ({
  apiFetch: (...args: any[]) => mockApiFetch(...args),
}));

// Mock @builtpanel/auth
let mockPerms = ["admin.server.read", "admin.server.manage"];
vi.mock("@builtpanel/auth", () => ({
  useSession: () => ({
    hasPermission: (perm: string) => mockPerms.includes(perm),
    data: { user: { role: "admin" } },
  }),
}));

// Mock @builtpanel/i18n
vi.mock("@builtpanel/i18n", () => ({
  useTranslation: () => ({
    t: (key: string, defaultOrParams?: string | Record<string, any>, params?: Record<string, any>) => {
      const fallback = typeof defaultOrParams === "string" ? defaultOrParams : key;
      const p = typeof defaultOrParams === "object" ? defaultOrParams : params;
      if (p) {
        let str = fallback;
        for (const [k, v] of Object.entries(p)) {
          str = str.replace(`{{${k}}}`, String(v));
        }
        return str;
      }
      return fallback;
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

describe("Admin System Page Group with 6 Tabs (T10.15)", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockPerms = ["admin.server.read", "admin.server.manage"];
  });

  const mockNodeServer = {
    hostname: "node-1.builtpanel.internal",
    os: "Ubuntu 24.04.1 LTS",
    kernel: "6.8.0-45-generic",
    uptime_seconds: 432000,
    timezone: "UTC",
    memory_total_bytes: 8192 * 1048576,
    memory_available_bytes: 4620 * 1048576,
    swap_total_bytes: 4096 * 1048576,
    swap_used_bytes: 256 * 1048576,
    ssh: [
      { name: "Port", value: "22" },
      { name: "PermitRootLogin", value: "no" },
      { name: "PasswordAuthentication", value: "no" },
      { name: "PubkeyAuthentication", value: "yes" },
      { name: "MaxAuthTries", value: "6" },
    ],
  };

  const setupDefaultMocks = () => {
    mockApiFetch.mockImplementation((url: string, opts?: any) => {
      if (url === "/api/v1/admin/nodes" && (!opts || opts.method === "GET")) {
        return Promise.resolve({
          items: [{ id: "node-1", name: "node-1", hostname: "node-1.builtpanel.internal", status: "online", agent_version: "0.1.0" }],
        });
      }
      if (url === "/api/v1/admin/nodes/node-1/server") {
        return Promise.resolve(mockNodeServer);
      }
      if (url === "/api/v1/admin/nodes/node-1/ssh") {
        return Promise.resolve({
          port: 22,
          permit_root_login: "no",
          password_authentication: false,
          pubkey_authentication: true,
          max_auth_tries: 6,
        });
      }
      if (url === "/api/v1/admin/nodes/node-1/ssh/keys") {
        return Promise.resolve({
          items: [
            {
              id: "key-01",
              name: "admin-workstation",
              key_type: "ssh-ed25519",
              fingerprint: "SHA256:abcd1234",
              public_key: "ssh-ed25519 AAAAC3... admin@workstation",
              created_at: "2026-11-20T10:00:00Z",
            },
          ],
        });
      }
      if (url === "/api/v1/admin/nodes/node-1/ssh/raw") {
        return Promise.resolve({ raw: "Port 22\nPermitRootLogin no\n" });
      }
      if (url === "/api/v1/admin/nodes/node-1/migrate/status") {
        return Promise.resolve({
          id: "mig-idle",
          status: "idle",
          progress: 0,
          current_step: "Ready",
          steps: [],
          logs: [],
        });
      }
      if (url.startsWith("/api/v1/admin/audit-logs")) {
        return Promise.resolve({
          items: [
            {
              id: "aud-01",
              action: "server.set_timezone",
              actor_id: "admin",
              at: "2026-11-24T12:00:00Z",
              outcome: "success",
              metadata: { timezone: "UTC" },
            },
          ],
        });
      }
      if (url === "/api/v1/auth/sudo") {
        return Promise.resolve({ success: true });
      }
      return Promise.resolve({});
    });
  };

  it("renders AdminSystemPage with all 6 tabs, node selector, overview metrics, and audit log", async () => {
    setupDefaultMocks();
    render(<AdminSystemPage />);

    await waitFor(() => {
      expect(screen.getByTestId("system-page-title")).toBeInTheDocument();
      expect(screen.getByTestId("text-node-hostname")).toHaveTextContent("node-1.builtpanel.internal");
      expect(screen.getByTestId("tab-system-time")).toBeInTheDocument();
      expect(screen.getByTestId("tab-system-ssh")).toBeInTheDocument();
      expect(screen.getByTestId("tab-system-reboot")).toBeInTheDocument();
      expect(screen.getByTestId("tab-system-swap")).toBeInTheDocument();
      expect(screen.getByTestId("tab-system-migrate")).toBeInTheDocument();
      expect(screen.getByTestId("tab-system-multi-server")).toBeInTheDocument();
      expect(screen.getByTestId("title-server-audit-trail")).toBeInTheDocument();
    });
  });

  it("Tab 1 - Server Time: supports searchable timezone filtering and sudo update", async () => {
    setupDefaultMocks();
    render(<AdminSystemPage />);

    await waitFor(() => {
      expect(screen.getByTestId("title-timezone-config")).toBeInTheDocument();
    });

    // Search for Dhaka
    const searchInput = screen.getByTestId("input-timezone-search");
    fireEvent.change(searchInput, { target: { value: "Dhaka" } });

    // Option should be visible
    const dhakaOption = screen.getByTestId("tz-option-Asia/Dhaka");
    expect(dhakaOption).toBeInTheDocument();
    fireEvent.click(dhakaOption);

    // Save button becomes active
    const applyBtn = screen.getByTestId("btn-apply-timezone");
    expect(applyBtn).not.toBeDisabled();
    fireEvent.click(applyBtn);

    // Sudo confirmation modal opens
    expect(screen.getByTestId("sudo-modal-title")).toBeInTheDocument();
    const sudoInput = screen.getByTestId("input-sudo-modal-password");
    fireEvent.change(sudoInput, { target: { value: "AdminSecret123!" } });

    const confirmBtn = screen.getByTestId("btn-sudo-modal-confirm");
    fireEvent.click(confirmBtn);

    await waitFor(() => {
      expect(mockApiFetch).toHaveBeenCalledWith(
        "/api/v1/auth/sudo",
        expect.objectContaining({ method: "POST" }),
      );
      expect(mockApiFetch).toHaveBeenCalledWith(
        "/api/v1/admin/nodes/node-1/server/timezone",
        expect.objectContaining({
          method: "POST",
          body: JSON.stringify({ zone: "Asia/Dhaka" }),
        }),
      );
    });
  });

  it("Tab 2 - SSH Access: displays lockout warning, switches sub-tabs, and manages keys", async () => {
    setupDefaultMocks();
    render(<AdminSystemPage />);

    // Switch to SSH access tab
    const sshTab = await screen.findByTestId("tab-system-ssh");
    fireEvent.click(sshTab);

    // Mandatory Rule: Lock-out warning must be prominently displayed
    await waitFor(() => {
      expect(screen.getByTestId("ssh-lockout-warning")).toBeInTheDocument();
      expect(screen.getByTestId("card-ssh-basic")).toBeInTheDocument();
    });

    // Sub-tab 2: Authorized Keys
    const keysSubtab = screen.getByTestId("subtab-ssh-keys");
    fireEvent.click(keysSubtab);

    await waitFor(() => {
      expect(screen.getByTestId("card-ssh-keys")).toBeInTheDocument();
      expect(screen.getByText("admin-workstation")).toBeInTheDocument();
    });

    // Open Add Key form
    const openAddBtn = screen.getByTestId("btn-open-add-key");
    fireEvent.click(openAddBtn);

    const keyNameInput = screen.getByTestId("input-new-key-name");
    const keyContentTextarea = screen.getByTestId("textarea-new-public-key");
    fireEvent.change(keyNameInput, { target: { value: "backup-key" } });
    fireEvent.change(keyContentTextarea, { target: { value: "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5... user@backup" } });

    const submitAddKey = screen.getByTestId("btn-submit-add-key");
    fireEvent.click(submitAddKey);

    // Verify Sudo Modal appears
    expect(screen.getByTestId("sudo-modal-title")).toBeInTheDocument();
    const sudoInput = screen.getByTestId("input-sudo-modal-password");
    fireEvent.change(sudoInput, { target: { value: "SudoSecret!" } });
    fireEvent.click(screen.getByTestId("btn-sudo-modal-confirm"));

    await waitFor(() => {
      expect(mockApiFetch).toHaveBeenCalledWith(
        "/api/v1/admin/nodes/node-1/ssh/keys",
        expect.objectContaining({ method: "POST" }),
      );
    });

    // Sub-tab 3: Advanced
    const advSubtab = screen.getByTestId("subtab-ssh-advanced");
    fireEvent.click(advSubtab);

    await waitFor(() => {
      expect(screen.getByTestId("card-ssh-advanced")).toBeInTheDocument();
      expect(screen.getByTestId("textarea-sshd-config")).toHaveValue("Port 22\nPermitRootLogin no\n");
    });
  });

  it("Tab 3 - Server Reboot: supports graceful vs hard options and shows 'server is coming back' state", async () => {
    setupDefaultMocks();
    render(<AdminSystemPage />);

    const rebootTab = await screen.findByTestId("tab-system-reboot");
    fireEvent.click(rebootTab);

    await waitFor(() => {
      expect(screen.getByTestId("card-server-reboot")).toBeInTheDocument();
    });

    // Select Hard Reboot option
    const hardOpt = screen.getByTestId("option-reboot-hard");
    fireEvent.click(hardOpt);

    // Type confirmation
    const confirmInput = screen.getByTestId("input-reboot-confirm");
    fireEvent.change(confirmInput, { target: { value: "node-1" } });

    // Sudo password
    const sudoInput = screen.getByTestId("input-reboot-sudo");
    fireEvent.change(sudoInput, { target: { value: "RebootSudo!" } });

    // Submit reboot
    const rebootBtn = screen.getByTestId("btn-execute-reboot");
    expect(rebootBtn).not.toBeDisabled();
    fireEvent.click(rebootBtn);

    // Rule: Then show "server is coming back"
    await waitFor(() => {
      expect(mockApiFetch).toHaveBeenCalledWith(
        "/api/v1/admin/nodes/node-1/server/reboot",
        expect.objectContaining({
          method: "POST",
          body: JSON.stringify({ confirm: "node-1", mode: "hard" }),
        }),
      );
      expect(screen.getByTestId("text-server-coming-back")).toBeInTheDocument();
    });
  });

  it("Tab 4 - Swap: displays metrics, quick presets, drop swap, and drop cache", async () => {
    setupDefaultMocks();
    render(<AdminSystemPage />);

    const swapTab = await screen.findByTestId("tab-system-swap");
    fireEvent.click(swapTab);

    await waitFor(() => {
      expect(screen.getByTestId("card-swap-config")).toBeInTheDocument();
      expect(screen.getByTestId("card-drop-caches")).toBeInTheDocument();
    });

    // Drop swap button (0 MB)
    const dropSwapBtn = screen.getByTestId("btn-swap-preset-0");
    fireEvent.click(dropSwapBtn);

    const saveSwapBtn = screen.getByTestId("btn-save-swap");
    fireEvent.click(saveSwapBtn);

    // Sudo modal
    expect(screen.getByTestId("sudo-modal-title")).toBeInTheDocument();
    const sudoInput = screen.getByTestId("input-sudo-modal-password");
    fireEvent.change(sudoInput, { target: { value: "SwapSudoPass!" } });
    fireEvent.click(screen.getByTestId("btn-sudo-modal-confirm"));

    await waitFor(() => {
      expect(mockApiFetch).toHaveBeenCalledWith(
        "/api/v1/admin/nodes/node-1/server/swap",
        expect.objectContaining({
          method: "POST",
          body: JSON.stringify({ megabytes: 0 }),
        }),
      );
    });

    // Drop page cache
    const dropCacheBtn = screen.getByTestId("btn-drop-caches");
    fireEvent.click(dropCacheBtn);

    const sudoInput2 = screen.getByTestId("input-sudo-modal-password");
    fireEvent.change(sudoInput2, { target: { value: "CacheSudoPass!" } });
    fireEvent.click(screen.getByTestId("btn-sudo-modal-confirm"));

    await waitFor(() => {
      expect(mockApiFetch).toHaveBeenCalledWith(
        "/api/v1/admin/nodes/node-1/server/drop-caches",
        expect.objectContaining({ method: "POST" }),
      );
    });
  });

  it("Tab 5 - Migrate: clears remote password immediately after submit and displays progress logs", async () => {
    setupDefaultMocks();
    render(<AdminSystemPage />);

    const migrateTab = await screen.findByTestId("tab-system-migrate");
    fireEvent.click(migrateTab);

    await waitFor(() => {
      expect(screen.getByTestId("card-server-migrate")).toBeInTheDocument();
    });

    const hostInput = screen.getByTestId("input-migrate-host");
    const userInput = screen.getByTestId("input-migrate-user");
    const passwordInput = screen.getByTestId("input-migrate-password");

    fireEvent.change(hostInput, { target: { value: "target.cluster.internal" } });
    fireEvent.change(userInput, { target: { value: "root" } });
    fireEvent.change(passwordInput, { target: { value: "SuperSecretRemotePass!" } });

    const startBtn = screen.getByTestId("btn-start-migration");
    expect(startBtn).not.toBeDisabled();
    fireEvent.click(startBtn);

    // Mandatory Rule: Remote password lives ONLY in form field in memory. Cleared immediately after submit!
    expect(passwordInput).toHaveValue("");

    // Sudo confirmation modal
    expect(screen.getByTestId("sudo-modal-title")).toBeInTheDocument();
    const sudoInput = screen.getByTestId("input-sudo-modal-password");
    fireEvent.change(sudoInput, { target: { value: "AdminSudoPass!" } });

    // Mock response when migration starts
    mockApiFetch.mockImplementation((url: string) => {
      if (url === "/api/v1/auth/sudo") return Promise.resolve({ success: true });
      if (url === "/api/v1/admin/nodes/node-1/migrate") return Promise.resolve({ id: "mig-123", message: "Started" });
      if (url === "/api/v1/admin/nodes/node-1/migrate/status") {
        return Promise.resolve({
          id: "mig-123",
          status: "running",
          progress: 40,
          current_step: "Transferring databases",
          steps: [
            { id: "s1", name: "SSH connection", status: "completed" },
            { id: "s2", name: "Transferring databases", status: "in_progress" },
          ],
          logs: ["[2026-11-24] Connected to target.cluster.internal", "[2026-11-24] Syncing MySQL data..."],
        });
      }
      if (url === "/api/v1/admin/nodes/node-1/server") {
        return Promise.resolve({
          timezone: "UTC",
          system_time: "2026-11-24T12:00:00Z",
          uptime_seconds: 123456,
          overview: {
            node_id: "node-1",
            hostname: "test-node.local",
            os_version: "Ubuntu 24.04 LTS",
            kernel_version: "6.8.0-generic",
            system_time: "2026-11-24T12:00:00Z",
            timezone: "UTC",
            uptime_seconds: 123456,
            load_averages: [0.12, 0.08, 0.05],
          },
          swap: { total_mb: 2048, used_mb: 512, free_mb: 1536, swappiness: 60 },
        });
      }
      if (url === "/api/v1/admin/nodes/node-1/audit") return Promise.resolve([]);
      return Promise.resolve({});
    });

    fireEvent.click(screen.getByTestId("btn-sudo-modal-confirm"));

    await waitFor(() => {
      expect(screen.getByTestId("card-migration-progress")).toBeInTheDocument();
      expect(screen.getByTestId("migration-log-box")).toHaveTextContent("Connected to target.cluster.internal");
    });
  });

  it("Tab 6 - Multi-server: generates join code, shows install command, and adds server", async () => {
    setupDefaultMocks();
    mockApiFetch.mockImplementation((url: string, opts?: any) => {
      if (url === "/api/v1/admin/nodes" && (!opts || opts.method === "GET")) {
        return Promise.resolve({
          items: [{ id: "node-1", name: "node-master", hostname: "master.builtpanel.internal", status: "online", agent_version: "0.1.0" }],
        });
      }
      if (url === "/api/v1/admin/nodes/join-code") {
        return Promise.resolve({
          code: "dhn_join_test123",
          expires_at: "2026-11-24T12:15:00Z",
          install_command: "curl -sSL https://get.builtpanel.com/agent.sh | sudo bash -s -- --token dhn_join_test123",
        });
      }
      if (url === "/api/v1/admin/nodes/join") {
        return Promise.resolve({ success: true, node_id: "node-2" });
      }
      if (url === "/api/v1/auth/sudo") {
        return Promise.resolve({ success: true });
      }
      return Promise.resolve({});
    });

    render(<AdminSystemPage />);

    const multiTab = await screen.findByTestId("tab-system-multi-server");
    fireEvent.click(multiTab);

    await waitFor(() => {
      expect(screen.getByTestId("card-generate-join-code")).toBeInTheDocument();
      expect(screen.getByTestId("card-join-server")).toBeInTheDocument();
      expect(screen.getByTestId("card-cluster-nodes")).toBeInTheDocument();
    });

    // Generate Join Code
    const generateBtn = screen.getByTestId("btn-generate-join-code");
    fireEvent.click(generateBtn);

    await waitFor(() => {
      expect(screen.getByTestId("input-generated-code")).toHaveValue("dhn_join_test123");
      expect(screen.getByTestId("input-generated-cmd")).toHaveValue(
        "curl -sSL https://get.builtpanel.com/agent.sh | sudo bash -s -- --token dhn_join_test123",
      );
    });

    // Add / Join server
    const addrInput = screen.getByTestId("input-join-address");
    const nameInput = screen.getByTestId("input-join-name");
    const codeInput = screen.getByTestId("input-join-code");

    fireEvent.change(addrInput, { target: { value: "198.51.100.80" } });
    fireEvent.change(nameInput, { target: { value: "node-worker-2" } });
    fireEvent.change(codeInput, { target: { value: "dhn_join_token_ext" } });

    const joinBtn = screen.getByTestId("btn-submit-join");
    expect(joinBtn).not.toBeDisabled();
    fireEvent.click(joinBtn);

    // Sudo confirmation
    expect(screen.getByTestId("sudo-modal-title")).toBeInTheDocument();
    const sudoInput = screen.getByTestId("input-sudo-modal-password");
    fireEvent.change(sudoInput, { target: { value: "JoinSudoPass!" } });
    fireEvent.click(screen.getByTestId("btn-sudo-modal-confirm"));

    await waitFor(() => {
      expect(mockApiFetch).toHaveBeenCalledWith(
        "/api/v1/admin/nodes/join",
        expect.objectContaining({
          method: "POST",
          body: JSON.stringify({
            address: "198.51.100.80",
            name: "node-worker-2",
            join_code: "dhn_join_token_ext",
          }),
        }),
      );
    });
  });

  it("Permissions: denies viewing when admin lacks 'admin.server.read'", async () => {
    mockPerms = []; // no permissions
    setupDefaultMocks();
    render(<AdminSystemPage />);

    await waitFor(() => {
      expect(screen.getByTestId("permission-denied-server")).toBeInTheDocument();
    });
  });
});
