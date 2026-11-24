import React from "react";
import { describe, it, expect, beforeEach } from "vitest";
import { render, screen, waitFor, fireEvent, within } from "@testing-library/react";
import { ToastProvider } from "@builtpanel/ui";
import { resetMockModules } from "@builtpanel/api";
import { ModuleManagerPage } from "./ModuleManagerPage";

function renderWithProviders(ui: React.ReactElement) {
  return render(<ToastProvider>{ui}</ToastProvider>);
}

describe("ModuleManagerPage (T03.12)", () => {
  beforeEach(() => {
    resetMockModules();
  });

  it("renders page header and loading state initially", async () => {
    renderWithProviders(<ModuleManagerPage />);

    expect(screen.getByText(/Module Manager|মডিউল ম্যানেজার/i)).toBeInTheDocument();

    await waitFor(() => {
      expect(screen.queryByText(/Loading|লোড হচ্ছে/i)).not.toBeInTheDocument();
    });
  });

  it("renders modules list with version, category, and dependencies", async () => {
    renderWithProviders(<ModuleManagerPage />);

    await waitFor(() => {
      expect(screen.getByText("(dns)")).toBeInTheDocument();
    });

    // Check version
    expect(screen.getAllByText("v1.0.0").length).toBeGreaterThan(0);

    // Check domains module and its dependencies
    expect(screen.getByText("(domains)")).toBeInTheDocument();
    expect(screen.getByText("webserver")).toBeInTheDocument();
    expect(screen.getByText("dns")).toBeInTheDocument();
  });

  it("disables toggle for core modules", async () => {
    renderWithProviders(<ModuleManagerPage />);

    await waitFor(() => {
      expect(screen.getByText("(auth)")).toBeInTheDocument();
    });

    const authRow = screen.getByText("(auth)").closest("tr");
    expect(authRow).not.toBeNull();

    const coreBtn = authRow!.querySelector("button")!;
    expect(coreBtn).toBeInTheDocument();
    expect(coreBtn).toBeDisabled();
  });

  it("filters modules by category", async () => {
    renderWithProviders(<ModuleManagerPage />);

    await waitFor(() => {
      expect(screen.getByText("(dns)")).toBeInTheDocument();
    });

    // Click Database category button
    const dbFilterBtn = screen.getByRole("button", { name: /Database|ডাটাবেস/i });
    fireEvent.click(dbFilterBtn);

    expect(screen.getByText("(db-mysql)")).toBeInTheDocument();
    expect(screen.queryByText("(dns)")).not.toBeInTheDocument();
  });

  it("filters modules by search query", async () => {
    renderWithProviders(<ModuleManagerPage />);

    await waitFor(() => {
      expect(screen.getByText("(dns)")).toBeInTheDocument();
    });

    const searchInput = screen.getByPlaceholderText(/Search modules|মডিউল খুঁজুন/i);
    fireEvent.change(searchInput, { target: { value: "dns" } });

    expect(screen.getByText("(dns)")).toBeInTheDocument();
    expect(screen.queryByText("(db-mysql)")).not.toBeInTheDocument();
  });

  it("shows dependency warning dialog when disabling dns (DoD)", async () => {
    renderWithProviders(<ModuleManagerPage />);

    await waitFor(() => {
      expect(screen.getByText("(dns)")).toBeInTheDocument();
    });

    // Find the Disable button for DNS
    const dnsRow = screen.getByText("(dns)").closest("tr");
    expect(dnsRow).not.toBeNull();

    const disableBtn = dnsRow!.querySelector("button")!;
    expect(disableBtn).toBeInTheDocument();
    fireEvent.click(disableBtn);

    // DoD check: Warning dialog appears mentioning domains
    await waitFor(() => {
      expect(
        screen.getByText(/Disable Module & Dependent Services\?|মডিউল ও নির্ভরশীল সার্ভিস বন্ধ করবেন\?/i)
      ).toBeInTheDocument();
    });

    const dialog = screen.getByRole("dialog");
    expect(dialog).toBeInTheDocument();

    // Check dialog contains the dependent module (domains)
    expect(within(dialog).getByText("(domains)")).toBeInTheDocument();

    // Confirm disabling in the modal
    const confirmBtn = within(dialog).getByRole("button", {
      name: /Confirm & Disable|নিশ্চিত করে বন্ধ করুন/i,
    });
    fireEvent.click(confirmBtn);

    // Dialog should close after confirming
    await waitFor(() => {
      expect(
        screen.queryByText(/Disable Module & Dependent Services\?|মডিউল ও নির্ভরশীল সার্ভিস বন্ধ করবেন\?/i)
      ).not.toBeInTheDocument();
    });
  });

  it("toggles module without dependents directly without warning modal", async () => {
    renderWithProviders(<ModuleManagerPage />);

    await waitFor(() => {
      expect(screen.getByText("(files)")).toBeInTheDocument();
    });

    // File Manager is currently disabled; click Enable
    const filesRow = screen.getByText("(files)").closest("tr");
    expect(filesRow).not.toBeNull();

    const enableBtn = filesRow!.querySelector("button")!;
    fireEvent.click(enableBtn);

    // Should NOT open warning modal
    expect(
      screen.queryByText(/Disable Module & Dependent Services\?|মডিউল ও নির্ভরশীল সার্ভিস বন্ধ করবেন\?/i)
    ).not.toBeInTheDocument();
  });
});
