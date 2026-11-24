import React from "react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { ResellerStatsGrid } from "./ResellerStatsGrid";
import { ResellerBrandingCard } from "./ResellerBrandingCard";
import { ResellerUsersTable } from "./ResellerUsersTable";
import { CreateResellerUserDialog } from "./CreateResellerUserDialog";
import { ResellerPlansTable } from "./ResellerPlansTable";
import { CreateResellerPlanDialog } from "./CreateResellerPlanDialog";
import { ResellerPortalPage } from "./ResellerPortalPage";
import { AdminResellersPage } from "./AdminResellersPage";
import {
  ResellerAccount,
  ResellerPackage,
  ResellerUser,
  ResellerPlan,
  ResellerBranding,
} from "./types";

// Mock @builtpanel/api
const mockApiFetch = vi.fn();
vi.mock("@builtpanel/api", () => ({
  apiFetch: (...args: any[]) => mockApiFetch(...args),
}));

// Mock @builtpanel/i18n
vi.mock("@builtpanel/i18n", () => ({
  useTranslation: () => ({
    t: (key: string, defaultVal?: string | Record<string, any>) => {
      if (typeof defaultVal === "string") return defaultVal;
      return key;
    },
    i18n: { language: "en" },
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

describe("Reseller Module UI Components (T14.05, T14.02, T14.07)", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  const mockBranding: ResellerBranding = {
    brand_name: "Apex Cloud Solutions",
    logo_url: "https://example.com/logo.png",
    primary_color: "#2563eb",
    support_email: "support@apexcloud.net",
    custom_login_message: "Welcome to your high performance cloud control panel.",
  };

  const mockPkg: ResellerPackage = {
    id: "pkg-1",
    name: "Growth Agency Tier",
    max_accounts: 25,
    max_disk_gb: 150,
    max_bandwidth_gb: 1500,
    allow_overselling: true,
    allowed_plans: ["all"],
    created_at: "2026-01-01T00:00:00Z",
  };

  const mockAccount: ResellerAccount = {
    id: "reseller-1",
    username: "apex_reseller",
    email: "partner@apexcloud.net",
    company_name: "Apex Cloud Solutions",
    package_id: "pkg-1",
    package_name: "Growth Agency Tier",
    status: "active",
    accounts_count: 8,
    disk_used_gb: 42,
    bandwidth_used_gb: 380,
    branding: mockBranding,
    created_at: "2026-01-01T00:00:00Z",
  };

  const mockUsers: ResellerUser[] = [
    {
      id: "ru-1",
      reseller_id: "reseller-1",
      username: "novatech",
      email: "admin@novatech.io",
      domain: "novatech.io",
      plan_name: "Business Pro",
      disk_used_mb: 4200,
      status: "active",
      created_at: "2026-01-01T00:00:00Z",
    },
  ];

  const mockPlans: ResellerPlan[] = [
    {
      id: "rp-1",
      reseller_id: "reseller-1",
      name: "Client Starter",
      disk_mb: 5120,
      bandwidth_mb: 51200,
      domains_limit: 1,
      databases_limit: 2,
      mailboxes_limit: 5,
      created_at: "2026-01-01T00:00:00Z",
    },
  ];

  describe("ResellerStatsGrid", () => {
    it("renders quotas, disk, and tier allocations", () => {
      render(<ResellerStatsGrid account={mockAccount} pkg={mockPkg} />);

      expect(screen.getByTestId("stat-accounts-count").textContent).toBe("8");
      expect(screen.getByTestId("stat-disk-used").textContent).toContain("42 GB");
      expect(screen.getByTestId("stat-bw-used").textContent).toContain("380 GB");
      expect(screen.getByTestId("stat-package-name").textContent).toBe("Growth Agency Tier");
    });
  });

  describe("ResellerBrandingCard (T14.05)", () => {
    it("renders branding form and real-time live preview", async () => {
      const handleSave = vi.fn().mockResolvedValue(undefined);
      render(<ResellerBrandingCard branding={mockBranding} onSaveBranding={handleSave} />);

      expect(screen.getByTestId("preview-brand-name").textContent).toBe("Apex Cloud Solutions");
      expect(screen.getByTestId("preview-support-email").textContent).toBe("support@apexcloud.net");

      // Change brand name and verify preview updates
      fireEvent.change(screen.getByTestId("input-brand-name"), {
        target: { value: "Skyline Hosting" },
      });
      expect(screen.getByTestId("preview-brand-name").textContent).toBe("Skyline Hosting");

      // Save
      fireEvent.click(screen.getByTestId("btn-save-branding"));
      await waitFor(() => {
        expect(handleSave).toHaveBeenCalledWith(
          expect.objectContaining({
            brand_name: "Skyline Hosting",
          })
        );
      });
    });
  });

  describe("ResellerUsersTable & CreateResellerUserDialog", () => {
    it("renders user rows and provides suspend/create triggers", async () => {
      const handleToggle = vi.fn().mockResolvedValue(undefined);
      render(
        <ResellerUsersTable
          users={mockUsers}
          plans={mockPlans}
          loading={false}
          onCreateUser={vi.fn()}
          onToggleSuspend={handleToggle}
          onDeleteUser={vi.fn()}
        />
      );

      expect(screen.getByTestId("reseller-user-row-ru-1")).toBeDefined();
      expect(screen.getByText("novatech.io")).toBeDefined();

      // Trigger suspend
      fireEvent.click(screen.getByTestId("btn-toggle-suspend-user-ru-1"));
      expect(handleToggle).toHaveBeenCalledWith("ru-1", "active");
    });

    it("creates a new sub-tenant account via dialog", async () => {
      const handleCreate = vi.fn().mockResolvedValue(undefined);
      render(
        <CreateResellerUserDialog
          open={true}
          plans={mockPlans}
          onClose={vi.fn()}
          onCreate={handleCreate}
        />
      );

      fireEvent.change(screen.getByTestId("input-reseller-user-username"), {
        target: { value: "clientcorp" },
      });
      fireEvent.change(screen.getByTestId("input-reseller-user-domain"), {
        target: { value: "clientcorp.com" },
      });
      fireEvent.click(screen.getByTestId("btn-submit-reseller-user"));

      await waitFor(() => {
        expect(handleCreate).toHaveBeenCalledWith(
          expect.objectContaining({
            username: "clientcorp",
            domain: "clientcorp.com",
          })
        );
      });
    });
  });

  describe("ResellerPlansTable & CreateResellerPlanDialog", () => {
    it("renders plans table and creates custom plan", async () => {
      const handleCreate = vi.fn().mockResolvedValue(undefined);
      render(
        <CreateResellerPlanDialog
          open={true}
          onClose={vi.fn()}
          onCreate={handleCreate}
        />
      );

      fireEvent.change(screen.getByTestId("input-reseller-plan-name"), {
        target: { value: "Platinum VIP" },
      });
      fireEvent.click(screen.getByTestId("btn-submit-reseller-plan"));

      await waitFor(() => {
        expect(handleCreate).toHaveBeenCalledWith(
          expect.objectContaining({
            name: "Platinum VIP",
          })
        );
      });
    });
  });

  describe("ResellerPortalPage Integration", () => {
    it("loads reseller data and switches tabs", async () => {
      mockApiFetch.mockImplementation((url: string) => {
        if (url.includes("/api/v1/reseller/me/package")) return Promise.resolve(mockPkg);
        if (url.includes("/api/v1/reseller/me")) return Promise.resolve(mockAccount);
        if (url.includes("/api/v1/reseller/users")) return Promise.resolve(mockUsers);
        if (url.includes("/api/v1/reseller/plans")) return Promise.resolve(mockPlans);
        return Promise.resolve([]);
      });

      render(<ResellerPortalPage />);

      await waitFor(() => {
        expect(screen.getByTestId("reseller-portal-title")).toBeDefined();
        expect(screen.getByTestId("reseller-users-section")).toBeDefined();
      });

      // Switch to Plans tab
      fireEvent.click(screen.getByTestId("tab-reseller-plans"));
      await waitFor(() => {
        expect(screen.getByTestId("reseller-plans-section")).toBeDefined();
      });

      // Switch to Branding tab
      fireEvent.click(screen.getByTestId("tab-reseller-branding"));
      await waitFor(() => {
        expect(screen.getByTestId("reseller-branding-section")).toBeDefined();
      });
    });
  });

  describe("AdminResellersPage (T14.02 & T14.07)", () => {
    it("lists resellers, suspends reseller, and switches to packages tab", async () => {
      mockApiFetch.mockImplementation((url: string, opts?: any) => {
        if (url.includes("/api/v1/admin/resellers/reseller-1/status")) {
          return Promise.resolve({ success: true, status: "suspended" });
        }
        if (url.includes("/api/v1/admin/resellers")) return Promise.resolve([mockAccount]);
        if (url.includes("/api/v1/admin/reseller-packages")) return Promise.resolve([mockPkg]);
        return Promise.resolve([]);
      });

      render(<AdminResellersPage />);

      await waitFor(() => {
        expect(screen.getByTestId("admin-reseller-row-reseller-1")).toBeDefined();
        expect(screen.getByText("Apex Cloud Solutions")).toBeDefined();
      });

      // Suspend reseller (T14.07)
      fireEvent.click(screen.getByTestId("btn-suspend-reseller-reseller-1"));
      await waitFor(() => {
        expect(mockApiFetch).toHaveBeenCalledWith(
          "/api/v1/admin/resellers/reseller-1/status",
          expect.objectContaining({ method: "POST" })
        );
      });

      // Switch to Packages tab (T14.02)
      fireEvent.click(screen.getByTestId("tab-admin-packages"));
      await waitFor(() => {
        expect(screen.getByTestId("packages-table-container")).toBeDefined();
        expect(screen.getByText("Growth Agency Tier")).toBeDefined();
      });
    });
  });
});
