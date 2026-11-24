import React from "react";
import { describe, it, expect, beforeEach } from "vitest";
import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import { ToastProvider } from "@builtpanel/ui";
import { resetMockSsl } from "@builtpanel/api";
import { SslPage } from "./SslPage";

function renderWithProviders(ui: React.ReactElement) {
  return render(<ToastProvider>{ui}</ToastProvider>);
}

describe("SslPage (T05.19)", () => {
  beforeEach(() => {
    resetMockSsl();
  });

  it("renders page title, metric cards, and initial seeded domain certificates", async () => {
    renderWithProviders(<SslPage />);

    expect(screen.getByRole("heading", { level: 1 })).toBeInTheDocument();

    // Verify initial seeded certificates loaded
    await waitFor(() => {
      expect(screen.getByRole("heading", { level: 3, name: "alpha-hosting.com" })).toBeInTheDocument();
    });

    expect(screen.getByRole("heading", { level: 3, name: "blog.alpha-hosting.com" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { level: 3, name: "shop.alpha-hosting.com" })).toBeInTheDocument();

    // Check metric cards
    expect(screen.getByText("3")).toBeInTheDocument(); // total
  });

  it("filters certificates by status pill and search keyword", async () => {
    renderWithProviders(<SslPage />);

    await waitFor(() => {
      expect(screen.getByRole("heading", { level: 3, name: "alpha-hosting.com" })).toBeInTheDocument();
    });

    // Filter by expired
    const expiredFilterBtn = screen.getByTestId("btn-filter-ssl-expired");
    fireEvent.click(expiredFilterBtn);

    expect(screen.getByRole("heading", { level: 3, name: "shop.alpha-hosting.com" })).toBeInTheDocument();
    expect(screen.queryByRole("heading", { level: 3, name: "alpha-hosting.com" })).not.toBeInTheDocument();

    // Reset to all
    const allFilterBtn = screen.getByTestId("btn-filter-ssl-ALL");
    fireEvent.click(allFilterBtn);

    // Search by keyword
    const searchInput = screen.getByTestId("input-search-ssl");
    fireEvent.change(searchInput, { target: { value: "blog" } });

    expect(screen.getByRole("heading", { level: 3, name: "blog.alpha-hosting.com" })).toBeInTheDocument();
    expect(screen.queryByRole("heading", { level: 3, name: "shop.alpha-hosting.com" })).not.toBeInTheDocument();
  });

  it("reads the certificate state again (the web server renews certificates itself)", async () => {
    renderWithProviders(<SslPage />);

    await waitFor(() => {
      expect(screen.getByRole("heading", { level: 3, name: "blog.alpha-hosting.com" })).toBeInTheDocument();
    });

    fireEvent.click(screen.getByTestId("btn-reissue-ssl-dom_02"));

    await waitFor(() => {
      expect(screen.getByText(/read again|আবার পড়া/)).toBeInTheDocument();
    });
  });

  it("uploads and installs custom commercial SSL certificate (DoD)", async () => {
    renderWithProviders(<SslPage />);

    await waitFor(() => {
      expect(screen.getByRole("heading", { level: 3, name: "shop.alpha-hosting.com" })).toBeInTheDocument();
    });

    // Open Custom Cert dialog for shop.alpha-hosting.com (dom_03)
    const customBtn = screen.getByTestId("btn-upload-custom-ssl-dom_03");
    fireEvent.click(customBtn);

    await waitFor(() => {
      expect(screen.getByTestId("dialog-custom-ssl")).toBeInTheDocument();
    });

    // Enter PEM Cert and Private Key
    const certInput = screen.getByTestId("textarea-ssl-cert-pem");
    const keyInput = screen.getByTestId("textarea-ssl-key-pem");

    fireEvent.change(certInput, {
      target: {
        value: `-----BEGIN CERTIFICATE-----
MIIDXTCCAkWgAwIBAgIJAP999EXAMPLE
-----END CERTIFICATE-----`,
      },
    });

    fireEvent.change(keyInput, {
      target: {
        value: `-----BEGIN RSA PRIVATE KEY-----
MIIEowIBAAKCAQEAz999EXAMPLE
-----END RSA PRIVATE KEY-----`,
      },
    });

    // Submit form
    const submitBtn = screen.getByTestId("btn-submit-custom-ssl");
    fireEvent.click(submitBtn);

    // Modal closes and status changes to active with custom issuer
    await waitFor(() => {
      expect(screen.queryByTestId("dialog-custom-ssl")).not.toBeInTheDocument();
    });

    await waitFor(() => {
      expect(screen.getByText("Custom CA")).toBeInTheDocument();
    });
  });

  it("configures HTTPS redirection and HSTS security headers", async () => {
    renderWithProviders(<SslPage />);

    await waitFor(() => {
      expect(screen.getByRole("heading", { level: 3, name: "alpha-hosting.com" })).toBeInTheDocument();
    });

    // Open settings dialog for alpha-hosting.com (dom_01)
    const settingsBtn = screen.getByTestId("btn-settings-ssl-dom_01");
    fireEvent.click(settingsBtn);

    await waitFor(() => {
      expect(screen.getByTestId("dialog-ssl-settings")).toBeInTheDocument();
    });

    // Check checkboxes
    const forceHttpsCheckbox = screen.getByTestId("checkbox-force-https");
    const hstsCheckbox = screen.getByTestId("checkbox-hsts");

    fireEvent.click(forceHttpsCheckbox);
    fireEvent.click(hstsCheckbox);

    // Save settings
    const saveBtn = screen.getByTestId("btn-save-ssl-settings");
    fireEvent.click(saveBtn);

    await waitFor(() => {
      expect(screen.queryByTestId("dialog-ssl-settings")).not.toBeInTheDocument();
    });
  });

  it("goes back from an uploaded certificate to the web server's own one", async () => {
    renderWithProviders(<SslPage />);

    await waitFor(() => {
      expect(screen.getByRole("heading", { level: 3, name: "blog.alpha-hosting.com" })).toBeInTheDocument();
    });

    // Only an uploaded certificate (dom_02) can be taken away; the others are the web server's own.
    expect(screen.queryByTestId("btn-revoke-ssl-dom_01")).not.toBeInTheDocument();
    fireEvent.click(screen.getByTestId("btn-revoke-ssl-dom_02"));
    fireEvent.click(screen.getByTestId("btn-confirm-revoke-ssl"));

    await waitFor(() => {
      expect(screen.queryByTestId("btn-revoke-ssl-dom_02")).not.toBeInTheDocument();
    });
  });
});
