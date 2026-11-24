import { describe, it, expect, vi, beforeEach } from "vitest";
import React from "react";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { LaAdminPage } from "./LaAdminPage";
import { ToastProvider } from "@builtpanel/ui";
import * as builtpanelApi from "@builtpanel/api";
import type { LaLicenseItem, LaOverviewMetrics } from "./types";

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
        "la.title": "License Authority (LA) Console",
        "la.desc": "Manage cryptographically issued licenses",
        "la.issueBtn": "Issue License",
        "la.revokeBtn": "Revoke",
        "la.empty": "No licenses found.",
        "la.metrics.totalLicenses": "Total Licenses",
        "la.metrics.activeLicenses": "Active Licenses",
        "la.metrics.revokedLicenses": "Revoked Licenses",
        "la.metrics.activeActivations": "Active Activations",
        "la.tabs.licenses": "Issued Licenses",
        "la.tabs.activations": "Node Activations",
        "la.tabs.customers": "Customer Directory",
        "la.table.licenseKey": "License Key",
        "la.table.customer": "Customer",
        "la.table.tier": "Tier",
        "la.table.limits": "Limits",
        "la.table.status": "Status",
        "la.table.activations": "Activations",
        "la.table.actions": "Actions",
        "la.status.active": "Active",
        "la.status.revoked": "Revoked",
        "la.status.suspended": "Suspended",
        "la.status.expired": "Expired",
        "la.issue.title": "Issue New Customer License",
        "la.issue.desc": "Generate cryptographically signed license keys",
        "la.issue.nameLabel": "Customer / Organization Name",
        "la.issue.emailLabel": "Billing Email Address",
        "la.issue.companyLabel": "Company (Optional)",
        "la.issue.planTierLabel": "Product Tier",
        "la.issue.validityLabel": "License Validity Period",
        "la.issue.maxNodesLabel": "Max Nodes",
        "la.issue.maxTenantsLabel": "Max Tenants",
        "la.issue.maxActivationsLabel": "Max Activations",
        "la.issue.submitBtn": "Issue License Key",
        "la.issue.successToast": "License key issued successfully.",
        "la.revoke.title": "Revoke Customer License",
        "la.revoke.desc": "Revoking this license immediately halts new provisioning",
        "la.revoke.reasonLabel": "Revocation Justification",
        "la.revoke.submitBtn": "Confirm Revocation",
        "la.revoke.successToast": "License revoked successfully.",
        "la.plan.starter": "Starter Cloud",
        "la.plan.business": "Business Pro",
        "la.plan.enterprise": "Enterprise Cluster",
        "la.plan.unlimited": "Unlimited Partner",
        "la.validity.30days": "30 Days (Evaluation)",
        "la.validity.1year": "1 Year (Commercial)",
        "la.validity.lifetime": "Lifetime Perpetual",
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

const mockLicensesInitial: LaLicenseItem[] = [
  {
    id: "lic_01",
    license_key: "DHP-ENT-2026-A1B2-C3D4",
    customer_id: "cust_01",
    customer_name: "Acme Global Hosting",
    customer_email: "billing@acme.com",
    plan_tier: "enterprise",
    status: "active",
    max_nodes: 10,
    max_tenants: 500,
    activations_count: 2,
    max_activations: 5,
    created_at: "2026-10-15T00:00:00Z",
    expires_at: "2027-10-15T00:00:00Z",
    revoked_at: null,
    revoke_reason: null,
  },
];

describe("LaAdminPage (T17.06)", () => {
  let mockLicenses: LaLicenseItem[] = [];

  beforeEach(() => {
    vi.restoreAllMocks();
    mockHasPermission = true;
    mockLicenses = [...mockLicensesInitial];

    vi.spyOn(builtpanelApi, "apiFetch").mockImplementation(async (url: string, opts?: any) => {
      if (url === "/api/v1/la/overview") {
        return {
          total_licenses: mockLicenses.length,
          active_licenses: mockLicenses.filter((l) => l.status === "active").length,
          revoked_licenses: mockLicenses.filter((l) => l.status === "revoked").length,
          total_customers: 1,
          active_activations: 2,
        } as LaOverviewMetrics;
      }
      if (url === "/api/v1/la/licenses" && (!opts || opts.method === "GET")) {
        return { items: mockLicenses };
      }
      if (url === "/api/v1/la/licenses" && opts?.method === "POST") {
        const body = JSON.parse(opts.body || "{}");
        const newLic: LaLicenseItem = {
          id: `lic_${Date.now()}`,
          license_key: "DHP-ENT-2026-NEW1-KEY2",
          customer_id: "cust_new",
          customer_name: body.customer_name,
          customer_email: body.customer_email,
          plan_tier: body.plan_tier || "enterprise",
          status: "active",
          max_nodes: body.max_nodes || 5,
          max_tenants: body.max_tenants || 100,
          activations_count: 0,
          max_activations: body.max_activations || 1,
          created_at: new Date().toISOString(),
          expires_at: new Date(Date.now() + 365 * 86400 * 1000).toISOString(),
          revoked_at: null,
          revoke_reason: null,
        };
        mockLicenses.unshift(newLic);
        return { success: true, license: newLic };
      }
      if (url.includes("/revoke") && opts?.method === "POST") {
        const body = JSON.parse(opts.body || "{}");
        const id = url.split("/")[5];
        const target = mockLicenses.find((l) => l.id === id);
        if (target) {
          target.status = "revoked";
          target.revoked_at = new Date().toISOString();
          target.revoke_reason = body.reason;
        }
        return { success: true, license: target };
      }
      if (url === "/api/v1/la/activations") {
        return { items: [] };
      }
      if (url === "/api/v1/la/customers") {
        return { items: [] };
      }
      return {};
    });
  });

  it("renders overview metrics and license list", async () => {
    render(
      <ToastProvider>
        <LaAdminPage />
      </ToastProvider>
    );

    expect(screen.getByText("License Authority (LA) Console")).toBeInTheDocument();

    await waitFor(() => {
      expect(screen.getByTestId("metric-total-licenses")).toHaveTextContent("1");
      expect(screen.getByTestId("metric-active-licenses")).toHaveTextContent("1");
      expect(screen.getByText("DHP-ENT-2026-A1B2-C3D4")).toBeInTheDocument();
      expect(screen.getByText("Acme Global Hosting")).toBeInTheDocument();
    });
  });

  it("issues a new license via dialog (DoD T17.06: license issue)", async () => {
    render(
      <ToastProvider>
        <LaAdminPage />
      </ToastProvider>
    );

    await waitFor(() => {
      expect(screen.getByTestId("btn-open-issue-license")).toBeInTheDocument();
    });

    // Open issue license modal
    fireEvent.click(screen.getByTestId("btn-open-issue-license"));

    await waitFor(() => {
      expect(screen.getByText("Issue New Customer License")).toBeInTheDocument();
    });

    // Fill customer form
    fireEvent.change(screen.getByTestId("input-customer-name"), {
      target: { value: "Omega Cloud Corp" },
    });
    fireEvent.change(screen.getByTestId("input-customer-email"), {
      target: { value: "license@omegacloud.com" },
    });

    // Submit
    fireEvent.click(screen.getByTestId("btn-confirm-issue-license"));

    // Verify license appears in the table (DoD T17.06)
    await waitFor(() => {
      expect(screen.getByText("DHP-ENT-2026-NEW1-KEY2")).toBeInTheDocument();
      expect(screen.getByText("Omega Cloud Corp")).toBeInTheDocument();
      expect(screen.queryByText("Issue New Customer License")).not.toBeInTheDocument();
    });
  });

  it("revokes an active license with reason (DoD T17.06: license revoke)", async () => {
    render(
      <ToastProvider>
        <LaAdminPage />
      </ToastProvider>
    );

    await waitFor(() => {
      expect(screen.getByTestId("btn-revoke-license-lic_01")).toBeInTheDocument();
    });

    // Click Revoke button on active license
    fireEvent.click(screen.getByTestId("btn-revoke-license-lic_01"));

    await waitFor(() => {
      expect(screen.getByText("Revoke Customer License")).toBeInTheDocument();
      expect(screen.getByTestId("btn-confirm-revoke-license")).toBeInTheDocument();
    });

    // Confirm revocation
    fireEvent.click(screen.getByTestId("btn-confirm-revoke-license"));

    // Verify license is marked revoked (DoD T17.06)
    await waitFor(() => {
      expect(screen.getByTestId("license-revoked-badge")).toHaveTextContent("Revoked");
      expect(screen.queryByText("Revoke Customer License")).not.toBeInTheDocument();
    });
  });
});
