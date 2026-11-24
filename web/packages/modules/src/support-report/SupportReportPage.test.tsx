import { describe, it, expect, vi, beforeEach } from "vitest";
import React from "react";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { SupportReportPage } from "./SupportReportPage";
import {
  sanitizeSupportBundle,
  verifyNoSecrets,
  scrubString,
  MOCK_DIAGNOSTIC_BUNDLE,
} from "./types";
import { useToast } from "@builtpanel/ui";
import { apiFetch } from "@builtpanel/api";

vi.mock("@builtpanel/ui", async () => {
  const actual = await vi.importActual<any>("@builtpanel/ui");
  return {
    ...actual,
    useToast: vi.fn(),
  };
});

vi.mock("@builtpanel/api", () => ({
  apiFetch: vi.fn(),
}));

vi.mock("@builtpanel/i18n", () => ({
  useTranslation: () => ({
    t: (key: string) => key,
    i18n: { language: "en" },
  }),
}));

describe("Support Report & Zero-Secrets Verification (T18.12 DoD)", () => {
  describe("scrubString & sanitizeSupportBundle", () => {
    it("scrubs database connection passwords from URIs", () => {
      const rawUri = "postgres://root:p@ssw0rd123!@10.0.0.5:5432/builtpanel_prod";
      const scrubbed = scrubString(rawUri);
      expect(scrubbed).not.toContain("p@ssw0rd123!");
      expect(scrubbed).toContain("postgres://root:[REDACTED_PASSWORD]@10.0.0.5:5432/builtpanel_prod");
    });

    it("scrubs Bearer JWT authorization tokens", () => {
      const rawLog = "GET /api/v1/users Authorization: Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.e30.t-ID";
      const scrubbed = scrubString(rawLog);
      expect(scrubbed).not.toContain("eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.e30.t-ID");
      expect(scrubbed).toContain("Bearer [REDACTED_TOKEN]");
    });

    it("scrubs PEM format private keys", () => {
      const rawKey = `-----BEGIN RSA PRIVATE KEY-----
MIIEowIBAAKCAQEA0Y1Wd...secretkeybytes...
-----END RSA PRIVATE KEY-----`;
      const scrubbed = scrubString(rawKey);
      expect(scrubbed).not.toContain("MIIEowIBAAKCAQEA0Y1Wd");
      expect(scrubbed).toBe("[REDACTED_PRIVATE_KEY]");
    });

    it("masks confidential keys inside config trees", () => {
      const rawConfig = {
        database_password: "super_secret_db_pass",
        jwt_secret: "my_very_private_key_value",
        api_token: "tok_live_994821034",
        nested: {
          app_secret: "12345",
          cluster_mode: "ha",
        },
      };

      const sanitized = sanitizeSupportBundle(rawConfig) as any;
      expect(sanitized.database_password).toBe("[REDACTED_CONFIDENTIAL]");
      expect(sanitized.jwt_secret).toBe("[REDACTED_CONFIDENTIAL]");
      expect(sanitized.api_token).toBe("[REDACTED_CONFIDENTIAL]");
      expect(sanitized.nested.app_secret).toBe("[REDACTED_CONFIDENTIAL]");
      expect(sanitized.nested.cluster_mode).toBe("ha");
    });
  });

  describe("verifyNoSecrets validation function (DoD)", () => {
    it("confirms MOCK_DIAGNOSTIC_BUNDLE is audited clean with zero leaks", () => {
      const result = verifyNoSecrets(MOCK_DIAGNOSTIC_BUNDLE);
      expect(result.clean).toBe(true);
      expect(result.leaks).toEqual([]);
    });

    it("detects leaks if unmasked private key is present", () => {
      const leakedBundle = {
        ...MOCK_DIAGNOSTIC_BUNDLE,
        sanitized_logs: [
          {
            timestamp: new Date().toISOString(),
            level: "error" as const,
            service: "tls",
            message: "-----BEGIN PRIVATE KEY-----\nMIIEvgIBADANBgk...\n-----END PRIVATE KEY-----",
          },
        ],
      };

      const result = verifyNoSecrets(leakedBundle);
      expect(result.clean).toBe(false);
      expect(result.leaks).toContain("Unmasked private key header found");
    });

    it("detects leaks if unmasked database URI is present", () => {
      const leakedBundle = {
        ...MOCK_DIAGNOSTIC_BUNDLE,
        sanitized_config: {
          dsn: "postgres://admin:unmasked_secret@10.0.0.1:5432/db",
        },
      };

      const result = verifyNoSecrets(leakedBundle);
      expect(result.clean).toBe(false);
      expect(result.leaks).toContain("Plaintext database password in URI found");
    });
  });

  describe("SupportReportPage Component", () => {
    const mockToast = vi.fn();

    beforeEach(() => {
      vi.clearAllMocks();
      (useToast as any).mockReturnValue({ toast: mockToast });
      (apiFetch as any).mockResolvedValue(MOCK_DIAGNOSTIC_BUNDLE);
    });

    it("renders page, metadata, services, and zero-secrets badge", async () => {
      render(<SupportReportPage />);

      expect(screen.getByTestId("support-report-page")).toBeInTheDocument();
      expect(screen.getByTestId("badge-zero-secrets")).toHaveTextContent("Audited Clean");
      expect(screen.getByTestId("meta-bundle-id")).toHaveTextContent("diag-20261114-8841");
      expect(screen.getByTestId("sanitized-config-preview")).toBeInTheDocument();
    });

    it("copies diagnostic bundle to clipboard", async () => {
      const writeTextMock = vi.fn().mockResolvedValue(undefined);
      Object.assign(navigator, {
        clipboard: {
          writeText: writeTextMock,
        },
      });

      render(<SupportReportPage />);

      const copyBtn = screen.getByTestId("btn-copy-bundle");
      fireEvent.click(copyBtn);

      await waitFor(() => {
        expect(writeTextMock).toHaveBeenCalledTimes(1);
        const copied = JSON.parse(writeTextMock.mock.calls[0][0]);
        expect(copied.metadata.bundle_id).toBe("diag-20261114-8841");
      });
    });

    it("initiates download of sanitized bundle JSON", () => {
      const createObjectURLMock = vi.fn().mockReturnValue("blob:http://localhost/test");
      const revokeObjectURLMock = vi.fn();
      global.URL.createObjectURL = createObjectURLMock;
      global.URL.revokeObjectURL = revokeObjectURLMock;

      render(<SupportReportPage />);

      const downloadBtn = screen.getByTestId("btn-download-bundle");
      fireEvent.click(downloadBtn);

      expect(createObjectURLMock).toHaveBeenCalledTimes(1);
      expect(mockToast).toHaveBeenCalledWith(
        expect.objectContaining({
          title: "support.downloadStarted",
        })
      );
    });
  });
});
