import { describe, it, expect, vi, beforeEach } from "vitest";
import React from "react";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { NodesPage } from "./NodesPage";
import { ToastProvider } from "@builtpanel/ui";
import * as builtpanelApi from "@builtpanel/api";

let mockHasPermission = true;

vi.mock("@builtpanel/auth", () => ({
  useSession: () => ({
    data: {
      user: { id: "00000000-0000-0000-0000-000000000001", username: "admin" },
      roles: ["super_admin"],
      permissions: ["*"],
    },
    isLoading: false,
    hasPermission: (_perm: string) => mockHasPermission,
    hasRole: (_role: string) => true,
  }),
}));

vi.mock("@builtpanel/i18n", () => ({
  useTranslation: () => ({
    t: (key: string) => {
      const dict: Record<string, string> = {
        "nodes.title": "Cluster Nodes",
        "nodes.desc": "Manage host servers running the node-agent daemon.",
        "nodes.addNode": "Add Node",
        "nodes.status.online": "Online",
        "nodes.status.offline": "Offline",
        "nodes.status.draining": "Draining",
        "nodes.status.revoked": "Revoked",
        "nodes.table.node": "Node / Hostname",
        "nodes.table.status": "Status",
        "nodes.table.saturation": "Fullness (%)",
        "nodes.table.cpu": "CPU",
        "nodes.table.ram": "RAM",
        "nodes.table.disk": "Disk",
        "nodes.table.version": "Agent / Docker",
        "nodes.table.lastSeen": "Last Seen",
        "nodes.table.actions": "Actions",
        "nodes.table.viewDetails": "View Specs",
        "nodes.table.editCapacity": "Capacity / Oversell",
        "nodes.table.drain": "Drain",
        "nodes.table.undrain": "Undrain",
        "nodes.table.revoke": "Revoke",
        "nodes.capacity.title": "Node Capacity & Saturation",
        "nodes.capacity.totalNodes": "Total Nodes",
        "nodes.capacity.saturation": "Saturation",
        "nodes.capacity.fullness": "Fullness",
        "nodes.capacity.tenants": "Tenants",
        "nodes.capacity.clusterFullness": "Cluster Fullness",
        "nodes.capacity.avgSaturation": "Avg Saturation",
        "nodes.capacity.configureOversell": "Configure Oversell",
        "nodes.capacity.oversellSettings": "Oversell Settings",
        "nodes.capacity.oversellDesc": "Configure CPU, RAM, and Disk oversell allocation multipliers for this node.",
        "nodes.capacity.cpuRatioLabel": "CPU Oversell Multiplier",
        "nodes.capacity.ramRatioLabel": "RAM Oversell Multiplier",
        "nodes.capacity.diskRatioLabel": "Disk Oversell Multiplier",
        "nodes.capacity.maxTenantsLabel": "Maximum Tenants Capacity",
        "nodes.capacity.saveSuccess": "Node capacity & oversell settings updated successfully.",
        "nodes.capacity.drainSuccess": "Node set to draining mode. No new tenants will be scheduled.",
        "nodes.capacity.undrainSuccess": "Node restored to active service.",
        "nodes.scheduler.title": "Placement Scheduler & Policies",
        "nodes.scheduler.desc": "Configure automated cluster placement policy for incoming tenants",
        "nodes.scheduler.configure": "Placement Policy",
        "nodes.scheduler.policyLabel": "Active Placement Algorithm",
        "nodes.scheduler.leastLoadedTitle": "Least-Loaded (Resource Saturation)",
        "nodes.scheduler.leastLoadedDesc": "Automatically routes new tenants to the node with the lowest resource load",
        "nodes.scheduler.roundRobinTitle": "Round-Robin (Cyclic Distribution)",
        "nodes.scheduler.roundRobinDesc": "Distributes new tenants sequentially",
        "nodes.scheduler.drainInvariantTitle": "Drainage Invariant",
        "nodes.scheduler.drainInvariantDesc": "Nodes in draining or maintenance mode are strictly disqualified",
        "nodes.scheduler.simulationTitle": "Live Placement Simulation",
        "nodes.scheduler.nextTarget": "Next Scheduled Destination",
        "nodes.scheduler.excludedNodesTitle": "Disqualified Draining / Offline Nodes",
        "nodes.scheduler.saveSuccess": "Placement scheduler policy updated successfully.",
        "nodes.scheduler.drain": "Drain",
        "nodes.scheduler.undrain": "Undrain",
        "nodes.scheduler.drainSuccess": "Node set to draining mode.",
        "nodes.scheduler.undrainSuccess": "Node restored to active cluster service.",
        "common.save": "Save Changes",
        "nodes.enrollments.title": "Enrollment Tokens",
        "nodes.enrollments.desc": "Single-use tokens issued for server onboarding.",
        "nodes.enrollments.name": "Node Name",
        "nodes.enrollments.status": "Token Status",
        "nodes.enrollments.created": "Created",
        "nodes.enrollments.expires": "Expires",
        "nodes.enrollments.cancel": "Cancel",
        "nodes.enrollments.cancelSuccess": "Enrollment token cancelled successfully.",
        "nodes.enrollments.statusPending": "Pending",
        "nodes.wizard.title": "Enroll New Node",
        "nodes.wizard.desc": "Generate a single-use enrollment token.",
        "nodes.wizard.nameLabel": "Node Name",
        "nodes.wizard.namePlaceholder": "e.g. node-sgp-01",
        "nodes.wizard.generateToken": "Generate Enrollment Token",
        "nodes.wizard.generating": "Generating...",
        "nodes.wizard.tokenLabel": "One-time Enrollment Token",
        "nodes.wizard.expiresIn": "Valid for 15 minutes",
        "nodes.wizard.commandLabel": "Run this command on your server:",
        "nodes.wizard.copyCommand": "Copy Command",
        "nodes.wizard.copyToken": "Copy Token",
        "nodes.wizard.copied": "Copied!",
        "nodes.wizard.sudoPrompt": "Administrative sudo verification required. Please enter password:",
        "nodes.wizard.passwordLabel": "Password",
        "nodes.wizard.verifySudo": "Verify Password",
        "nodes.wizard.done": "Done & Refresh",
        "nodes.details.title": "Node Specifications & Status",
        "nodes.details.docker": "Docker Engine",
        "nodes.details.cpuModel": "CPU Model",
        "nodes.details.securityFeatures": "Security Sandboxing",
        "nodes.details.fingerprint": "Certificate Fingerprint",
        "nodes.details.close": "Close",
        "nodes.revoke.title": "Revoke Node Certificate",
        "nodes.revoke.desc": "Are you sure you want to revoke this node?",
        "nodes.revoke.confirmPrompt": "Type the node name to confirm revocation:",
        "nodes.revoke.success": "Node revoked successfully",
        "nodes.update.actionBtn": "Update Agent",
        "nodes.update.dialogTitle": "Signed Agent Self-Update",
        "nodes.update.dialogDesc": "Cryptographically verify and apply signed release updates",
        "nodes.update.currentNodeLabel": "Target Node",
        "nodes.update.targetReleaseLabel": "Target Release",
        "nodes.update.manifestTitle": "Signed Release Manifest",
        "nodes.update.corruptSigLabel": "Fault Injection: Simulate Invalid Signature",
        "nodes.update.corruptSigDesc": "Simulate corrupted signature",
        "nodes.update.rejectedTitle": "Signature Verification Failed",
        "nodes.update.rejectedError": "ERR_SIGNATURE_MISMATCH: Agent binary signature verification failed. Update rejected.",
        "nodes.update.verifiedTitle": "Cryptographic Signature Verified",
        "nodes.update.verifiedDesc": "Agent binary successfully verified and upgraded to v0.2.0.",
        "nodes.update.applyBtn": "Apply Signed Update",
        "nodes.update.verifying": "Verifying Signature...",
        "nodes.update.successToast": "Agent self-update verified and applied successfully.",
        "nodes.health.alertTitle": "Node Offline Alert (Heartbeat missed > 60s)",
        "nodes.health.alertDesc": "Node {{nodes}} is unreachable. Failover may be initiated.",
        "nodes.health.triggerFailover": "Trigger Failover",
        "nodes.health.failoverModalTitle": "Emergency Node Failover",
        "nodes.health.failoverModalDesc": "Evacuate and failover hosted tenants",
        "nodes.health.sourceNode": "Offline Source Node",
        "nodes.health.targetNode": "Target Destination Node",
        "nodes.health.executeFailover": "Execute Failover",
        "nodes.health.failoverSuccess": "Failover initiated successfully",
        "common.loading": "Loading...",
        "common.reload": "Reload",
        "common.cancel": "Cancel",
        "common.error": "An error occurred",
        "common.noPermission": "Permission denied",
      };
      return dict[key] || key;
    },
  }),
  useFormatters: () => ({
    formatDate: (d: Date) => d.toISOString(),
  }),
}));

const mockNodesList = [
  {
    id: "node_01",
    name: "node-master-01",
    hostname: "cp1.builtpanel.net",
    ip_address: "167.104.101.81",
    status: "online" as const,
    version: "0.1.0",
    fingerprint: "SHA256:7a8b9c0d1e2f3a4b5c6d7e8f",
    last_seen_at: "2026-11-12T00:00:00Z",
    enrolled_at: "2026-11-01T00:00:00Z",
    capabilities: {
      cpu_cores: 8,
      cpu_model: "AMD EPYC 7763",
      cpu_usage_pct: 18,
      ram_total_bytes: 16 * 1024 * 1024 * 1024,
      ram_used_bytes: 5.4 * 1024 * 1024 * 1024,
      disk_total_bytes: 250 * 1024 * 1024 * 1024,
      disk_used_bytes: 52 * 1024 * 1024 * 1024,
      kernel: "Linux 6.8.0-45-generic",
      docker_version: "27.3.1",
      security_features: ["seccomp", "apparmor"],
    },
    capacity: {
      allocated_cpu_cores: 12,
      allocated_ram_bytes: 18 * 1024 * 1024 * 1024,
      allocated_disk_bytes: 160 * 1024 * 1024 * 1024,
      tenants_count: 24,
      max_tenants: 50,
      saturation_pct: 68,
      oversell: {
        cpu_ratio: 2.0,
        ram_ratio: 1.25,
        disk_ratio: 1.0,
      },
    },
  },
  {
    id: "node_02",
    name: "node-worker-02",
    hostname: "worker2.builtpanel.net",
    ip_address: "167.104.101.82",
    status: "draining" as const,
    version: "0.1.0",
    fingerprint: "SHA256:1a2b3c4d5e6f",
    last_seen_at: "2026-11-11T20:00:00Z",
    enrolled_at: "2026-11-05T00:00:00Z",
    capabilities: {
      cpu_cores: 4,
      cpu_model: "Intel Xeon E-2288G",
      cpu_usage_pct: 0,
      ram_total_bytes: 8 * 1024 * 1024 * 1024,
      ram_used_bytes: 1.1 * 1024 * 1024 * 1024,
      disk_total_bytes: 100 * 1024 * 1024 * 1024,
      disk_used_bytes: 20 * 1024 * 1024 * 1024,
      kernel: "Linux 6.5.0",
      docker_version: "26.1.4",
      security_features: ["seccomp"],
    },
    capacity: {
      allocated_cpu_cores: 6,
      allocated_ram_bytes: 7 * 1024 * 1024 * 1024,
      allocated_disk_bytes: 80 * 1024 * 1024 * 1024,
      tenants_count: 48,
      max_tenants: 60,
      saturation_pct: 85,
      oversell: {
        cpu_ratio: 2.0,
        ram_ratio: 1.25,
        disk_ratio: 1.0,
      },
    },
  },
  {
    id: "node_03",
    name: "node-worker-03-backup",
    hostname: "worker3.builtpanel.net",
    ip_address: "167.104.101.83",
    status: "offline" as const,
    version: "0.1.0",
    fingerprint: "SHA256:3c4d5e6f7a8b",
    last_seen_at: "2026-11-10T12:00:00Z",
    enrolled_at: "2026-11-01T00:00:00Z",
    capabilities: {
      cpu_cores: 4,
      cpu_model: "Intel Xeon E-2288G",
      cpu_usage_pct: 0,
      ram_total_bytes: 8 * 1024 * 1024 * 1024,
      ram_used_bytes: 1.1 * 1024 * 1024 * 1024,
      disk_total_bytes: 100 * 1024 * 1024 * 1024,
      disk_used_bytes: 20 * 1024 * 1024 * 1024,
      kernel: "Linux 6.5.0",
      docker_version: "26.1.4",
      security_features: ["seccomp"],
    },
  },
];

/** A node as the backend returns it (`NodeView`). */
const view = (n: (typeof mockNodesList)[number]) => ({
  id: n.id,
  name: n.name,
  hostname: n.hostname,
  public_ip: n.ip_address,
  status: n.status,
  agent_version: n.version,
  capabilities: {
    cpu_cores: n.capabilities.cpu_cores,
    memory_bytes: n.capabilities.ram_total_bytes,
    disk_total_bytes: n.capabilities.disk_total_bytes,
    disk_free_bytes: n.capabilities.disk_total_bytes - n.capabilities.disk_used_bytes,
    kernel: n.capabilities.kernel,
    docker_version: n.capabilities.docker_version,
    seccomp: n.capabilities.security_features.includes("seccomp"),
    apparmor: n.capabilities.security_features.includes("apparmor"),
  },
  last_seen_at: n.last_seen_at,
  created_at: n.enrolled_at,
});

const mockEnrollmentsList = [
  {
    id: "token_01",
    name: "node-worker-pending",
    created_at: "2026-11-12T01:00:00Z",
    created_by: "00000000-0000-0000-0000-000000000001",
    expires_at: "2026-11-12T01:15:00Z",
    status: "pending" as const,
  },
];

describe("NodesPage", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    mockHasPermission = true;

    vi.spyOn(builtpanelApi, "apiFetch").mockImplementation(async (url: string, opts?: any) => {
      if (url === "/api/v1/admin/nodes" && (!opts || opts.method === "GET")) {
        return { items: mockNodesList.map(view) };
      }
      if (url === "/api/v1/admin/nodes/enrollments") {
        return { items: mockEnrollmentsList };
      }
      if (url === "/api/v1/admin/nodes" && opts?.method === "POST") {
        const body = JSON.parse(opts.body || "{}");
        return {
          id: "new_enroll_id",
          name: body.name || "node-test",
          token: "dhenroll_test123",
          expires_at: "2026-11-12T01:00:00Z",
          expires_in: 900,
          enroll_command: "curl -sSL https://get.builtpanel.com/agent.sh | sudo bash -s -- --token dhenroll_test123",
        };
      }
      if (url.includes("/enrollments/") && opts?.method === "DELETE") {
        return {};
      }
      if (url.includes("/revoke")) {
        return { success: true, message: "Node revoked successfully" };
      }
      if (url.includes("/release-manifest")) {
        return {
          version: "0.2.0",
          channel: "stable",
          signing_key_id: "ed25519-release-v1",
          signature: "ed25519:7a8b9c0d",
          checksum_sha256: "b94d27b9934d",
        };
      }
      if (url.includes("/update-agent") && opts?.method === "POST") {
        const body = JSON.parse(opts.body || "{}");
        if (body.corrupt_signature || body.manifest_signature?.includes("invalid")) {
          throw new Error("ERR_SIGNATURE_MISMATCH: Agent binary signature verification failed. Update rejected.");
        }
        return {
          success: true,
          version: "0.2.0",
          message: "Agent self-update verified and applied successfully",
        };
      }
      if (url.includes("/drain") || url.includes("/undrain")) {
        return { success: true, status: "draining" };
      }
      if (url.includes("/capacity") && opts?.method === "PUT") {
        return { success: true };
      }
      if (url.includes("/cluster/scheduler") && opts?.method === "PUT") {
        return { success: true };
      }
      if (url.includes("/failover")) {
        return { success: true, message: "Failover completed successfully", migrated_tenants: 12 };
      }
      return {};
    });
  });

  it("renders nodes table with status badges and capacity", async () => {
    render(
      <ToastProvider>
        <NodesPage />
      </ToastProvider>
    );

    expect(screen.getByRole("heading", { name: "Cluster Nodes" })).toBeInTheDocument();

    await waitFor(() => {
      expect(screen.getByText("node-master-01")).toBeInTheDocument();
      expect(screen.getByText("node-worker-02")).toBeInTheDocument();
      expect(screen.getByText("Online")).toBeInTheDocument();
      expect(screen.getByText("Draining")).toBeInTheDocument();
    });
  });

  it("renders enrollment tokens list and cancels an enrollment", async () => {
    render(
      <ToastProvider>
        <NodesPage />
      </ToastProvider>
    );

    await waitFor(() => {
      expect(screen.getByText("Enrollment Tokens")).toBeInTheDocument();
      expect(screen.getByText("node-worker-pending")).toBeInTheDocument();
      expect(screen.getByText("Pending")).toBeInTheDocument();
    });

    const cancelBtn = screen.getByRole("button", { name: /Cancel/i });
    fireEvent.click(cancelBtn);

    await waitFor(() => {
      expect(builtpanelApi.apiFetch).toHaveBeenCalledWith(
        expect.stringContaining("/api/v1/admin/nodes/enrollments/token_01"),
        expect.objectContaining({ method: "DELETE" })
      );
    });
  });

  it("opens add node wizard dialog, inputs name, and generates token with install command", async () => {
    const writeTextMock = vi.fn().mockResolvedValue(undefined);
    Object.assign(navigator, {
      clipboard: {
        writeText: writeTextMock,
      },
    });

    render(
      <ToastProvider>
        <NodesPage />
      </ToastProvider>
    );

    await waitFor(() => {
      expect(screen.getByText("node-master-01")).toBeInTheDocument();
    });

    const addBtn = screen.getByRole("button", { name: /Add Node/i });
    fireEvent.click(addBtn);

    await waitFor(() => {
      expect(screen.getByText("Enroll New Node")).toBeInTheDocument();
    });

    const nameInput = screen.getByPlaceholderText("e.g. node-sgp-01");
    fireEvent.change(nameInput, { target: { value: "node-worker-03" } });

    const generateBtn = screen.getByRole("button", { name: /Generate Enrollment Token/i });
    fireEvent.click(generateBtn);

    await waitFor(() => {
      expect(screen.getByText("dhenroll_test123")).toBeInTheDocument();
      expect(screen.getByText(/curl -sSL https:\/\/get\.builtpanel\.com\/agent\.sh/i)).toBeInTheDocument();
    });

    const copyBtn = screen.getByRole("button", { name: /Copy Command/i });
    fireEvent.click(copyBtn);

    expect(writeTextMock).toHaveBeenCalledWith(
      expect.stringContaining("dhenroll_test123")
    );
  });

  it("handles 403 reauth_required by prompting for sudo password", async () => {
    let callCount = 0;
    vi.spyOn(builtpanelApi, "apiFetch").mockImplementation(async (url: string, opts?: any) => {
      if (url === "/api/v1/admin/nodes" && (!opts || opts.method === "GET")) {
        return { items: mockNodesList.map(view) };
      }
      if (url === "/api/v1/admin/nodes/enrollments") {
        return { items: [] };
      }
      if (url === "/api/v1/admin/nodes" && opts?.method === "POST") {
        callCount++;
        if (callCount === 1) {
          const err: any = new Error("reauth_required");
          err.status = 403;
          err.details = { code: "reauth_required", title: "reauth_required" };
          throw err;
        }
        return {
          id: "new_enroll_id",
          name: "node-secured",
          token: "dhenroll_after_sudo",
          expires_at: "2026-11-12T01:00:00Z",
          expires_in: 900,
          enroll_command: null,
        };
      }
      if (url === "/api/v1/auth/sudo") {
        return {};
      }
      return {};
    });

    render(
      <ToastProvider>
        <NodesPage />
      </ToastProvider>
    );

    await waitFor(() => {
      expect(screen.getByText("node-master-01")).toBeInTheDocument();
    });

    fireEvent.click(screen.getByRole("button", { name: /Add Node/i }));

    const nameInput = screen.getByPlaceholderText("e.g. node-sgp-01");
    fireEvent.change(nameInput, { target: { value: "node-secured" } });
    fireEvent.click(screen.getByRole("button", { name: /Generate Enrollment Token/i }));

    // Should prompt for sudo password
    await waitFor(() => {
      expect(screen.getByText(/Administrative sudo verification required/i)).toBeInTheDocument();
    });

    const pwdInput = screen.getByPlaceholderText("••••••••••••");
    fireEvent.change(pwdInput, { target: { value: "secret123" } });
    fireEvent.click(screen.getByRole("button", { name: /Verify Password/i }));

    // Upon sudo success, retry succeeds and shows token alone (enroll_command is null)
    await waitFor(() => {
      expect(screen.getByText("dhenroll_after_sudo")).toBeInTheDocument();
      expect(screen.queryByText(/Run this command on your server/i)).not.toBeInTheDocument();
    });
  });

  it("opens node specifications dialog", async () => {
    render(
      <ToastProvider>
        <NodesPage />
      </ToastProvider>
    );

    await waitFor(() => {
      expect(screen.getByText("node-master-01")).toBeInTheDocument();
    });

    const viewSpecsBtns = screen.getAllByRole("button", { name: /View Specs/i });
    fireEvent.click(viewSpecsBtns[0]);

    await waitFor(() => {
      // What the agent reports: Docker and the kernel (a CPU model and a fingerprint are not part of it).
      expect(screen.getByText("27.3.1")).toBeInTheDocument();
      expect(screen.getByText("Linux 6.8.0-45-generic")).toBeInTheDocument();
    });
  });

  it("opens revoke confirmation dialog and requires typed confirmation", async () => {
    render(
      <ToastProvider>
        <NodesPage />
      </ToastProvider>
    );

    await waitFor(() => {
      expect(screen.getByText("node-master-01")).toBeInTheDocument();
    });

    const revokeBtns = screen.getAllByRole("button", { name: /Revoke/i });
    fireEvent.click(revokeBtns[0]);

    await waitFor(() => {
      expect(screen.getByText("Revoke Node Certificate")).toBeInTheDocument();
    });

    // The confirm button is initially disabled
    const confirmBtn = screen.getByRole("button", { name: /^Confirm$/i });
    expect(confirmBtn).toBeDisabled();

    // Type the node name into the confirmation input
    const input = screen.getByPlaceholderText("node-master-01");
    fireEvent.change(input, { target: { value: "node-master-01" } });

    expect(confirmBtn).toBeEnabled();
    fireEvent.click(confirmBtn);

    await waitFor(() => {
      expect(screen.queryByText("Revoke Node Certificate")).not.toBeInTheDocument();
    });
  });

  it("renders permission denied when user lacks admin.nodes.read", () => {
    mockHasPermission = false;
    render(
      <ToastProvider>
        <NodesPage />
      </ToastProvider>
    );
    expect(screen.getByText("Permission denied")).toBeInTheDocument();
  });

  it("rejects agent update when Ed25519 signature verification fails (DoD T16.08)", async () => {
    render(
      <ToastProvider>
        <NodesPage />
      </ToastProvider>
    );

    await waitFor(() => {
      expect(screen.getByTestId("btn-update-agent-node_01")).toBeInTheDocument();
    });

    // Open Agent Update Dialog
    fireEvent.click(screen.getByTestId("btn-update-agent-node_01"));

    await waitFor(() => {
      expect(screen.getByText("Signed Agent Self-Update")).toBeInTheDocument();
      expect(screen.getByTestId("toggle-corrupt-signature")).toBeInTheDocument();
    });

    // Toggle corrupt signature
    fireEvent.click(screen.getByTestId("toggle-corrupt-signature"));

    // Click Apply Signed Update
    fireEvent.click(screen.getByTestId("btn-apply-agent-update"));

    // Verify rejection alert (DoD T16.08)
    await waitFor(() => {
      expect(screen.getByTestId("agent-update-rejected-alert")).toBeInTheDocument();
      expect(screen.getByTestId("badge-signature-rejected")).toHaveTextContent("REJECTED");
      expect(screen.getByText(/ERR_SIGNATURE_MISMATCH/i)).toBeInTheDocument();
    });
  });

  it("applies agent update when cryptographic signature is valid (DoD T16.08)", async () => {
    render(
      <ToastProvider>
        <NodesPage />
      </ToastProvider>
    );

    await waitFor(() => {
      expect(screen.getByTestId("btn-update-agent-node_01")).toBeInTheDocument();
    });

    fireEvent.click(screen.getByTestId("btn-update-agent-node_01"));

    await waitFor(() => {
      expect(screen.getByTestId("btn-apply-agent-update")).toBeInTheDocument();
    });

    // Valid signature (untoggled corrupt signature)
    fireEvent.click(screen.getByTestId("btn-apply-agent-update"));

    // Verify success banner and verified status
    await waitFor(() => {
      expect(screen.getByTestId("agent-update-success-alert")).toBeInTheDocument();
      expect(screen.getByTestId("badge-update-verified")).toHaveTextContent("VERIFIED & UPDATED");
    });
  });

});