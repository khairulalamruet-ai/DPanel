import React from "react";
import { describe, it, expect, beforeEach } from "vitest";
import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import { ToastProvider } from "@builtpanel/ui";
import { resetMockPhp } from "@builtpanel/api";
import { PhpManagementPage } from "./PhpManagementPage";

function renderWithProviders(ui: React.ReactElement) {
  return render(<ToastProvider>{ui}</ToastProvider>);
}

describe("PhpManagementPage (T06.11)", () => {
  beforeEach(() => {
    resetMockPhp();
  });

  it("renders page title, metric summary cards, and default version selector", async () => {
    renderWithProviders(<PhpManagementPage />);

    expect(screen.getByRole("heading", { level: 1 })).toBeInTheDocument();

    await waitFor(() => {
      expect(screen.getByTestId("php-version-selector")).toBeInTheDocument();
    });

    // Check metric cards
    expect(screen.getByTestId("metric-php-default-version")).toHaveTextContent("PHP 8.3");
    expect(screen.getByTestId("metric-php-memory-limit")).toHaveTextContent("256M");
    expect(screen.getByTestId("metric-php-upload")).toHaveTextContent("64M");

    // Check domains listed
    expect(screen.getByText("alpha-hosting.com")).toBeInTheDocument();
    expect(screen.getByText("blog.alpha-hosting.com")).toBeInTheDocument();
  });

  it("changes account default PHP version via dropdown selector", async () => {
    renderWithProviders(<PhpManagementPage />);

    await waitFor(() => {
      expect(screen.getByTestId("select-default-php-version")).toBeInTheDocument();
    });

    const select = screen.getByTestId("select-default-php-version");
    fireEvent.change(select, { target: { value: "8.4" } });

    const updateBtn = screen.getByTestId("btn-update-default-php");
    expect(updateBtn).not.toBeDisabled();
    fireEvent.click(updateBtn);

    await waitFor(() => {
      expect(screen.getByTestId("metric-php-default-version")).toHaveTextContent("PHP 8.4");
    });
  });

  it("updates PHP version override for a specific domain", async () => {
    renderWithProviders(<PhpManagementPage />);

    await waitFor(() => {
      expect(screen.getByTestId("select-domain-php-dom_01")).toBeInTheDocument();
    });

    const domainSelect = screen.getByTestId("select-domain-php-dom_01");
    fireEvent.change(domainSelect, { target: { value: "8.2" } });

    await waitFor(() => {
      expect(screen.getByTestId("row-domain-php-dom_01")).toHaveTextContent("PHP 8.2");
    });
  });

  it("switches to php.ini tab and displays form inputs with plan limits", async () => {
    renderWithProviders(<PhpManagementPage />);

    await waitFor(() => {
      expect(screen.getByTestId("tab-php-ini")).toBeInTheDocument();
    });

    // Click php.ini tab
    fireEvent.click(screen.getByTestId("tab-php-ini"));

    await waitFor(() => {
      expect(screen.getByTestId("php-ini-form-card")).toBeInTheDocument();
      expect(screen.getByTestId("input-memory-limit")).toHaveValue("256M");
      expect(screen.getByTestId("input-max-execution-time")).toHaveValue(60);
      expect(screen.getByTestId("input-upload-max-filesize")).toHaveValue("64M");
    });
  });

  it("edits php.ini settings and saves changes", async () => {
    renderWithProviders(<PhpManagementPage />);

    await waitFor(() => {
      expect(screen.getByTestId("tab-php-ini")).toBeInTheDocument();
    });

    fireEvent.click(screen.getByTestId("tab-php-ini"));

    await waitFor(() => {
      expect(screen.getByTestId("input-memory-limit")).toBeInTheDocument();
    });

    // Change memory limit to 512M
    const memInput = screen.getByTestId("input-memory-limit");
    fireEvent.change(memInput, { target: { value: "512M" } });

    const saveBtn = screen.getByTestId("btn-save-php-ini");
    fireEvent.click(saveBtn);

    await waitFor(() => {
      expect(memInput).toHaveValue("512M");
    });
  });

  it("DoD: resets php.ini directives to hosting plan defaults", async () => {
    renderWithProviders(<PhpManagementPage />);

    await waitFor(() => {
      expect(screen.getByTestId("tab-php-ini")).toBeInTheDocument();
    });

    fireEvent.click(screen.getByTestId("tab-php-ini"));

    await waitFor(() => {
      expect(screen.getByTestId("input-memory-limit")).toBeInTheDocument();
    });

    // First change input to custom value
    const memInput = screen.getByTestId("input-memory-limit");
    fireEvent.change(memInput, { target: { value: "384M" } });
    expect(memInput).toHaveValue("384M");

    // Click Reset to Defaults button
    const resetBtn = screen.getByTestId("btn-reset-php-ini");
    fireEvent.click(resetBtn);

    // Form reverts to initial plan default (256M)
    await waitFor(() => {
      expect(memInput).toHaveValue("256M");
    });
  });
});
