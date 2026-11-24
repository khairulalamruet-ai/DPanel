import React from "react";
import { describe, it, expect, beforeEach } from "vitest";
import { render, screen, waitFor, fireEvent, within } from "@testing-library/react";
import { ToastProvider } from "@builtpanel/ui";
import { resetMockAdminDomains } from "@builtpanel/api";
import { AdminDomainsPage } from "./AdminDomainsPage";

function renderWithProviders(ui: React.ReactElement) {
  return render(<ToastProvider>{ui}</ToastProvider>);
}

describe("AdminDomainsPage (T05.06)", () => {
  beforeEach(() => {
    resetMockAdminDomains();
  });

  it("renders page title, metrics cards, and initial seeded cluster domains", async () => {
    renderWithProviders(<AdminDomainsPage />);

    expect(screen.getByRole("heading", { level: 1 })).toBeInTheDocument();

    await waitFor(() => {
      expect(screen.getByText("alpha-hosting.com")).toBeInTheDocument();
    });

    expect(screen.getByText("beta-hosting.com")).toBeInTheDocument();
    expect(screen.getByText("gamma-hosting.com")).toBeInTheDocument();
    expect(screen.getByText("delta-hosting.com")).toBeInTheDocument();

    // Verify tenant usernames appear in table
    expect(screen.getAllByText("alpha").length).toBeGreaterThan(0);
    expect(screen.getAllByText("beta").length).toBeGreaterThan(0);
  });

  it("filters domains table by search keyword", async () => {
    renderWithProviders(<AdminDomainsPage />);

    await waitFor(() => {
      expect(screen.getByText("alpha-hosting.com")).toBeInTheDocument();
    });

    const searchInput = screen.getByTestId("input-search-admin-domains");
    fireEvent.change(searchInput, { target: { value: "beta" } });

    await waitFor(() => {
      expect(screen.getByText("beta-hosting.com")).toBeInTheDocument();
      expect(screen.queryByText("alpha-hosting.com")).not.toBeInTheDocument();
    });
  });

  it("toggles domain suspend and unsuspend status", async () => {
    renderWithProviders(<AdminDomainsPage />);

    await waitFor(() => {
      expect(screen.getByText("alpha-hosting.com")).toBeInTheDocument();
    });

    // Find suspend button for adm_dom_1
    const suspendBtn = screen.getByTestId("btn-toggle-suspend-adm_dom_1");
    fireEvent.click(suspendBtn);

    // Alpha domain status changes to Suspended
    await waitFor(() => {
      const badge = screen.getByTestId("domain-status-badge-adm_dom_1");
      expect(badge).toHaveTextContent(/Suspended|স্থগিত/i);
    });

    // Click Unsuspend
    const unsuspendBtn = screen.getByTestId("btn-toggle-suspend-adm_dom_1");
    fireEvent.click(unsuspendBtn);

    // Alpha domain status reverts to Active
    await waitFor(() => {
      const badge = screen.getByTestId("domain-status-badge-adm_dom_1");
      expect(badge).toHaveTextContent(/Active|সক্রিয়/i);
    });
  });

  it("DoD: Bulk adds 100 domains through the operation queue dialog", async () => {
    renderWithProviders(<AdminDomainsPage />);

    await waitFor(() => {
      expect(screen.getByText("alpha-hosting.com")).toBeInTheDocument();
    });

    // Click "Bulk Add (Queue)"
    const bulkBtn = screen.getByTestId("btn-bulk-add-domains");
    fireEvent.click(bulkBtn);

    // Dialog appears
    await waitFor(() => {
      expect(screen.getByTestId("btn-fill-100-samples")).toBeInTheDocument();
    });

    // Click "Sample 100 domains" quick fill
    const sampleBtn = screen.getByTestId("btn-fill-100-samples");
    fireEvent.click(sampleBtn);

    // Verify textarea populated with 100 lines
    const textarea = screen.getByTestId("textarea-bulk-domains") as HTMLTextAreaElement;
    expect(textarea.value).toContain("bulk-site-001.example.com");
    expect(textarea.value).toContain("bulk-site-100.example.com");

    // Counter shows 100
    expect(screen.getByText("100")).toBeInTheDocument();

    // Submit bulk operation
    const submitBtn = screen.getByTestId("btn-submit-bulk-domains");
    fireEvent.click(submitBtn);

    // Operation view appears with operation ID and succeeded badge
    await waitFor(() => {
      expect(screen.getByTestId("bulk-op-succeeded-badge")).toBeInTheDocument();
    });

    // Click Done & Close
    const doneBtn = screen.getByTestId("btn-done-bulk-domains");
    fireEvent.click(doneBtn);

    // Dialog closes and new domains appear in the admin domains list
    await waitFor(() => {
      expect(screen.getByText("bulk-site-001.example.com")).toBeInTheDocument();
    });
  });

  it("deletes a domain from the cluster", async () => {
    renderWithProviders(<AdminDomainsPage />);

    await waitFor(() => {
      expect(screen.getByText("landing.gamma-hosting.com")).toBeInTheDocument();
    });

    // Click delete on adm_dom_07
    const deleteBtn = screen.getByTestId("btn-delete-domain-adm_dom_07");
    fireEvent.click(deleteBtn);

    // Confirm dialog appears
    await waitFor(() => {
      expect(screen.getByTestId("btn-confirm-delete-domain")).toBeInTheDocument();
    });

    fireEvent.click(screen.getByTestId("btn-confirm-delete-domain"));

    // Domain removed
    await waitFor(() => {
      expect(screen.queryByText("landing.gamma-hosting.com")).not.toBeInTheDocument();
    });
  });
});
