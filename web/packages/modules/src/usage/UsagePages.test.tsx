import React from "react";
import { describe, it, expect, vi } from "vitest";
import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import { ToastProvider } from "@builtpanel/ui";
import { ClientUsagePage } from "./ClientUsagePage";
import { AdminUsagePage } from "./AdminUsagePage";

function renderWithProviders(ui: React.ReactElement) {
  return render(<ToastProvider>{ui}</ToastProvider>);
}

describe("Usage Chart UI (T10.10)", () => {
  describe("ClientUsagePage", () => {
    it("renders page header, 4 metric gauge cards, and 4 timeline charts", async () => {
      renderWithProviders(<ClientUsagePage />);

      await waitFor(() => {
        expect(screen.getByTestId("usage-page-title")).toBeInTheDocument();
      });

      expect(screen.getByTestId("card-usage-cpu")).toBeInTheDocument();
      expect(screen.getByTestId("card-usage-ram")).toBeInTheDocument();
      expect(screen.getByTestId("card-usage-disk")).toBeInTheDocument();
      expect(screen.getByTestId("card-usage-bandwidth")).toBeInTheDocument();

      await waitFor(() => {
        expect(screen.getByTestId("metric-current-cpu")).toHaveTextContent(/\d+(\.\d+)?%/);
        expect(screen.getByTestId("metric-current-ram")).toHaveTextContent(/\d+(\.\d+)? MB/);
        expect(screen.getByTestId("metric-current-disk")).toHaveTextContent(/1\.17 GB/);
        expect(screen.getByTestId("metric-current-bandwidth")).toHaveTextContent(/GB/);
        expect(screen.getByTestId("card-chart-cpu")).toBeInTheDocument();
        expect(screen.getByTestId("card-chart-ram")).toBeInTheDocument();
        expect(screen.getByTestId("card-chart-disk")).toBeInTheDocument();
        expect(screen.getByTestId("card-chart-network")).toBeInTheDocument();
      });
    });

    it("switches time ranges between 24h, 7d, and 30d", async () => {
      renderWithProviders(<ClientUsagePage />);

      await waitFor(() => {
        expect(screen.getByTestId("btn-range-7d")).toBeInTheDocument();
      });

      fireEvent.click(screen.getByTestId("btn-range-7d"));

      await waitFor(() => {
        expect(screen.getByTestId("card-chart-cpu")).toBeInTheDocument();
      });

      fireEvent.click(screen.getByTestId("btn-range-30d"));

      await waitFor(() => {
        expect(screen.getByTestId("card-chart-cpu")).toBeInTheDocument();
      });
    });
  });

  describe("AdminUsagePage", () => {
    it("renders node summary gauges, node timeline chart, and top-N tenants", async () => {
      renderWithProviders(<AdminUsagePage />);

      await waitFor(() => {
        expect(screen.getByTestId("admin-usage-title")).toBeInTheDocument();
      });

      expect(screen.getByTestId("select-admin-usage-node")).toBeInTheDocument();

      await waitFor(() => {
        expect(screen.getByTestId("card-admin-node-cpu")).toBeInTheDocument();
        expect(screen.getByTestId("card-admin-node-ram")).toBeInTheDocument();
        expect(screen.getByTestId("card-admin-node-disk")).toBeInTheDocument();
        expect(screen.getByTestId("card-admin-node-network")).toBeInTheDocument();
        expect(screen.getByTestId("metric-admin-node-cpu")).toHaveTextContent(/%/);
        expect(screen.getByTestId("card-admin-history-chart")).toBeInTheDocument();
        expect(screen.getByTestId("card-top-tenants")).toBeInTheDocument();
        expect(screen.getByTestId("row-tenant-t_01")).toBeInTheDocument();
        expect(screen.getByTestId("tenant-name-t_01")).toHaveTextContent("acme-corp");
      });
    });

    it("switches top-N ranking tabs: CPU, RAM, Disk, and Bandwidth", async () => {
      renderWithProviders(<AdminUsagePage />);

      await waitFor(() => {
        expect(screen.getByTestId("tab-top-ram")).toBeInTheDocument();
      });

      // Switch to RAM
      fireEvent.click(screen.getByTestId("tab-top-ram"));
      await waitFor(() => {
        expect(screen.getByTestId("row-tenant-t_03")).toBeInTheDocument();
        expect(screen.getByTestId("tenant-name-t_03")).toHaveTextContent("shopify-clone");
      });

      // Switch to Disk
      fireEvent.click(screen.getByTestId("tab-top-disk"));
      await waitFor(() => {
        expect(screen.getByTestId("row-tenant-t_06")).toBeInTheDocument();
        expect(screen.getByTestId("tenant-name-t_06")).toHaveTextContent("video-stream");
      });

      // Switch to Bandwidth
      fireEvent.click(screen.getByTestId("tab-top-bandwidth"));
      await waitFor(() => {
        expect(screen.getByTestId("row-tenant-t_06")).toBeInTheDocument();
      });
    });
  });
});
