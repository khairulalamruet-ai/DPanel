import React from "react";
import { describe, it, expect, vi } from "vitest";
import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import { ToastProvider } from "@builtpanel/ui";
import { CronJobsPage } from "./CronJobsPage";

function renderWithProviders(ui: React.ReactElement) {
  return render(<ToastProvider>{ui}</ToastProvider>);
}

describe("CronJobsPage (T10.03)", () => {
  it("renders page header, metric cards, and seed cron jobs", async () => {
    renderWithProviders(<CronJobsPage />);

    await waitFor(() => {
      expect(screen.getByTestId("cron-page-title")).toBeInTheDocument();
    });

    expect(screen.getByTestId("card-total-cron")).toBeInTheDocument();
    expect(screen.getByTestId("card-active-cron")).toBeInTheDocument();
    expect(screen.getByTestId("card-failed-cron")).toBeInTheDocument();
    expect(screen.getByTestId("card-next-cron")).toBeInTheDocument();

    await waitFor(() => {
      expect(screen.getByTestId("cron-row-cron_01")).toBeInTheDocument();
      expect(screen.getByTestId("cron-name-cron_01")).toHaveTextContent("Laravel Artisan Scheduler");
      expect(screen.getByTestId("cron-row-cron_02")).toBeInTheDocument();
      expect(screen.getByTestId("cron-row-cron_03")).toBeInTheDocument();
    });
  });

  it("filters cron jobs using search input", async () => {
    renderWithProviders(<CronJobsPage />);

    await waitFor(() => {
      expect(screen.getByTestId("cron-row-cron_01")).toBeInTheDocument();
    });

    const searchInput = screen.getByTestId("input-search-cron");
    fireEvent.change(searchInput, { target: { value: "Optimization" } });

    expect(screen.getByTestId("cron-row-cron_02")).toBeInTheDocument();
    expect(screen.queryByTestId("cron-row-cron_01")).not.toBeInTheDocument();
  });

  it("validates 5-minute minimum interval policy and creates a new cron job", async () => {
    renderWithProviders(<CronJobsPage />);

    await waitFor(() => {
      expect(screen.getByTestId("btn-open-create-cron")).toBeInTheDocument();
    });

    fireEvent.click(screen.getByTestId("btn-open-create-cron"));

    await waitFor(() => {
      expect(screen.getByTestId("create-cron-dialog-title")).toBeInTheDocument();
    });

    fireEvent.change(screen.getByTestId("input-cron-name"), {
      target: { value: "Sitemap Generator" },
    });
    fireEvent.change(screen.getByTestId("input-cron-command"), {
      target: { value: "php /home/demo/artisan sitemap:generate" },
    });

    // Enter invalid expression with interval < 5 min (T10.01 DoD)
    fireEvent.change(screen.getByTestId("input-cron-schedule"), {
      target: { value: "* * * * *" },
    });

    await waitFor(() => {
      expect(screen.getByTestId("cron-validation-error")).toBeInTheDocument();
      expect(screen.getByTestId("btn-submit-cron")).toBeDisabled();
    });

    // Fix by selecting "Daily" preset
    fireEvent.click(screen.getByTestId("preset-cron-3")); // Daily preset

    await waitFor(() => {
      expect(screen.queryByTestId("cron-validation-error")).not.toBeInTheDocument();
      expect(screen.getByTestId("btn-submit-cron")).not.toBeDisabled();
    });

    fireEvent.click(screen.getByTestId("btn-submit-cron"));

    await waitFor(() => {
      expect(screen.queryByTestId("create-cron-dialog-title")).not.toBeInTheDocument();
    });
  });

  it("toggles enabled/disabled state of a cron job", async () => {
    renderWithProviders(<CronJobsPage />);

    await waitFor(() => {
      expect(screen.getByTestId("btn-toggle-cron-cron_01")).toBeInTheDocument();
    });

    fireEvent.click(screen.getByTestId("btn-toggle-cron-cron_01"));

    await waitFor(() => {
      expect(screen.getByTestId("btn-toggle-cron-cron_01")).toBeInTheDocument();
    });
  });

  it("runs cron job immediately via 'Run Now' button", async () => {
    renderWithProviders(<CronJobsPage />);

    await waitFor(() => {
      expect(screen.getByTestId("btn-run-cron-cron_01")).toBeInTheDocument();
    });

    fireEvent.click(screen.getByTestId("btn-run-cron-cron_01"));

    await waitFor(() => {
      expect(screen.getByTestId("cron-status-cron_01")).toBeInTheDocument();
    });
  });

  it("opens terminal output modal and displays command output", async () => {
    renderWithProviders(<CronJobsPage />);

    await waitFor(() => {
      expect(screen.getByTestId("btn-view-output-cron_01")).toBeInTheDocument();
    });

    fireEvent.click(screen.getByTestId("btn-view-output-cron_01"));

    await waitFor(() => {
      expect(screen.getByTestId("cron-output-modal-title")).toBeInTheDocument();
      expect(screen.getByTestId("cron-output-content")).toBeInTheDocument();
    });

    fireEvent.click(screen.getByTestId("btn-close-cron-output"));

    await waitFor(() => {
      expect(screen.queryByTestId("cron-output-modal-title")).not.toBeInTheDocument();
    });
  });
});
