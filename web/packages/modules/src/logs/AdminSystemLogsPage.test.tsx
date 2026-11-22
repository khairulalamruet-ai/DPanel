import React from "react";
import { describe, it, expect, vi } from "vitest";
import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import { ToastProvider } from "@builtpanel/ui";
import { AdminSystemLogsPage } from "./AdminSystemLogsPage";

function renderWithProviders(ui: React.ReactElement) {
  return render(<ToastProvider>{ui}</ToastProvider>);
}

describe("AdminSystemLogsPage (T10.07)", () => {
  it("renders page header, service tabs, node selector, and virtualized system log entries", async () => {
    renderWithProviders(<AdminSystemLogsPage />);

    await waitFor(() => {
      expect(screen.getByTestId("admin-system-logs-title")).toBeInTheDocument();
    });

    expect(screen.getByTestId("tab-service-control-plane")).toBeInTheDocument();
    expect(screen.getByTestId("select-admin-log-node")).toBeInTheDocument();
    expect(screen.getByTestId("select-admin-log-level")).toBeInTheDocument();
    expect(screen.getByTestId("btn-download-admin-log")).toBeInTheDocument();
    expect(screen.getByTestId("virtualized-log-container")).toBeInTheDocument();

    await waitFor(() => {
      expect(screen.getByTestId("virtualized-log-line-0")).toBeInTheDocument();
      expect(screen.getByTestId("metric-admin-lines")).toBeInTheDocument();
    });
  });

  it("switches between the control plane and the core services of a node", async () => {
    renderWithProviders(<AdminSystemLogsPage />);

    await waitFor(() => {
      expect(screen.getByTestId("tab-service-database")).toBeInTheDocument();
    });

    fireEvent.click(screen.getByTestId("tab-service-database"));

    await waitFor(() => {
      expect(screen.getByTestId("badge-service-active")).toHaveTextContent("database");
      expect(screen.getByTestId("metric-admin-lines")).toHaveTextContent("40");
    });

    fireEvent.click(screen.getByTestId("tab-service-control-plane"));

    await waitFor(() => {
      expect(screen.getByTestId("badge-service-active")).toHaveTextContent("control-plane");
    });
  });

  it("filters system logs by log level", async () => {
    renderWithProviders(<AdminSystemLogsPage />);

    await waitFor(() => {
      expect(screen.getByTestId("select-admin-log-level")).toBeInTheDocument();
    });

    fireEvent.change(screen.getByTestId("select-admin-log-level"), {
      target: { value: "error" },
    });

    await waitFor(() => {
      expect(screen.getByTestId("virtualized-log-container")).toBeInTheDocument();
    });
  });

  it("toggles Live Tail mode on admin system stream", async () => {
    renderWithProviders(<AdminSystemLogsPage />);

    await waitFor(() => {
      expect(screen.getByTestId("btn-toggle-admin-live-mode")).toBeInTheDocument();
    });

    fireEvent.click(screen.getByTestId("btn-toggle-admin-live-mode"));

    await waitFor(() => {
      expect(screen.getByTestId("badge-admin-live-status")).toHaveTextContent(
        /System Stream Active|সিস্টেম স্ট্রিম সক্রিয়/i
      );
    });

    // Toggle off
    fireEvent.click(screen.getByTestId("btn-toggle-admin-live-mode"));

    await waitFor(() => {
      expect(screen.getByTestId("badge-admin-live-status")).toHaveTextContent(
        /Stream Paused|স্ট্রিম স্থগিত/i
      );
    });
  });
});
