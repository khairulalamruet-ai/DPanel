import React from "react";
import { describe, it, expect, beforeEach } from "vitest";
import { render, screen, waitFor, fireEvent, within } from "@testing-library/react";
import { ToastProvider } from "@builtpanel/ui";
import { resetMockPlans } from "@builtpanel/api";
import { PlansPage } from "./PlansPage";

function renderWithProviders(ui: React.ReactElement) {
  return render(<ToastProvider>{ui}</ToastProvider>);
}

describe("PlansPage (T04.04)", () => {
  beforeEach(() => {
    resetMockPlans();
  });

  it("renders page title, initial plans, and assigned users count", async () => {
    renderWithProviders(<PlansPage />);

    expect(screen.getByRole("heading", { level: 1 })).toBeInTheDocument();

    await waitFor(() => {
      expect(screen.getByText("Standard Business")).toBeInTheDocument();
    });

    expect(screen.getByText("Starter Cloud")).toBeInTheDocument();
    expect(screen.getByText("Pro Developer")).toBeInTheDocument();
    expect(screen.getByText("Enterprise Dedicated")).toBeInTheDocument();

    // Check users count requirement: কতজন user এই plan-এ আছে দেখানো
    expect(screen.getByText(/24 জন ব্যবহারকারী|24 users/i)).toBeInTheDocument();
    expect(screen.getByText(/8 জন ব্যবহারকারী|8 users/i)).toBeInTheDocument();
  });

  it("creates a new hosting plan with unlimited toggles from UI dialog", async () => {
    renderWithProviders(<PlansPage />);

    await waitFor(() => {
      expect(screen.getByText("Standard Business")).toBeInTheDocument();
    });

    // Click "New Plan" button
    const newBtn = screen.getByRole("button", { name: /New Plan|নতুন প্ল্যান/i });
    fireEvent.click(newBtn);

    // Dialog appears
    await waitFor(() => {
      expect(
        screen.getByText(/Create Hosting Plan|নতুন হোস্টিং প্ল্যান তৈরি করুন/i)
      ).toBeInTheDocument();
    });

    const dialog = screen.getByRole("dialog");
    expect(dialog).toBeInTheDocument();

    // Fill Name
    const nameInput = within(dialog).getByLabelText(/Plan Name|প্ল্যানের নাম/i);
    fireEvent.change(nameInput, { target: { value: "Startup Scale" } });

    // Toggle Unlimited on CPU limit
    const unlimitedButtons = within(dialog).getAllByRole("button", {
      name: /Unlimited|আনলিমিটেড/i,
    });
    expect(unlimitedButtons.length).toBeGreaterThan(0);
    // Click first unlimited button (CPU)
    fireEvent.click(unlimitedButtons[0]);

    // Save plan
    const saveBtn = within(dialog).getByRole("button", {
      name: /Save Plan|সংরক্ষণ করুন/i,
    });
    fireEvent.click(saveBtn);

    // Dialog closes and new plan appears
    await waitFor(() => {
      expect(screen.getByText("Startup Scale")).toBeInTheDocument();
    });
  });

  it("edits an existing hosting plan from UI dialog", async () => {
    renderWithProviders(<PlansPage />);

    await waitFor(() => {
      expect(screen.getByText("Starter Cloud")).toBeInTheDocument();
    });

    const starterRow = screen.getByText("Starter Cloud").closest("tr")!;
    const editBtn = within(starterRow).getByRole("button", { name: /Edit|সম্পাদনা/i });
    fireEvent.click(editBtn);

    await waitFor(() => {
      expect(
        screen.getByText(/Edit Hosting Plan|হোস্টিং প্ল্যান সম্পাদনা করুন/i)
      ).toBeInTheDocument();
    });

    const dialog = screen.getByRole("dialog");
    const nameInput = within(dialog).getByLabelText(/Plan Name|প্ল্যানের নাম/i);
    fireEvent.change(nameInput, { target: { value: "Starter Cloud Plus" } });

    const saveBtn = within(dialog).getByRole("button", {
      name: /Save Plan|সংরক্ষণ করুন/i,
    });
    fireEvent.click(saveBtn);

    await waitFor(() => {
      expect(screen.getByText("Starter Cloud Plus")).toBeInTheDocument();
    });
  });

  it("disables delete for default plan or plans with active users", async () => {
    renderWithProviders(<PlansPage />);

    await waitFor(() => {
      expect(screen.getByText("Standard Business")).toBeInTheDocument();
    });

    // Standard Business is default and has 24 users -> delete button is disabled
    const standardRow = screen.getByText("Standard Business").closest("tr")!;
    const standardDeleteBtn = within(standardRow).getByRole("button", { name: /Delete|মুছে ফেলুন/i });
    expect(standardDeleteBtn).toBeDisabled();

    // Starter Cloud is not default but has 8 users -> delete button is disabled
    const starterRow = screen.getByText("Starter Cloud").closest("tr")!;
    const starterDeleteBtn = within(starterRow).getByRole("button", { name: /Delete|মুছে ফেলুন/i });
    expect(starterDeleteBtn).toBeDisabled();
  });

  it("renders node group badges in the plans table (T16.03 DoD)", async () => {
    renderWithProviders(<PlansPage />);

    await waitFor(() => {
      expect(screen.getByText("Standard Business")).toBeInTheDocument();
    });

    // Check node group column header
    expect(screen.getByText(/Node Group|নোড গ্রুপ/i)).toBeInTheDocument();

    // Check node group badges
    const badges = screen.getAllByTestId("plan-node-group-badge");
    expect(badges.length).toBeGreaterThanOrEqual(4);

    expect(screen.getByText("Global Cluster (Default)")).toBeInTheDocument();
    expect(screen.getByText("US-East General Pool")).toBeInTheDocument();
    expect(screen.getByText("Singapore High-RAM Pool")).toBeInTheDocument();
    expect(screen.getByText("Frankfurt EU-West Group")).toBeInTheDocument();
  });

  it("binds a new plan to a selected node group (T16.03 DoD)", async () => {
    renderWithProviders(<PlansPage />);

    await waitFor(() => {
      expect(screen.getByText("Standard Business")).toBeInTheDocument();
    });

    const newBtn = screen.getByRole("button", { name: /New Plan|নতুন প্ল্যান/i });
    fireEvent.click(newBtn);

    const dialog = await screen.findByRole("dialog");

    // Enter name
    const nameInput = within(dialog).getByLabelText(/Plan Name|প্ল্যানের নাম/i);
    fireEvent.change(nameInput, { target: { value: "APAC Enterprise" } });

    // Select Singapore High-RAM Pool node group
    const groupSelect = within(dialog).getByTestId("plan-node-group-select");
    fireEvent.change(groupSelect, { target: { value: "ng_sgp_highmem" } });

    // Verify tag badges appear
    expect(within(dialog).getByText("ap-southeast-1")).toBeInTheDocument();
    expect(within(dialog).getByText("highmem")).toBeInTheDocument();

    // Save
    const saveBtn = within(dialog).getByRole("button", { name: /Save Plan|সংরক্ষণ করুন/i });
    fireEvent.click(saveBtn);

    // Verify plan is listed with bound node group
    await waitFor(() => {
      expect(screen.getByText("APAC Enterprise")).toBeInTheDocument();
    });

    const row = screen.getByText("APAC Enterprise").closest("tr")!;
    expect(within(row).getByTestId("plan-node-group-badge")).toHaveTextContent("Singapore High-RAM Pool");
    expect(within(row).getByText("ap-southeast-1")).toBeInTheDocument();
  });
});
