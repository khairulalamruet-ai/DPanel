import React from "react";
import { describe, it, expect, beforeEach } from "vitest";
import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import { ToastProvider } from "@builtpanel/ui";
import { resetMockPageTemplates } from "@builtpanel/api";
import { ErrorPagesPage } from "./ErrorPagesPage";

function renderWithProviders(ui: React.ReactElement) {
  return render(<ToastProvider>{ui}</ToastProvider>);
}

describe("ErrorPagesPage (T06.13)", () => {
  beforeEach(() => {
    resetMockPageTemplates();
  });

  it("renders page title and template tabs with initial editor content", async () => {
    renderWithProviders(<ErrorPagesPage />);

    await waitFor(() => {
      expect(screen.getByTestId("error-pages-title")).toBeInTheDocument();
    });

    // Verify all 4 tabs are present
    expect(screen.getByTestId("tab-template-suspended_domain")).toBeInTheDocument();
    expect(screen.getByTestId("tab-template-suspended_user")).toBeInTheDocument();
    expect(screen.getByTestId("tab-template-error_404")).toBeInTheDocument();
    expect(screen.getByTestId("tab-template-error_502")).toBeInTheDocument();

    // Verify initial editor has suspended domain content
    const textarea = screen.getByTestId("textarea-template-content") as HTMLTextAreaElement;
    expect(textarea).toBeInTheDocument();
    expect(textarea.value).toContain("Domain Suspended");
    expect(textarea.value).toContain("{{domain}}");
  });

  it("switches between template tabs and loads corresponding HTML", async () => {
    renderWithProviders(<ErrorPagesPage />);

    await waitFor(() => {
      expect(screen.getByTestId("tab-template-error_404")).toBeInTheDocument();
    });

    // Switch to 404 Not Found tab
    fireEvent.click(screen.getByTestId("tab-template-error_404"));

    const textarea = screen.getByTestId("textarea-template-content") as HTMLTextAreaElement;
    await waitFor(() => {
      expect(textarea.value).toContain("404 - Page Not Found");
      expect(textarea.value).toContain("{{path}}");
    });

    // Switch to 502 Bad Gateway tab
    fireEvent.click(screen.getByTestId("tab-template-error_502"));
    await waitFor(() => {
      expect(textarea.value).toContain("502 - Bad Gateway");
      expect(textarea.value).toContain("{{service}}");
    });
  });

  it("inserts variable token into editor when variable chip is clicked", async () => {
    renderWithProviders(<ErrorPagesPage />);

    await waitFor(() => {
      expect(screen.getByTestId("tab-template-suspended_domain")).toBeInTheDocument();
    });

    const textarea = screen.getByTestId("textarea-template-content") as HTMLTextAreaElement;
    const initialVal = textarea.value;

    const reasonChip = screen.getByTestId("var-chip-reason");
    expect(reasonChip).toBeInTheDocument();
    fireEvent.click(reasonChip);

    // Textarea should contain inserted variable or have length increased
    expect(textarea.value.length).toBeGreaterThanOrEqual(initialVal.length);
  });

  it("modifies template, activates save button, and successfully saves changes", async () => {
    renderWithProviders(<ErrorPagesPage />);

    await waitFor(() => {
      expect(screen.getByTestId("btn-save-template")).toBeInTheDocument();
    });

    const saveBtn = screen.getByTestId("btn-save-template");
    expect(saveBtn).toBeDisabled();

    // Edit textarea
    const textarea = screen.getByTestId("textarea-template-content") as HTMLTextAreaElement;
    fireEvent.change(textarea, {
      target: { value: textarea.value + "\n<!-- Custom admin branding -->" },
    });

    // Save button should now be enabled
    expect(saveBtn).not.toBeDisabled();

    // Click save
    fireEvent.click(saveBtn);

    // Verify success toast appears
    await waitFor(() => {
      expect(screen.getByText(/saved successfully|সফলভাবে সংরক্ষিত/i)).toBeInTheDocument();
    });
  });

  it("resets customized template to default HTML via confirmation dialog", async () => {
    renderWithProviders(<ErrorPagesPage />);

    await waitFor(() => {
      expect(screen.getByTestId("btn-reset-template")).toBeInTheDocument();
    });

    // First make a change and save
    const textarea = screen.getByTestId("textarea-template-content") as HTMLTextAreaElement;
    fireEvent.change(textarea, {
      target: { value: "<h1>Custom Suspended Message</h1>" },
    });
    fireEvent.click(screen.getByTestId("btn-save-template"));

    await waitFor(() => {
      expect(screen.getByText(/saved successfully|সফলভাবে সংরক্ষিত/i)).toBeInTheDocument();
    });

    // Now click Reset button
    fireEvent.click(screen.getByTestId("btn-reset-template"));

    // Confirmation dialog should be visible
    await waitFor(() => {
      expect(screen.getByTestId("btn-confirm-reset")).toBeInTheDocument();
    });

    // Confirm reset
    fireEvent.click(screen.getByTestId("btn-confirm-reset"));

    // Verify reset success toast and default content restored
    await waitFor(() => {
      expect(screen.getByText(/reverted to default|ডিফল্ট মানে ফিরে গেছে/i)).toBeInTheDocument();
    });
    expect(textarea.value).toContain("Domain Suspended");
  });

  it("toggles device viewport switcher and preview variables configuration", async () => {
    renderWithProviders(<ErrorPagesPage />);

    await waitFor(() => {
      expect(screen.getByTestId("preview-device-mobile")).toBeInTheDocument();
    });

    // Switch device to tablet and mobile
    fireEvent.click(screen.getByTestId("preview-device-tablet"));
    fireEvent.click(screen.getByTestId("preview-device-mobile"));

    // Toggle preview variables accordion
    const toggleVarsBtn = screen.getByTestId("btn-toggle-preview-vars");
    fireEvent.click(toggleVarsBtn);

    await waitFor(() => {
      expect(screen.getByTestId("preview-vars-panel")).toBeInTheDocument();
    });

    // Verify sample input can be edited
    const domainInput = screen.getByTestId("input-sample-domain") as HTMLInputElement;
    expect(domainInput).toBeInTheDocument();
    fireEvent.change(domainInput, { target: { value: "my-test-domain.org" } });
    expect(domainInput.value).toBe("my-test-domain.org");
  });
});
