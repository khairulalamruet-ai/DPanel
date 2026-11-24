import React from "react";
import { describe, it, expect, vi } from "vitest";
import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import { ToastProvider } from "@builtpanel/ui";
import { ClientBackupsPage } from "./ClientBackupsPage";

function renderWithProviders(ui: React.ReactElement) {
  return render(<ToastProvider>{ui}</ToastProvider>);
}

describe("ClientBackupsPage (T09.09)", () => {
  it("renders page title, summary metric cards, and seed backups", async () => {
    renderWithProviders(<ClientBackupsPage />);

    await waitFor(() => {
      expect(screen.getByTestId("backups-page-title")).toBeInTheDocument();
    });

    expect(screen.getByTestId("card-metric-total-backups")).toBeInTheDocument();
    expect(screen.getByTestId("card-metric-storage-used")).toBeInTheDocument();
    expect(screen.getByTestId("card-metric-last-backup")).toBeInTheDocument();

    await waitFor(() => {
      expect(screen.getByTestId("metric-backup-count")).toHaveTextContent("2");
      expect(screen.getByTestId("metric-backup-storage")).toHaveTextContent("500.0 MB");
      expect(screen.getByTestId("backup-name-bk_01")).toBeInTheDocument();
      expect(screen.getByTestId("backup-name-bk_02")).toBeInTheDocument();
    });
  });

  it("filters backups table using search input", async () => {
    renderWithProviders(<ClientBackupsPage />);

    await waitFor(() => {
      expect(screen.getByTestId("backup-name-bk_01")).toBeInTheDocument();
    });

    const searchInput = screen.getByTestId("input-search-backups");
    fireEvent.change(searchInput, { target: { value: "schedule" } });

    expect(screen.getByTestId("backup-name-bk_02")).toBeInTheDocument();
    expect(screen.queryByTestId("backup-name-bk_01")).not.toBeInTheDocument();
  });

  it("makes a backup now (the whole account) via CreateBackupDialog", async () => {
    renderWithProviders(<ClientBackupsPage />);

    await waitFor(() => {
      expect(screen.getByTestId("btn-open-create-backup")).toBeInTheDocument();
    });

    fireEvent.click(screen.getByTestId("btn-open-create-backup"));

    await waitFor(() => {
      expect(screen.getByTestId("create-backup-dialog-title")).toBeInTheDocument();
    });

    fireEvent.click(screen.getByTestId("btn-submit-create-backup"));

    await waitFor(() => {
      expect(screen.queryByTestId("create-backup-dialog-title")).not.toBeInTheDocument();
    });
  });

  it("opens restore wizard and submits full restore with safety snapshot", async () => {
    renderWithProviders(<ClientBackupsPage />);

    await waitFor(() => {
      expect(screen.getByTestId("btn-restore-bk_01")).toBeInTheDocument();
    });

    fireEvent.click(screen.getByTestId("btn-restore-bk_01"));

    await waitFor(() => {
      expect(screen.getByTestId("restore-wizard-title")).toBeInTheDocument();
    });

    // Verify default safety snapshot is checked
    expect(screen.getByTestId("checkbox-safety-snapshot")).toBeChecked();

    // Submit restore
    fireEvent.click(screen.getByTestId("btn-submit-restore"));

    await waitFor(() => {
      expect(screen.queryByTestId("restore-wizard-title")).not.toBeInTheDocument();
    });
  });

  it("switches to selective restore mode and allows picking specific databases", async () => {
    renderWithProviders(<ClientBackupsPage />);

    await waitFor(() => {
      expect(screen.getByTestId("btn-restore-bk_01")).toBeInTheDocument();
    });

    fireEvent.click(screen.getByTestId("btn-restore-bk_01"));

    await waitFor(() => {
      expect(screen.getByTestId("mode-select-selective")).toBeInTheDocument();
    });

    fireEvent.click(screen.getByTestId("mode-select-selective"));

    await waitFor(() => {
      expect(screen.getByTestId("selective-components-area")).toBeInTheDocument();
      expect(screen.getByTestId("checkbox-select-db-demo_wordpress")).toBeInTheDocument();
    });

    fireEvent.click(screen.getByTestId("btn-submit-restore"));

    await waitFor(() => {
      expect(screen.queryByTestId("restore-wizard-title")).not.toBeInTheDocument();
    });
  });

  it("deletes a backup after confirmation", async () => {
    const confirmSpy = vi.spyOn(window, "confirm").mockImplementation(() => true);

    renderWithProviders(<ClientBackupsPage />);

    await waitFor(() => {
      expect(screen.getByTestId("btn-delete-backup-bk_02")).toBeInTheDocument();
    });

    fireEvent.click(screen.getByTestId("btn-delete-backup-bk_02"));

    await waitFor(() => {
      expect(screen.queryByTestId("backup-name-bk_02")).not.toBeInTheDocument();
    });

    confirmSpy.mockRestore();
  });
});
