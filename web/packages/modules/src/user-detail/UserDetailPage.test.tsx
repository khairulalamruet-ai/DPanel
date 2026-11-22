import { describe, it, expect, vi, beforeEach } from "vitest";
import React from "react";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { ToastProvider } from "@builtpanel/ui";
import { UserDetailPage } from "./UserDetailPage";

function renderWithToast(ui: React.ReactElement) {
  return render(<ToastProvider>{ui}</ToastProvider>);
}

// Mock @tanstack/react-router
vi.mock("@tanstack/react-router", () => ({
  useParams: () => ({ id: "usr_0001" }),
  Link: ({ children, to }: any) => <a href={to}>{children}</a>,
}));

// Mock @builtpanel/i18n
vi.mock("@builtpanel/i18n", () => ({
  useTranslation: () => ({
    t: (key: string, opts?: any) => {
      const dict: Record<string, string> = {
        "common.error": "Error",
        "common.loading": "Loading...",
        "common.appName": "Built Panel",
        "userDetail.title": "Tenant Details",
        "userDetail.backToUsers": "Back to Users",
        "userDetail.status.active": "Active",
        "userDetail.status.suspended": "Suspended",
        "userDetail.status.pending": "Pending",
        "userDetail.actions.impersonate": "Login as User",
        "userDetail.actions.suspend": "Suspend Account",
        "userDetail.actions.unsuspend": "Unsuspend Account",
        "userDetail.actions.resetPassword": "Reset Password",
        "userDetail.actions.delete": "Delete Tenant",
        "userDetail.tabs.overview": "Overview",
        "userDetail.tabs.resources": "Resources & Quota",
        "userDetail.tabs.domains": "Domains",
        "userDetail.tabs.databases": "Databases",
        "userDetail.tabs.logs": "Container Logs",
        "userDetail.tabs.activity": "Activity Ledger",
        "userDetail.overview.accountDetails": "Account Identity & Root",
        "userDetail.overview.containerStack": "Container Runtime Stack",
        "userDetail.resources.title": "Hardware Resource Allocations",
        "userDetail.resources.desc": "Real-time resource utilization against enforced cgroup and filesystem limits",
        "userDetail.domains.title": "Configured Web Domains",
        "userDetail.domains.desc": "Vhosts and routing associated with this tenant's web server container",
        "userDetail.databases.title": "Database Allocations",
        "userDetail.databases.desc": "Managed database instances provisioned for this tenant container (Phase 07 Preview)",
        "userDetail.logs.title": "Container Log Streams",
        "userDetail.logs.desc": "Standard output and error streams from tenant web and runtime containers",
        "userDetail.activity.title": "Tenant Activity & Audit Ledger",
        "userDetail.activity.desc": "Chronological history of administrative actions, logins, and container operations",
        "userDetail.dialog.resetPasswordTitle": "Reset Password: " + (opts?.username || ""),
        "userDetail.dialog.resetPasswordDesc": "Generate or assign a new secure password",
        "userDetail.dialog.newPasswordLabel": "New Password",
        "userDetail.dialog.generateNew": "Generate",
        "userDetail.dialog.updatePassword": "Save New Password",
        "userDetail.dialog.confirmDelete": "Are you sure you want to delete?",
        "userDetail.toast.impersonateSuccess": "Impersonation session created",
        "userDetail.toast.impersonateDesc": "Redirecting...",
        "userDetail.toast.suspendSuccess": "Suspended",
        "userDetail.toast.unsuspendSuccess": "Unsuspended",
        "userDetail.toast.passwordResetSuccess": "Password updated",
      };
      return dict[key] || key;
    },
  }),
}));

const mockUser = {
  id: "usr_0001",
  username: "alpha",
  email: "alpha@example.com",
  status: "active",
  uid: 1001,
  root_path: "/srv/tenants/alpha",
  plan_id: "plan_standard",
  plan_name: "Standard Business",
  node_id: "node-sgp-01",
  node_name: "node-sgp-01 (Singapore)",
  primary_domain: "alpha.com",
  created_at: "2026-10-01T00:00:00Z",
  suspended_at: null,
  suspended_reason: null,
  resources: {
    cpu_cores: 2,
    cpu_used_pct: 28,
    memory_limit_mb: 2048,
    memory_used_mb: 850,
    disk_limit_mb: 25600,
    disk_used_mb: 1240,
    inodes_limit: 250000,
    inodes_used: 12450,
    bandwidth_limit_mb: -1,
    bandwidth_used_mb: 4800,
    pids_limit: 256,
    pids_used: 42,
  },
  container: {
    id: "cnt_alpha_01",
    name: "bp-tenant-alpha",
    status: "running",
    image: "builtpanel/stack-php83-nginx:latest",
    ip_address: "172.28.0.5",
    started_at: "2026-10-01T00:00:00Z",
    uptime: "Up 14 days",
  },
  domains: [
    {
      id: "dom_01",
      domain: "alpha.com",
      is_primary: true,
      docroot: "/srv/tenants/alpha/domains/alpha.com/public_html",
      ssl_status: "active",
      php_version: "PHP 8.3",
    },
  ],
  databases: [
    {
      id: "db_01",
      name: "alpha_wp",
      type: "mysql",
      size_mb: 48,
      charset: "utf8mb4",
    },
  ],
  logs: [
    {
      id: "log_01",
      timestamp: "2026-11-12T00:00:00Z",
      level: "info",
      source: "nginx",
      message: "GET / HTTP/2.0 200",
    },
  ],
  activities: [
    {
      id: "act_01",
      action: "user.login",
      actor: "alpha",
      timestamp: "2026-11-12T00:00:00Z",
      status: "success",
      details: "Client portal session initiated",
    },
  ],
};

const mockApiFetch = vi.fn();
vi.mock("@builtpanel/api", () => ({
  apiFetch: (...args: any[]) => mockApiFetch(...args),
}));
// The page is made of several calls (see ./api); here it is given what they add up to.
vi.mock("./api", () => ({
  loadUserDetail: async () => mockUser,
}));

describe("UserDetailPage", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockApiFetch.mockImplementation(async (url: string, _options?: any) => {
      if (url.includes("/suspend")) {
        return { success: true, status: "suspended" };
      }
      if (url.includes("/impersonate")) {
        return { token: "tok_123", redirect_url: "/client?user=usr_0001" };
      }
      if (url.includes("/password")) {
        return { success: true };
      }
      return mockUser;
    });
  });

  it("renders user header and overview tab by default", async () => {
    renderWithToast(<UserDetailPage />);

    await waitFor(() => {
      expect(screen.getByRole("heading", { level: 1, name: "alpha" })).toBeInTheDocument();
    });

    expect(screen.getByText("Active")).toBeInTheDocument();
    expect(screen.getByText("Standard Business")).toBeInTheDocument();
    expect(screen.getByText(/alpha@example\.com/)).toBeInTheDocument();
    expect(screen.getByText("running")).toBeInTheDocument();
    expect(screen.getByText("Account Identity & Root")).toBeInTheDocument();
    expect(screen.getByText("/srv/tenants/alpha")).toBeInTheDocument();
  });

  it("switches to resources tab and displays allocations", async () => {
    renderWithToast(<UserDetailPage />);

    await waitFor(() => {
      expect(screen.getByRole("heading", { level: 1, name: "alpha" })).toBeInTheDocument();
    });

    const resourcesTab = screen.getByRole("button", { name: /Resources & Quota/i });
    fireEvent.click(resourcesTab);

    expect(screen.getByText("Hardware Resource Allocations")).toBeInTheDocument();
    expect(screen.getByText(/CPU Cores & Frequency/i)).toBeInTheDocument();
    expect(screen.getByText(/Memory \(RAM\)/i)).toBeInTheDocument();
    expect(screen.getByText(/Filesystem Storage/i)).toBeInTheDocument();
  });

  it("switches to domains tab and displays configured vhosts", async () => {
    renderWithToast(<UserDetailPage />);

    await waitFor(() => {
      expect(screen.getByRole("heading", { level: 1, name: "alpha" })).toBeInTheDocument();
    });

    const domainsTab = screen.getByRole("button", { name: /Domains/i });
    fireEvent.click(domainsTab);

    expect(screen.getByText("Configured Web Domains")).toBeInTheDocument();
    expect(screen.getAllByText("alpha.com").length).toBeGreaterThanOrEqual(1);
    expect(screen.getByText("PHP 8.3")).toBeInTheDocument();
  });

  it("switches to databases tab and displays database allocations", async () => {
    renderWithToast(<UserDetailPage />);

    await waitFor(() => {
      expect(screen.getByRole("heading", { level: 1, name: "alpha" })).toBeInTheDocument();
    });

    const dbTab = screen.getByRole("button", { name: /Databases/i });
    fireEvent.click(dbTab);

    expect(screen.getByText("Database Allocations")).toBeInTheDocument();
    expect(screen.getByText("alpha_wp")).toBeInTheDocument();
    expect(screen.getByText("mysql")).toBeInTheDocument();
  });

  it("switches to logs tab and displays container log streams", async () => {
    renderWithToast(<UserDetailPage />);

    await waitFor(() => {
      expect(screen.getByRole("heading", { level: 1, name: "alpha" })).toBeInTheDocument();
    });

    const logsTab = screen.getByRole("button", { name: /Container Logs/i });
    fireEvent.click(logsTab);

    expect(screen.getByText("Container Log Streams")).toBeInTheDocument();
    expect(screen.getByText("GET / HTTP/2.0 200")).toBeInTheDocument();
  });

  it("switches to activity tab and displays audit ledger", async () => {
    renderWithToast(<UserDetailPage />);

    await waitFor(() => {
      expect(screen.getByRole("heading", { level: 1, name: "alpha" })).toBeInTheDocument();
    });

    const actTab = screen.getByRole("button", { name: /Activity Ledger/i });
    fireEvent.click(actTab);

    expect(screen.getByText("Tenant Activity & Audit Ledger")).toBeInTheDocument();
    expect(screen.getByText("Client portal session initiated")).toBeInTheDocument();
  });

  it("handles impersonation action", async () => {
    const originalOpen = window.open;
    window.open = vi.fn();

    renderWithToast(<UserDetailPage />);

    await waitFor(() => {
      expect(screen.getByRole("heading", { level: 1, name: "alpha" })).toBeInTheDocument();
    });

    const impBtn = screen.getByRole("button", { name: /Login as User/i });
    fireEvent.click(impBtn);

    await waitFor(() => {
      expect(mockApiFetch).toHaveBeenCalledWith(
        "/api/v1/admin/users/usr_0001/impersonate",
        expect.objectContaining({ method: "POST" })
      );
    });

    window.open = originalOpen;
  });

  it("handles suspend action", async () => {
    renderWithToast(<UserDetailPage />);

    await waitFor(() => {
      expect(screen.getByRole("heading", { level: 1, name: "alpha" })).toBeInTheDocument();
    });

    const suspendBtn = screen.getByRole("button", { name: /Suspend Account/i });
    fireEvent.click(suspendBtn);

    await waitFor(() => {
      expect(mockApiFetch).toHaveBeenCalledWith(
        "/api/v1/admin/users/usr_0001/suspend",
        expect.objectContaining({ method: "POST" })
      );
    });
  });

  it("opens reset password dialog and submits new password", async () => {
    renderWithToast(<UserDetailPage />);

    await waitFor(() => {
      expect(screen.getByRole("heading", { level: 1, name: "alpha" })).toBeInTheDocument();
    });

    const resetBtn = screen.getByRole("button", { name: /Reset Password/i });
    fireEvent.click(resetBtn);

    expect(screen.getByText("Reset Password: alpha")).toBeInTheDocument();

    const input = screen.getByPlaceholderText(/Enter new password/i);
    fireEvent.change(input, { target: { value: "NewSecurePass!234" } });

    const saveBtn = screen.getByRole("button", { name: /Save New Password/i });
    fireEvent.click(saveBtn);

    await waitFor(() => {
      expect(mockApiFetch).toHaveBeenCalledWith(
        "/api/v1/admin/users/usr_0001/password",
        expect.objectContaining({
          method: "POST",
          body: JSON.stringify({ password: "NewSecurePass!234" }),
        })
      );
    });
  });
});
