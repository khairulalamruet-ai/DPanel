import React from "react";
import { describe, it, expect, beforeEach } from "vitest";
import { render, screen, waitFor, fireEvent, within } from "@testing-library/react";
import { ToastProvider } from "@builtpanel/ui";
import { resetMockFeatureSets } from "@builtpanel/api";
import { FeatureSetsPage } from "./FeatureSetsPage";

function renderWithProviders(ui: React.ReactElement) {
  return render(<ToastProvider>{ui}</ToastProvider>);
}

describe("FeatureSetsPage (T04.02)", () => {
  beforeEach(() => {
    resetMockFeatureSets();
  });

  it("renders page title and initial seeded feature sets", async () => {
    renderWithProviders(<FeatureSetsPage />);

    expect(screen.getByRole("heading", { level: 1 })).toBeInTheDocument();

    await waitFor(() => {
      expect(screen.getByText("Default (Full Stack)")).toBeInTheDocument();
    });

    expect(screen.getByText("Basic Web")).toBeInTheDocument();
    expect(screen.getByText("DNS Only")).toBeInTheDocument();
    expect(screen.getByText("Database Only")).toBeInTheDocument();
  });

  it("DoD: creates a new feature set from the dialog", async () => {
    renderWithProviders(<FeatureSetsPage />);

    await waitFor(() => {
      expect(screen.getByText("Default (Full Stack)")).toBeInTheDocument();
    });

    // Click "New Feature Set" button
    const newBtn = screen.getByRole("button", { name: /New Feature Set|নতুন ফিচার সেট/i });
    fireEvent.click(newBtn);

    // Dialog appears
    await waitFor(() => {
      expect(
        screen.getByText(/Create Feature Set|নতুন ফিচার সেট তৈরি করুন/i)
      ).toBeInTheDocument();
    });

    const dialog = screen.getByRole("dialog");
    expect(dialog).toBeInTheDocument();

    // Fill Name
    const nameInput = within(dialog).getByLabelText(/Feature Set Name|ফিচার সেটের নাম/i);
    fireEvent.change(nameInput, { target: { value: "Pro Business Stack" } });

    // Click Save
    const saveBtn = within(dialog).getByRole("button", {
      name: /Save Feature Set|সংরক্ষণ করুন/i,
    });
    fireEvent.click(saveBtn);

    // Verify dialog closes and new set is displayed in the list
    await waitFor(() => {
      expect(screen.getByText("Pro Business Stack")).toBeInTheDocument();
    });
  });

  it("DoD: edits an existing feature set from the dialog", async () => {
    renderWithProviders(<FeatureSetsPage />);

    await waitFor(() => {
      expect(screen.getByText("Basic Web")).toBeInTheDocument();
    });

    // Locate Basic Web row and click Edit
    const basicRow = screen.getByText("Basic Web").closest("tr")!;
    const editBtn = within(basicRow).getByRole("button", { name: /Edit|সম্পাদনা/i });
    fireEvent.click(editBtn);

    // Dialog appears in edit mode
    await waitFor(() => {
      expect(
        screen.getByText(/Edit Feature Set|ফিচার সেট সম্পাদনা করুন/i)
      ).toBeInTheDocument();
    });

    const dialog = screen.getByRole("dialog");
    const nameInput = within(dialog).getByLabelText(/Feature Set Name|ফিচার সেটের নাম/i);
    fireEvent.change(nameInput, { target: { value: "Basic Web Modified" } });

    // Save changes
    const saveBtn = within(dialog).getByRole("button", {
      name: /Save Feature Set|সংরক্ষণ করুন/i,
    });
    fireEvent.click(saveBtn);

    // Verify updated title is in table
    await waitFor(() => {
      expect(screen.getByText("Basic Web Modified")).toBeInTheDocument();
    });
  });

  it("disables delete for default feature set", async () => {
    renderWithProviders(<FeatureSetsPage />);

    await waitFor(() => {
      expect(screen.getByText("Default (Full Stack)")).toBeInTheDocument();
    });

    const defaultRow = screen.getByText("Default (Full Stack)").closest("tr")!;
    const deleteBtn = within(defaultRow).getByRole("button", { name: /Delete|মুছে ফেলুন/i });
    expect(deleteBtn).toBeDisabled();
  });
});
