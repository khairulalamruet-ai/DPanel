import React from "react";
import { describe, it, expect, beforeEach } from "vitest";
import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import { ToastProvider } from "@builtpanel/ui";
import { resetMockDns } from "@builtpanel/api";
import { DnsEditorPage } from "./DnsEditorPage";

function renderWithProviders(ui: React.ReactElement) {
  return render(<ToastProvider>{ui}</ToastProvider>);
}

describe("DnsEditorPage (T05.11)", () => {
  beforeEach(() => {
    resetMockDns();
  });

  it("renders page title, nameserver info banner, zone selector, and initial seeded records", async () => {
    renderWithProviders(<DnsEditorPage />);

    expect(screen.getByRole("heading", { level: 1 })).toBeInTheDocument();

    // Verify initial zone records loaded
    await waitFor(() => {
      expect(screen.getByText("192.168.1.100")).toBeInTheDocument();
    });

    expect(screen.getByText("alpha-hosting.com.")).toBeInTheDocument();
    expect(screen.getByText("mail.alpha-hosting.com.")).toBeInTheDocument();
    expect(screen.getByText("v=spf1 a mx ~all")).toBeInTheDocument();
    expect(screen.getByText("2001:db8::1")).toBeInTheDocument();

    // Verify nameserver info
    expect(screen.getByText(/ns1\.builtpanel\.net/)).toBeInTheDocument();
    expect(screen.getByTestId("btn-verify-ns")).toBeInTheDocument();
  });

  it("filters records by type pill button and search keyword", async () => {
    renderWithProviders(<DnsEditorPage />);

    await waitFor(() => {
      expect(screen.getByText("192.168.1.100")).toBeInTheDocument();
    });

    // Filter by MX type pill button
    const mxFilterBtn = screen.getByTestId("btn-filter-type-MX");
    fireEvent.click(mxFilterBtn);

    // MX record is visible, A and CNAME are hidden
    expect(screen.getByText("mail.alpha-hosting.com.")).toBeInTheDocument();
    expect(screen.queryByText("192.168.1.100")).not.toBeInTheDocument();
    expect(screen.queryByText("alpha-hosting.com.")).not.toBeInTheDocument();

    // Reset to ALL
    const allFilterBtn = screen.getByTestId("btn-filter-type-ALL");
    fireEvent.click(allFilterBtn);

    // Search input filtering
    const searchInput = screen.getByTestId("input-search-dns-records");
    fireEvent.change(searchInput, { target: { value: "spf1" } });

    expect(screen.getByText("v=spf1 a mx ~all")).toBeInTheDocument();
    expect(screen.queryByText("192.168.1.100")).not.toBeInTheDocument();
  });

  it("creates a new DNS record using the dialog", async () => {
    renderWithProviders(<DnsEditorPage />);

    await waitFor(() => {
      expect(screen.getByText("192.168.1.100")).toBeInTheDocument();
    });

    // Click Add Record
    const addBtn = screen.getByTestId("btn-add-dns-record");
    fireEvent.click(addBtn);

    // Dialog appears
    await waitFor(() => {
      expect(screen.getByTestId("btn-submit-dns-record")).toBeInTheDocument();
    });

    // Fill form
    const nameInput = screen.getByTestId("input-record-name");
    fireEvent.change(nameInput, { target: { value: "api" } });

    const contentInput = screen.getByTestId("input-record-content");
    fireEvent.change(contentInput, { target: { value: "192.168.1.150" } });

    // Submit
    const submitBtn = screen.getByTestId("btn-submit-dns-record");
    fireEvent.click(submitBtn);

    // Dialog closes and new record appears in table
    await waitFor(() => {
      expect(screen.getByText("192.168.1.150")).toBeInTheDocument();
      expect(screen.getByText("api")).toBeInTheDocument();
    });
  });

  it("DoD: performs inline editing on a DNS record", async () => {
    renderWithProviders(<DnsEditorPage />);

    await waitFor(() => {
      expect(screen.getByText("192.168.1.100")).toBeInTheDocument();
    });

    // Click inline edit pencil on rec_01
    const editBtn = screen.getByTestId("btn-edit-record-rec_01");
    fireEvent.click(editBtn);

    // Inline row appears
    await waitFor(() => {
      expect(screen.getByTestId("inline-edit-row-rec_01")).toBeInTheDocument();
    });

    const contentInput = screen.getByTestId("input-inline-content-rec_01");
    fireEvent.change(contentInput, { target: { value: "192.168.1.200" } });

    // Click inline save (Check)
    const saveBtn = screen.getByTestId("btn-save-inline-rec_01");
    fireEvent.click(saveBtn);

    // Row returns to normal and shows updated IP
    await waitFor(() => {
      expect(screen.getByText("192.168.1.200")).toBeInTheDocument();
      expect(screen.queryByTestId("inline-edit-row-rec_01")).not.toBeInTheDocument();
    });
  });

  it("deletes a DNS record", async () => {
    renderWithProviders(<DnsEditorPage />);

    await waitFor(() => {
      expect(screen.getByText("192.168.1.105")).toBeInTheDocument();
    });

    // Click delete on rec_06 (mail A 192.168.1.105)
    const deleteBtn = screen.getByTestId("btn-delete-record-rec_06");
    fireEvent.click(deleteBtn);

    // Confirm dialog appears
    await waitFor(() => {
      expect(screen.getByTestId("btn-confirm-delete-record")).toBeInTheDocument();
    });

    // Confirm deletion
    const confirmBtn = screen.getByTestId("btn-confirm-delete-record");
    fireEvent.click(confirmBtn);

    // Record removed from table
    await waitFor(() => {
      expect(screen.queryByText("192.168.1.105")).not.toBeInTheDocument();
    });
  });

  it("DoD: opens BIND zone management dialog to export and import zone records", async () => {
    renderWithProviders(<DnsEditorPage />);

    await waitFor(() => {
      expect(screen.getByText("192.168.1.100")).toBeInTheDocument();
    });

    // Click BIND Zone button
    const bindBtn = screen.getByTestId("btn-bind-export-import");
    fireEvent.click(bindBtn);

    // Dialog appears with exported BIND text
    await waitFor(() => {
      expect(screen.getByTestId("pre-bind-export")).toBeInTheDocument();
    });

    const exportPre = screen.getByTestId("pre-bind-export");
    expect(exportPre.textContent).toContain("$ORIGIN alpha-hosting.com.");
    expect(exportPre.textContent).toContain("IN\tSOA");
    expect(exportPre.textContent).toContain("192.168.1.100");

    // Switch to Import Tab
    const importTab = screen.getByTestId("tab-bind-import");
    fireEvent.click(importTab);

    // Textarea appears
    await waitFor(() => {
      expect(screen.getByTestId("textarea-bind-import")).toBeInTheDocument();
    });

    const importTextarea = screen.getByTestId("textarea-bind-import");
    fireEvent.change(importTextarea, {
      target: {
        value: "staging IN A 192.168.1.222\napp IN CNAME alpha-hosting.com.",
      },
    });

    // Submit import
    const submitImportBtn = screen.getByTestId("btn-submit-bind-import");
    fireEvent.click(submitImportBtn);

    // Success notice
    await waitFor(() => {
      expect(screen.getByTestId("bind-import-success-notice")).toBeInTheDocument();
    });
  });
});
