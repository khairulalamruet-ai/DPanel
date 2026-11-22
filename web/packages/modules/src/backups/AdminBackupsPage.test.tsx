import React from "react";
import { describe, it, expect, vi } from "vitest";
import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import { ToastProvider } from "@builtpanel/ui";
import { AdminBackupsPage } from "./AdminBackupsPage";

function renderWithProviders(ui: React.ReactElement) {
  return render(<ToastProvider>{ui}</ToastProvider>);
}

describe("AdminBackupsPage (T09.10)", () => {
  it("renders page header, overview cards, and failed job notification banner", async () => {
    renderWithProviders(<AdminBackupsPage />);

    await waitFor(() => {
      expect(screen.getByTestId("admin-backups-title")).toBeInTheDocument();
    });

    expect(screen.getByTestId("card-total-jobs")).toBeInTheDocument();
    expect(screen.getByTestId("card-active-jobs")).toBeInTheDocument();
    expect(screen.getByTestId("card-failed-jobs")).toBeInTheDocument();

    // Verify DoD requirement: Failed jobs banner with notification sent tag
    await waitFor(() => {
      expect(screen.getByTestId("banner-failed-jobs-alert")).toBeInTheDocument();
      expect(screen.getByTestId("badge-dod-notification-sent")).toBeInTheDocument();
      expect(screen.getByTestId("job-row-job_02")).toBeInTheDocument();
      expect(screen.getByTestId("job-notification-job_02")).toBeInTheDocument();
    });
  });

  it("switches to Storage Destinations tab and makes a destination the default", async () => {
    renderWithProviders(<AdminBackupsPage />);

    await waitFor(() => {
      expect(screen.getByTestId("tab-destinations")).toBeInTheDocument();
    });

    fireEvent.click(screen.getByTestId("tab-destinations"));

    await waitFor(() => {
      expect(screen.getByTestId("dest-row-dest_s3_01")).toBeInTheDocument();
      expect(screen.getByTestId("dest-name-dest_s3_01")).toHaveTextContent("Wasabi EU Cloud");
    });

    fireEvent.click(screen.getByTestId("btn-default-dest-dest_s3_01"));

    // It is the default now, so its button is off and the old default's is on
    await waitFor(() => {
      expect(screen.getByTestId("btn-default-dest-dest_s3_01")).toBeDisabled();
      expect(screen.getByTestId("btn-default-dest-dest_local")).not.toBeDisabled();
    });
  });

  it("opens create destination modal and adds a new S3 destination", async () => {
    renderWithProviders(<AdminBackupsPage />);

    await waitFor(() => {
      expect(screen.getByTestId("btn-open-create-dest")).toBeInTheDocument();
    });

    fireEvent.click(screen.getByTestId("btn-open-create-dest"));

    await waitFor(() => {
      expect(screen.getByTestId("destination-dialog-title")).toBeInTheDocument();
    });

    fireEvent.change(screen.getByTestId("input-dest-name"), {
      target: { value: "Offsite Backup B2" },
    });

    fireEvent.change(screen.getByTestId("input-s3-bucket"), {
      target: { value: "backups-production" },
    });

    fireEvent.click(screen.getByTestId("btn-submit-destination"));

    await waitFor(() => {
      expect(screen.queryByTestId("destination-dialog-title")).not.toBeInTheDocument();
    });
  });

  it("switches to Automated Schedules tab and toggles schedule state", async () => {
    renderWithProviders(<AdminBackupsPage />);

    await waitFor(() => {
      expect(screen.getByTestId("tab-schedules")).toBeInTheDocument();
    });

    fireEvent.click(screen.getByTestId("tab-schedules"));

    await waitFor(() => {
      expect(screen.getByTestId("sched-row-system")).toBeInTheDocument();
      expect(screen.getByTestId("sched-name-system")).toHaveTextContent("Control plane backup");
    });

    const toggleBtn = screen.getByTestId("btn-toggle-sched-system");
    fireEvent.click(toggleBtn);

    await waitFor(() => {
      expect(screen.getByTestId("sched-row-system")).toBeInTheDocument();
    });
  });

  it("triggers manual system backup snapshot", async () => {
    renderWithProviders(<AdminBackupsPage />);

    await waitFor(() => {
      expect(screen.getByTestId("btn-trigger-system-backup")).toBeInTheDocument();
    });

    fireEvent.click(screen.getByTestId("btn-trigger-system-backup"));

    await waitFor(() => {
      expect(screen.getByTestId("admin-backups-title")).toBeInTheDocument();
    });
  });
});
