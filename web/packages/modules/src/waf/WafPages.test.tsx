import React from "react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { ToastProvider } from "@builtpanel/ui";
import { ClientWafPage } from "./ClientWafPage";
import { AdminWafPage } from "./AdminWafPage";

const mockApiFetch = vi.fn();
vi.mock("@builtpanel/api", () => ({
  apiFetch: (...args: any[]) => mockApiFetch(...args),
}));

function renderWithToast(ui: React.ReactElement) {
  return render(<ToastProvider>{ui}</ToastProvider>);
}

describe("Web application firewall policy", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("ClientWafPage: shows each domain's mode and switches rules off and on", async () => {
    let own: any = null;
    mockApiFetch.mockImplementation((url: string, opts?: any) => {
      if (url === "/api/v1/client/waf/domains") {
        return Promise.resolve({
          items: [
            {
              domain_id: "dom_01",
              fqdn: "alpha-hosting.com",
              effective: own ?? { mode: "detect", mode_from: "default", disabled_rules: [] },
              policy: own ? {} : null,
            },
          ],
        });
      }
      if (url === "/api/v1/client/waf/domains/dom_01" && opts?.method === "PUT") {
        const body = JSON.parse(opts.body);
        own = { mode: body.mode, mode_from: "domain", disabled_rules: body.disabled_rules };
        return Promise.resolve({ effective: own, policy: null });
      }
      return Promise.resolve({});
    });

    renderWithToast(<ClientWafPage />);

    await waitFor(() => {
      expect(screen.getByTestId("waf-policy-card")).toBeInTheDocument();
      expect(screen.getByTestId("waf-mode-badge")).toHaveTextContent(/Detect/);
    });

    fireEvent.click(screen.getByTestId("waf-mode-block"));
    fireEvent.change(screen.getByTestId("waf-rule-input"), { target: { value: "942100" } });
    fireEvent.click(screen.getByTestId("waf-add-rule"));
    fireEvent.click(screen.getByTestId("waf-save"));

    await waitFor(() => {
      expect(mockApiFetch).toHaveBeenCalledWith(
        "/api/v1/client/waf/domains/dom_01",
        expect.objectContaining({ method: "PUT", body: JSON.stringify({ mode: "block", disabled_rules: [942100] }) }),
      );
      expect(screen.getByTestId("waf-mode-badge")).toHaveTextContent(/Block/);
    });

    // Switch the rule on again
    fireEvent.click(screen.getByTestId("waf-enable-942100"));
    fireEvent.click(screen.getByTestId("waf-save"));
    await waitFor(() => {
      expect(mockApiFetch).toHaveBeenLastCalledWith(
        "/api/v1/client/waf/domains/dom_01",
        expect.objectContaining({ body: JSON.stringify({ mode: "block", disabled_rules: [] }) }),
      );
    });
  });

  it("AdminWafPage: reads and sets the panel-wide policy", async () => {
    mockApiFetch.mockImplementation((url: string, opts?: any) => {
      if (url === "/api/v1/admin/waf/policy" && opts?.method === "PUT") {
        return Promise.resolve({ effective: { mode: "block", mode_from: "global", disabled_rules: [] }, policy: null });
      }
      if (url === "/api/v1/admin/waf/policy") {
        return Promise.resolve({ effective: { mode: "detect", mode_from: "default", disabled_rules: [] }, policy: null });
      }
      return Promise.resolve({ items: [] });
    });

    renderWithToast(<AdminWafPage />);

    await waitFor(() => {
      expect(screen.getByTestId("admin-waf-title")).toBeInTheDocument();
      expect(screen.getByTestId("waf-policy-card")).toBeInTheDocument();
    });

    fireEvent.click(screen.getByTestId("waf-mode-block"));
    fireEvent.click(screen.getByTestId("waf-save"));

    await waitFor(() => {
      expect(mockApiFetch).toHaveBeenCalledWith(
        "/api/v1/admin/waf/policy",
        expect.objectContaining({ method: "PUT", body: JSON.stringify({ mode: "block", disabled_rules: [] }) }),
      );
    });
  });
});
