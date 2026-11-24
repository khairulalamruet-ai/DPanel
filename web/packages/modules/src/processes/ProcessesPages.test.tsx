import React from "react";
import { describe, it, expect, vi } from "vitest";
import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import { ToastProvider } from "@builtpanel/ui";
import { ClientProcessesPage } from "./ClientProcessesPage";
import { AdminProcessesPage } from "./AdminProcessesPage";

function renderWithProviders(ui: React.ReactElement) {
  return render(<ToastProvider>{ui}</ToastProvider>);
}

describe("Process Manager UI (T10.14)", () => {
  describe("ClientProcessesPage", () => {
    it("renders page header, metric cards, and isolated container processes (DoD: Client অন্যের process দেখতে পান না)", async () => {
      renderWithProviders(<ClientProcessesPage />);

      await waitFor(() => {
        expect(screen.getByTestId("processes-page-title")).toBeInTheDocument();
      });

      expect(screen.getByTestId("card-proc-total")).toBeInTheDocument();
      expect(screen.getByTestId("card-proc-cpu")).toBeInTheDocument();
      expect(screen.getByTestId("card-proc-ram")).toBeInTheDocument();

      await waitFor(() => {
        expect(screen.getByTestId("metric-total-processes")).toHaveTextContent("6");
        expect(screen.getByTestId("process-row-14")).toBeInTheDocument();
        expect(screen.getByTestId("proc-cmd-14")).toHaveTextContent("php-fpm: pool www");
      });

      // DoD Requirement: Client never sees host dockerd or other tenant processes (e.g. acme)
      expect(screen.queryByText(/dockerd/i)).not.toBeInTheDocument();
      expect(screen.queryByText(/acme/i)).not.toBeInTheDocument();
    });

    it("filters processes via search input", async () => {
      renderWithProviders(<ClientProcessesPage />);

      await waitFor(() => {
        expect(screen.getByTestId("process-row-14")).toBeInTheDocument();
      });

      const searchInput = screen.getByTestId("input-search-processes");
      fireEvent.change(searchInput, { target: { value: "python3" } });

      expect(screen.getByTestId("process-row-142")).toBeInTheDocument();
      expect(screen.queryByTestId("process-row-14")).not.toBeInTheDocument();
    });

    it("opens kill confirmation dialog and sends graceful SIGTERM", async () => {
      renderWithProviders(<ClientProcessesPage />);

      await waitFor(() => {
        expect(screen.getByTestId("btn-sigterm-14")).toBeInTheDocument();
      });

      fireEvent.click(screen.getByTestId("btn-sigterm-14"));

      await waitFor(() => {
        expect(screen.getByTestId("kill-process-dialog-title")).toBeInTheDocument();
        expect(screen.getByTestId("radio-signal-sigterm")).toBeInTheDocument();
      });

      fireEvent.click(screen.getByTestId("btn-confirm-kill-process"));

      await waitFor(() => {
        expect(screen.queryByTestId("kill-process-dialog-title")).not.toBeInTheDocument();
      });
    });

    it("opens kill confirmation dialog and sends force SIGKILL", async () => {
      renderWithProviders(<ClientProcessesPage />);

      await waitFor(() => {
        expect(screen.getByTestId("btn-sigkill-15")).toBeInTheDocument();
      });

      fireEvent.click(screen.getByTestId("btn-sigkill-15"));

      await waitFor(() => {
        expect(screen.getByTestId("kill-process-dialog-title")).toBeInTheDocument();
        expect(screen.getByTestId("radio-signal-sigkill")).toBeInTheDocument();
      });

      fireEvent.click(screen.getByTestId("btn-confirm-kill-process"));

      await waitFor(() => {
        expect(screen.queryByTestId("kill-process-dialog-title")).not.toBeInTheDocument();
      });
    });
  });

  describe("AdminProcessesPage", () => {
    it("renders host & cluster processes with sudo privileges and tenant scopes", async () => {
      renderWithProviders(<AdminProcessesPage />);

      await waitFor(() => {
        expect(screen.getByTestId("admin-processes-title")).toBeInTheDocument();
      });

      expect(screen.getByTestId("select-admin-proc-node")).toBeInTheDocument();
      expect(screen.getByTestId("select-admin-proc-tenant")).toBeInTheDocument();

      await waitFor(() => {
        expect(screen.getByTestId("admin-process-row-412")).toBeInTheDocument();
        expect(screen.getByTestId("admin-proc-cmd-412")).toHaveTextContent("dockerd");
        expect(screen.getByTestId("admin-process-row-2200")).toBeInTheDocument();
      });
    });

    it("filters by tenant in admin process manager", async () => {
      renderWithProviders(<AdminProcessesPage />);

      await waitFor(() => {
        expect(screen.getByTestId("select-admin-proc-tenant")).toBeInTheDocument();
      });

      fireEvent.change(screen.getByTestId("select-admin-proc-tenant"), {
        target: { value: "acme" },
      });

      await waitFor(() => {
        expect(screen.getByTestId("admin-process-row-2200")).toBeInTheDocument();
        expect(screen.queryByTestId("admin-process-row-412")).not.toBeInTheDocument();
      });
    });
  });
});
