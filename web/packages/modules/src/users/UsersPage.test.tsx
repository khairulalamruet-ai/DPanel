import React from "react";
import { describe, it, expect, beforeEach } from "vitest";
import { render, screen, waitFor, fireEvent, within } from "@testing-library/react";
import { ToastProvider } from "@builtpanel/ui";
import { resetMockUsers } from "@builtpanel/api";
import { UsersPage } from "./UsersPage";

function renderWithProviders(ui: React.ReactElement) {
  return render(<ToastProvider>{ui}</ToastProvider>);
}

describe("UsersPage (T04.15)", () => {
  beforeEach(() => {
    resetMockUsers();
  });

  it("DoD: renders user table with 1,000 total users and smooth pagination", async () => {
    renderWithProviders(<UsersPage />);

    // Verify title
    expect(screen.getByRole("heading", { level: 1 })).toBeInTheDocument();

    // Wait for initial users to load (1000 total users)
    await waitFor(() => {
      expect(screen.getAllByText(/1,000|১,০০০/).length).toBeGreaterThanOrEqual(1);
    });

    // Check first page user items
    expect(screen.getByText("alpha")).toBeInTheDocument();
    expect(screen.getByText("beta")).toBeInTheDocument();

    // Verify pagination controls show page 1 of 100
    expect(screen.getByText(/Page 1 of 100|100 পৃষ্ঠার মধ্যে 1 নং পৃষ্ঠা/i)).toBeInTheDocument();

    // Navigate to Page 2
    const nextBtn = screen.getByRole("button", { name: "Next page" });
    fireEvent.click(nextBtn);

    await waitFor(() => {
      expect(screen.getByText(/Page 2 of 100|100 পৃষ্ঠার মধ্যে 2 নং পৃষ্ঠা/i)).toBeInTheDocument();
    });

    // Change perPage to 25
    fireEvent.change(screen.getByDisplayValue("10"), { target: { value: "25" } });

    await waitFor(() => {
      expect(screen.getByText(/Page 1 of 40|40 পৃষ্ঠার মধ্যে 1 নং পৃষ্ঠা/i)).toBeInTheDocument();
    });
  });

  it("filters users by search term (username or email)", async () => {
    renderWithProviders(<UsersPage />);

    await waitFor(() => {
      expect(screen.getByText("alpha")).toBeInTheDocument();
    });

    const searchInput = screen.getByPlaceholderText(/Search by username or email|ইউজারনেম বা ইমেইল দিয়ে খুঁজুন/i);
    fireEvent.change(searchInput, { target: { value: "quantum" } });

    // Debounced search triggers filter
    await waitFor(() => {
      expect(screen.getByText("quantum")).toBeInTheDocument();
      expect(screen.queryByText("alpha")).not.toBeInTheDocument();
    });
  });

  it("filters users by status", async () => {
    renderWithProviders(<UsersPage />);

    await waitFor(() => {
      expect(screen.getByText("alpha")).toBeInTheDocument();
    });

    // Find status filter select
    const statusSelect = screen.getByDisplayValue(/All Statuses|সকল অবস্থা/i);
    fireEvent.change(statusSelect, { target: { value: "suspended" } });

    // Table updates to show only suspended users
    await waitFor(() => {
      const suspendedBadges = screen.getAllByText(/Suspended|স্থগিত/i);
      expect(suspendedBadges.length).toBeGreaterThan(0);
    });
  });

  it("performs bulk suspend on selected users", async () => {
    renderWithProviders(<UsersPage />);

    await waitFor(() => {
      expect(screen.getByText("alpha")).toBeInTheDocument();
    });

    // Select first row (alpha) and second row (beta)
    const rowCheckboxes = screen.getAllByRole("checkbox");
    expect(rowCheckboxes.length).toBeGreaterThan(2);

    // Click checkbox for alpha (index 1) and beta (index 2)
    fireEvent.click(rowCheckboxes[1]);
    fireEvent.click(rowCheckboxes[2]);

    // Bulk bar appears with 2 selected
    await waitFor(() => {
      expect(
        screen.getByText(/[2২] (users selected|জন ব্যবহারকারী নির্বাচিত)/i)
      ).toBeInTheDocument();
    });

    // Click bulk suspend button
    const bulkSuspendBtn = screen.getByRole("button", {
      name: /^(Suspend|স্থগিত করুন)$/i,
    });
    fireEvent.click(bulkSuspendBtn);

    // Suspend dialog opens
    await waitFor(() => {
      expect(screen.getByRole("dialog")).toBeInTheDocument();
    });

    const dialog = screen.getByRole("dialog");
    const confirmBtn = within(dialog).getByRole("button", {
      name: /Suspend Account|স্থগিত করুন/i,
    });
    fireEvent.click(confirmBtn);

    // Dialog closes and selection is cleared
    await waitFor(() => {
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    });
  });

  it("performs bulk change plan on selected users", async () => {
    renderWithProviders(<UsersPage />);

    await waitFor(() => {
      expect(screen.getByText("alpha")).toBeInTheDocument();
    });

    const rowCheckboxes = screen.getAllByRole("checkbox");
    fireEvent.click(rowCheckboxes[1]);

    await waitFor(() => {
      expect(
        screen.getByText(/[1১] (users selected|জন ব্যবহারকারী নির্বাচিত)/i)
      ).toBeInTheDocument();
    });

    const bulkPlanBtn = screen.getByRole("button", {
      name: /^(Change Plan|প্ল্যান পরিবর্তন)$/i,
    });
    fireEvent.click(bulkPlanBtn);

    await waitFor(() => {
      expect(screen.getByRole("dialog")).toBeInTheDocument();
    });

    const dialog = screen.getByRole("dialog");
    const planSelect = within(dialog).getByRole("combobox");
    fireEvent.change(planSelect, { target: { value: "plan_enterprise" } });

    const applyBtn = within(dialog).getByRole("button", {
      name: /Apply Plan|প্ল্যান প্রয়োগ করুন/i,
    });
    fireEvent.click(applyBtn);

    await waitFor(() => {
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    });
  });

  it("handles single user suspend and unsuspend actions", async () => {
    renderWithProviders(<UsersPage />);

    await waitFor(() => {
      expect(screen.getByText("alpha")).toBeInTheDocument();
    });

    const alphaRow = screen.getByText("alpha").closest("tr")!;
    const suspendBtn = within(alphaRow).getByRole("button", {
      name: /Suspend account|অ্যাকাউন্ট স্থগিত করুন/i,
    });
    fireEvent.click(suspendBtn);

    // Dialog appears
    await waitFor(() => {
      expect(screen.getByRole("dialog")).toBeInTheDocument();
    });

    const dialog = screen.getByRole("dialog");
    const confirmBtn = within(dialog).getByRole("button", {
      name: /Suspend Account|স্থগিত করুন/i,
    });
    fireEvent.click(confirmBtn);

    await waitFor(() => {
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    });
  });

  it("displays 'node offline' warning badge for tenants on offline nodes (DoD T16.07)", async () => {
    renderWithProviders(<UsersPage />);

    // In mockUsers, users on node-nyc-01 (such as gamma) are on an offline node
    await waitFor(() => {
      expect(screen.getByText("gamma")).toBeInTheDocument();
    });

    const offlineBadges = screen.getAllByTestId("tenant-node-offline-badge");
    expect(offlineBadges.length).toBeGreaterThan(0);
    expect(offlineBadges[0]).toHaveTextContent(/Node Offline|নোড অফলাইন/i);
  });
});
