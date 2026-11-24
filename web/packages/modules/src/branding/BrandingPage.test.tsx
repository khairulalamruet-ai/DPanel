import { describe, it, expect, vi, beforeEach } from "vitest";
import React from "react";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { BrandingPage } from "./BrandingPage";
import { sanitizeCustomCss, DEFAULT_BRANDING } from "./types";
import { useSession } from "@builtpanel/auth";
import { useToast } from "@builtpanel/ui";

vi.mock("@builtpanel/auth", () => ({
  useSession: vi.fn(),
}));

vi.mock("@builtpanel/ui", async () => {
  const actual = await vi.importActual<any>("@builtpanel/ui");
  return {
    ...actual,
    useToast: vi.fn(),
  };
});

vi.mock("@builtpanel/api", () => ({
  apiFetch: vi.fn().mockImplementation(async (url: string, opts?: any) => {
    if (opts?.method === "PUT") {
      return JSON.parse(opts.body);
    }
    return DEFAULT_BRANDING;
  }),
}));

vi.mock("@builtpanel/i18n", () => ({
  useTranslation: () => ({
    t: (key: string) => key,
    i18n: { language: "en" },
  }),
}));

describe("sanitizeCustomCss", () => {
  it("strips script and html tags", () => {
    const malicious = "body { color: red; }</style><script>alert('xss')</script><style>";
    const cleaned = sanitizeCustomCss(malicious);
    expect(cleaned).not.toContain("<script>");
    expect(cleaned).not.toContain("alert('xss')");
    expect(cleaned).not.toContain("</style>");
    expect(cleaned).toContain("body { color: red; }");
  });

  it("blocks javascript pseudo-protocols", () => {
    const malicious = "background: url('javascript:alert(1)')";
    const cleaned = sanitizeCustomCss(malicious);
    expect(cleaned).not.toContain("javascript:");
    expect(cleaned).toContain("blocked-js:");
  });

  it("neutralizes CSS expressions and behavior", () => {
    const malicious = "width: expression(document.body.clientWidth > 500); behavior: url(test.htc);";
    const cleaned = sanitizeCustomCss(malicious);
    expect(cleaned).not.toContain("expression(");
    expect(cleaned).toContain("behavior: none");
  });

  it("blocks remote @import rules", () => {
    const malicious = "@import url('https://evil.com/leak.css'); div { font-size: 14px; }";
    const cleaned = sanitizeCustomCss(malicious);
    expect(cleaned).not.toContain("@import url('https://evil.com/leak.css');");
    expect(cleaned).toContain("/* @import blocked for security */");
    expect(cleaned).toContain("div { font-size: 14px; }");
  });

  it("preserves valid CSS rules", () => {
    const valid = ".my-card { background: #fff; padding: 16px; border-radius: 8px; }";
    expect(sanitizeCustomCss(valid)).toBe(valid);
  });
});

describe("BrandingPage Component", () => {
  const mockSetSessionData = vi.fn();
  const mockToast = vi.fn();

  beforeEach(() => {
    vi.clearAllMocks();
    (useToast as any).mockReturnValue({ toast: mockToast });
    (useSession as any).mockReturnValue({
      data: {
        user: { id: "1", username: "admin", name: "Admin" },
        roles: ["admin"],
        permissions: ["*"],
        branding: {
          companyName: "Built Panel",
          logoUrl: "",
          faviconUrl: "",
          accentColor: "#3b82f6",
          loginTitle: "Built Panel",
          loginSubtitle: "Sign in",
          loginFooterText: "Built Panel Cloud",
          nameservers: ["ns1.builtpanel.net", "ns2.builtpanel.net"],
          customCss: "",
        },
      },
      setSessionData: mockSetSessionData,
    });
  });

  it("renders branding configuration form and interactive live preview", async () => {
    render(<BrandingPage />);

    expect(screen.getByTestId("branding-page")).toBeInTheDocument();
    expect(screen.getByTestId("branding-name-input")).toHaveValue("Built Panel");
    expect(screen.getByTestId("branding-live-preview")).toBeInTheDocument();
    expect(screen.getByTestId("preview-brand-name")).toHaveTextContent("Built Panel");
  });

  it("updates preview and applies branding live to session on submit without reload", async () => {
    const reloadSpy = vi.fn();
    Object.defineProperty(window, "location", {
      configurable: true,
      value: { reload: reloadSpy },
    });

    render(<BrandingPage />);

    const nameInput = screen.getByTestId("branding-name-input");
    fireEvent.change(nameInput, { target: { value: "Apex Cloud Host" } });

    const cssInput = screen.getByTestId("branding-css-input");
    fireEvent.change(cssInput, {
      target: { value: "<script>bad()</script>.custom-header { color: blue; }" },
    });

    // Verify live preview updated immediately
    expect(screen.getByTestId("preview-brand-name")).toHaveTextContent("Apex Cloud Host");

    // Submit form
    const saveBtn = screen.getByTestId("branding-save-btn");
    fireEvent.click(saveBtn);

    await waitFor(() => {
      expect(mockSetSessionData).toHaveBeenCalledTimes(1);
    });

    // Check that session data received sanitized CSS and updated companyName
    const updatedData = mockSetSessionData.mock.calls[0][0];
    expect(updatedData.branding.companyName).toBe("Apex Cloud Host");
    expect(updatedData.branding.customCss).not.toContain("<script>");
    expect(updatedData.branding.customCss).toContain(".custom-header { color: blue; }");

    // Check toast was shown
    expect(mockToast).toHaveBeenCalledWith(
      expect.objectContaining({
        title: "branding.savedTitle",
      })
    );

    // Verify DoD: reload was NOT called (updates apply live without reload)
    expect(reloadSpy).not.toHaveBeenCalled();
  });
});
