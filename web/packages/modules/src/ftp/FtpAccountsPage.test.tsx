import React from "react";
import { describe, it, expect, vi } from "vitest";
import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import { ToastProvider } from "@builtpanel/ui";
import { FtpAccountsPage } from "./FtpAccountsPage";

function renderWithProviders(ui: React.ReactElement) {
  return render(<ToastProvider>{ui}</ToastProvider>);
}

describe("FtpAccountsPage (T08.13)", () => {
  it("renders page title, connection info card, and seed accounts", async () => {
    renderWithProviders(<FtpAccountsPage />);

    await waitFor(() => {
      expect(screen.getByTestId("ftp-page-title")).toBeInTheDocument();
    });

    // Check connection settings card
    expect(screen.getByTestId("ftp-connection-card")).toBeInTheDocument();
    expect(screen.getByTestId("ftp-info-host")).toHaveTextContent("ftp.demo.builtpanel.test");
    expect(screen.getByTestId("ftp-info-port")).toHaveTextContent("21");
    expect(screen.getByTestId("ftp-info-protocol")).toHaveTextContent("FTP");
    expect(screen.getByTestId("ftp-info-encryption")).toHaveTextContent("FTPS (explicit TLS)");

    // Check seed accounts
    await waitFor(() => {
      expect(screen.getByTestId("ftp-username-demo_deploy")).toBeInTheDocument();
      expect(screen.getByTestId("ftp-username-demo_backup")).toBeInTheDocument();
    });
  });

  it("filters accounts table using search query", async () => {
    renderWithProviders(<FtpAccountsPage />);

    await waitFor(() => {
      expect(screen.getByTestId("ftp-username-demo_deploy")).toBeInTheDocument();
    });

    const searchInput = screen.getByTestId("input-search-ftp");
    fireEvent.change(searchInput, { target: { value: "backup" } });

    expect(screen.getByTestId("ftp-username-demo_backup")).toBeInTheDocument();
    expect(screen.queryByTestId("ftp-username-demo_deploy")).not.toBeInTheDocument();
  });

  it("creates a new FTP account with prefix enforcement", async () => {
    renderWithProviders(<FtpAccountsPage />);

    await waitFor(() => {
      expect(screen.getByTestId("btn-open-create-ftp")).toBeInTheDocument();
    });

    fireEvent.click(screen.getByTestId("btn-open-create-ftp"));

    await waitFor(() => {
      expect(screen.getByTestId("create-ftp-dialog-title")).toBeInTheDocument();
    });

    // Fill form
    const usernameInput = screen.getByTestId("input-ftp-username");
    fireEvent.change(usernameInput, { target: { value: "worker" } });

    // Generate password
    fireEvent.click(screen.getByTestId("btn-generate-ftp-pw"));

    // Submit
    fireEvent.click(screen.getByTestId("btn-submit-create-ftp"));

    await waitFor(() => {
      expect(screen.getByTestId("ftp-username-demo_worker")).toBeInTheDocument();
    });
  });

  it("opens change password dialog and updates password", async () => {
    renderWithProviders(<FtpAccountsPage />);

    await waitFor(() => {
      expect(screen.getByTestId("btn-change-pw-demo_deploy")).toBeInTheDocument();
    });

    fireEvent.click(screen.getByTestId("btn-change-pw-demo_deploy"));

    await waitFor(() => {
      expect(screen.getByTestId("change-ftp-password-title")).toBeInTheDocument();
      expect(screen.getByTestId("ftp-pw-target-user")).toHaveTextContent("demo_deploy");
    });

    // Fill new password
    fireEvent.change(screen.getByTestId("input-new-ftp-password"), {
      target: { value: "SuperSecretNewPass123!" },
    });

    fireEvent.click(screen.getByTestId("btn-submit-change-ftp-pw"));

    await waitFor(() => {
      expect(screen.queryByTestId("change-ftp-password-title")).not.toBeInTheDocument();
    });
  });

  it("opens change quota dialog and updates quota", async () => {
    renderWithProviders(<FtpAccountsPage />);

    await waitFor(() => {
      expect(screen.getByTestId("btn-change-quota-demo_deploy")).toBeInTheDocument();
    });

    fireEvent.click(screen.getByTestId("btn-change-quota-demo_deploy"));

    await waitFor(() => {
      expect(screen.getByTestId("change-ftp-quota-title")).toBeInTheDocument();
    });

    // Switch to custom quota
    fireEvent.click(screen.getByTestId("edit-radio-quota-limited"));

    const quotaInput = screen.getByTestId("edit-input-ftp-quota-mb");
    fireEvent.change(quotaInput, { target: { value: "2048" } });

    fireEvent.click(screen.getByTestId("btn-submit-change-quota"));

    await waitFor(() => {
      expect(screen.queryByTestId("change-ftp-quota-title")).not.toBeInTheDocument();
    });
  });

  it("deletes an FTP account after confirmation", async () => {
    const confirmSpy = vi.spyOn(window, "confirm").mockImplementation(() => true);

    renderWithProviders(<FtpAccountsPage />);

    await waitFor(() => {
      expect(screen.getByTestId("btn-delete-ftp-demo_backup")).toBeInTheDocument();
    });

    fireEvent.click(screen.getByTestId("btn-delete-ftp-demo_backup"));

    await waitFor(() => {
      expect(screen.queryByTestId("ftp-username-demo_backup")).not.toBeInTheDocument();
    });

    confirmSpy.mockRestore();
  });
});
