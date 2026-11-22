import React from "react";
import { describe, it, expect, beforeEach } from "vitest";
import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import { ToastProvider } from "@builtpanel/ui";
import { resetMockVhost } from "@builtpanel/api";
import { VhostTemplatesPage } from "./VhostTemplatesPage";

function renderWithProviders(ui: React.ReactElement) {
  return render(<ToastProvider>{ui}</ToastProvider>);
}

describe("VhostTemplatesPage (T06.07)", () => {
  beforeEach(() => {
    resetMockVhost();
  });

  it("renders page title, available templates list, and initial code editor", async () => {
    renderWithProviders(<VhostTemplatesPage />);

    expect(screen.getByRole("heading", { level: 1 })).toBeInTheDocument();

    // Verify seeded templates are loaded in the sidebar
    await waitFor(() => {
      expect(screen.getByTestId("card-vhost-template-nginx-php-fpm-standard")).toBeInTheDocument();
    });

    expect(screen.getByTestId("card-vhost-template-nginx-static-site")).toBeInTheDocument();
    expect(screen.getByTestId("card-vhost-template-caddy-reverse-proxy")).toBeInTheDocument();

    // Verify initial editor content
    const textarea = screen.getByTestId("textarea-vhost-content") as HTMLTextAreaElement;
    expect(textarea).toBeInTheDocument();
    expect(textarea.value).toContain("fastcgi_pass unix:{{php_socket}}");
  });

  it("switches between templates and loads corresponding configuration", async () => {
    renderWithProviders(<VhostTemplatesPage />);

    await waitFor(() => {
      expect(screen.getByTestId("card-vhost-template-caddy-reverse-proxy")).toBeInTheDocument();
    });

    // Click Caddy template card
    fireEvent.click(screen.getByTestId("card-vhost-template-caddy-reverse-proxy"));

    const textarea = screen.getByTestId("textarea-vhost-content") as HTMLTextAreaElement;
    await waitFor(() => {
      expect(textarea.value).toContain("reverse_proxy 127.0.0.1:{{port}}");
    });
  });

  it("switches to Live Preview tab and interpolates sample parameters", async () => {
    renderWithProviders(<VhostTemplatesPage />);

    await waitFor(() => {
      expect(screen.getByTestId("card-vhost-template-nginx-php-fpm-standard")).toBeInTheDocument();
    });

    // Switch to Preview tab
    const previewTab = screen.getByTestId("tab-preview");
    fireEvent.click(previewTab);

    // Verify preview renders interpolated variables instead of raw tokens
    await waitFor(() => {
      const renderedPre = screen.getByTestId("pre-vhost-rendered");
      expect(renderedPre).toHaveTextContent("server_name my-awesome-site.com www.my-awesome-site.com;");
      expect(renderedPre).toHaveTextContent("root /home/demo/public_html;");
      expect(renderedPre).toHaveTextContent("fastcgi_pass unix:/run/php/php8.3-fpm-demo.sock;");
    });
  });

  it("detects syntax errors and forbidden directives in code editor", async () => {
    renderWithProviders(<VhostTemplatesPage />);

    await waitFor(() => {
      expect(screen.getByTestId("textarea-vhost-content")).toBeInTheDocument();
    });

    const textarea = screen.getByTestId("textarea-vhost-content");

    // Insert mismatched brace
    fireEvent.change(textarea, {
      target: { value: "server { listen 80; server_name test.com; " },
    });

    await waitFor(() => {
      expect(screen.getByTestId("vhost-validation-errors")).toBeInTheDocument();
      expect(screen.getByText(/Mismatched curly braces/i)).toBeInTheDocument();
    });

    // Insert forbidden directive (T06.06 security rule)
    fireEvent.change(textarea, {
      target: { value: "server { alias /etc/passwd; }" },
    });

    await waitFor(() => {
      expect(screen.getByText(/Forbidden directive: 'alias \/etc'/i)).toBeInTheDocument();
    });
  });

  it("opens version history dialog and restores an older revision", async () => {
    renderWithProviders(<VhostTemplatesPage />);

    await waitFor(() => {
      expect(screen.getByTestId("btn-vhost-history")).toBeInTheDocument();
    });

    // Open History Dialog
    fireEvent.click(screen.getByTestId("btn-vhost-history"));

    await waitFor(() => {
      expect(screen.getByTestId("dialog-vhost-history")).toBeInTheDocument();
      expect(screen.getByTestId("version-item-1")).toBeInTheDocument();
    });

    // Select Version 1 item
    fireEvent.click(screen.getByTestId("version-item-1"));

    // Click Restore button
    const restoreBtn = screen.getByTestId("btn-revert-version");
    fireEvent.click(restoreBtn);

    // Dialog closes and editor content updates
    await waitFor(() => {
      expect(screen.queryByTestId("dialog-vhost-history")).not.toBeInTheDocument();
    });
  });

  it("DoD: executes 'Apply to all domains' background operation", async () => {
    renderWithProviders(<VhostTemplatesPage />);

    await waitFor(() => {
      expect(screen.getByTestId("btn-apply-all-domains")).toBeInTheDocument();
    });

    // Open Apply All dialog
    fireEvent.click(screen.getByTestId("btn-apply-all-domains"));

    await waitFor(() => {
      expect(screen.getByTestId("dialog-vhost-apply-all")).toBeInTheDocument();
    });

    // Confirm deploy
    const confirmBtn = screen.getByTestId("btn-confirm-apply-all");
    fireEvent.click(confirmBtn);

    // Operation tracker triggers and completes
    await waitFor(() => {
      expect(screen.getByText(/Deployment Succeeded!/i)).toBeInTheDocument();
    });
  });
});
