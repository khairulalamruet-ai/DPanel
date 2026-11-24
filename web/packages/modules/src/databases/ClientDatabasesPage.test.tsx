import React from "react";
import { describe, it, expect, beforeEach } from "vitest";
import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import { ToastProvider } from "@builtpanel/ui";
import { resetMockDatabases } from "@builtpanel/api";
import { ClientDatabasesPage } from "./ClientDatabasesPage";

function renderWithProviders(ui: React.ReactElement) {
  return render(<ToastProvider>{ui}</ToastProvider>);
}

describe("ClientDatabasesPage (T07.10)", () => {
  beforeEach(() => {
    resetMockDatabases();
  });

  it("renders page title, metrics, and initial database list", async () => {
    renderWithProviders(<ClientDatabasesPage />);

    await waitFor(() => {
      expect(screen.getByTestId("databases-title")).toBeInTheDocument();
    });

    expect(screen.getByTestId("metric-db-count")).toHaveTextContent("2");
    expect(screen.getByTestId("metric-user-count")).toHaveTextContent("2");
    expect(screen.getByTestId("metric-db-size")).toHaveTextContent("42.5 MB");

    // Verify seed databases
    expect(screen.getByTestId("db-row-demo_wordpress")).toBeInTheDocument();
    expect(screen.getByTestId("db-row-demo_analytics")).toBeInTheDocument();
  });

  it("switches to Database Users tab and displays existing users", async () => {
    renderWithProviders(<ClientDatabasesPage />);

    await waitFor(() => {
      expect(screen.getByTestId("subtab-users")).toBeInTheDocument();
    });

    fireEvent.click(screen.getByTestId("subtab-users"));

    await waitFor(() => {
      expect(screen.getByTestId("user-row-demo_wpuser")).toBeInTheDocument();
      expect(screen.getByTestId("user-row-demo_analyst")).toBeInTheDocument();
    });
  });

  it("creates a new database with prefix enforcement", async () => {
    renderWithProviders(<ClientDatabasesPage />);

    await waitFor(() => {
      expect(screen.getByTestId("btn-open-create-db")).toBeInTheDocument();
    });

    fireEvent.click(screen.getByTestId("btn-open-create-db"));

    await waitFor(() => {
      expect(screen.getByTestId("input-db-name")).toBeInTheDocument();
    });

    // Enter raw database name
    const input = screen.getByTestId("input-db-name");
    fireEvent.change(input, { target: { value: "shop" } });

    // Click submit
    fireEvent.click(screen.getByTestId("btn-submit-create-db"));

    // Toast notification and new row appears
    await waitFor(() => {
      expect(screen.getByText(/created successfully|সফলভাবে তৈরি হয়েছে/i)).toBeInTheDocument();
      expect(screen.getByTestId("db-row-demo_shop")).toBeInTheDocument();
    });
  });

  it("creates a new database user with password generator", async () => {
    renderWithProviders(<ClientDatabasesPage />);

    await waitFor(() => {
      expect(screen.getByTestId("subtab-users")).toBeInTheDocument();
    });

    fireEvent.click(screen.getByTestId("subtab-users"));

    await waitFor(() => {
      expect(screen.getByTestId("btn-open-create-user")).toBeInTheDocument();
    });

    fireEvent.click(screen.getByTestId("btn-open-create-user"));

    await waitFor(() => {
      expect(screen.getByTestId("input-db-username")).toBeInTheDocument();
    });

    fireEvent.change(screen.getByTestId("input-db-username"), { target: { value: "shopusr" } });

    // Click generate password
    fireEvent.click(screen.getByTestId("btn-generate-db-password"));

    const passInput = screen.getByTestId("input-db-password") as HTMLInputElement;
    expect(passInput.value.length).toBeGreaterThanOrEqual(10);

    // Click submit
    fireEvent.click(screen.getByTestId("btn-submit-create-user"));

    await waitFor(() => {
      expect(screen.getByText(/created successfully|সফলভাবে তৈরি হয়েছে/i)).toBeInTheDocument();
      expect(screen.getByTestId("user-row-demo_shopusr")).toBeInTheDocument();
    });
  });

  it("updates user privileges matrix using presets", async () => {
    renderWithProviders(<ClientDatabasesPage />);

    await waitFor(() => {
      expect(screen.getByTestId("subtab-users")).toBeInTheDocument();
    });

    fireEvent.click(screen.getByTestId("subtab-users"));

    await waitFor(() => {
      expect(screen.getByTestId("btn-manage-grants-dbusr_02")).toBeInTheDocument();
    });

    fireEvent.click(screen.getByTestId("btn-manage-grants-dbusr_02"));

    await waitFor(() => {
      expect(screen.getByTestId("preset-all")).toBeInTheDocument();
    });

    // Apply All Privileges preset
    fireEvent.click(screen.getByTestId("preset-all"));

    // Click save
    fireEvent.click(screen.getByTestId("btn-save-grants"));

    await waitFor(() => {
      expect(screen.getByText(/Privileges updated|অনুমতি আপডেট করা হয়েছে/i)).toBeInTheDocument();
    });
  });

  it("configures remote database CIDR access allowlist", async () => {
    renderWithProviders(<ClientDatabasesPage />);

    await waitFor(() => {
      expect(screen.getByTestId("btn-remote-access")).toBeInTheDocument();
    });

    fireEvent.click(screen.getByTestId("btn-remote-access"));

    await waitFor(() => {
      expect(screen.getByTestId("input-remote-ip")).toBeInTheDocument();
    });

    // Add new IP
    fireEvent.change(screen.getByTestId("input-remote-ip"), {
      target: { value: "203.0.113.195" },
    });
    fireEvent.click(screen.getByTestId("btn-add-remote-ip"));

    // Save
    fireEvent.click(screen.getByTestId("btn-save-remote-hosts"));

    await waitFor(() => {
      expect(screen.getByText(/allowlist updated|তালিকা আপডেট হয়েছে/i)).toBeInTheDocument();
    });
  });

  it("deletes a database via confirmation dialog", async () => {
    renderWithProviders(<ClientDatabasesPage />);

    await waitFor(() => {
      expect(screen.getByTestId("btn-delete-db-db_02")).toBeInTheDocument();
    });

    fireEvent.click(screen.getByTestId("btn-delete-db-db_02"));

    await waitFor(() => {
      expect(screen.getByTestId("btn-confirm-delete-db")).toBeInTheDocument();
    });

    fireEvent.click(screen.getByTestId("btn-confirm-delete-db"));

    await waitFor(() => {
      expect(screen.getByText(/deleted|মুছে ফেলা হয়েছে/i)).toBeInTheDocument();
    });

    expect(screen.queryByTestId("db-row-demo_analytics")).not.toBeInTheDocument();
  });
});
