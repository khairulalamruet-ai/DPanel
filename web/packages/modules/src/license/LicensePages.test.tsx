import React from "react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { LicenseState } from "./types";
import { LicenseStatusBanner } from "./LicenseStatusBanner";
import { LicenseDetailsCard } from "./LicenseDetailsCard";
import { LicenseEntitlementsCard } from "./LicenseEntitlementsCard";
import { ActivateLicenseDialog } from "./ActivateLicenseDialog";
import { RecoveryRequestDialog } from "./RecoveryRequestDialog";
import { LicensePage } from "./LicensePage";

const mockLicense: LicenseState = {
  license_id: "LIC-98421-ENT",
  license_key: "DHP-PRO-8492-9182-AA19",
  installation_id: "7b4c9102-4d82-41e9-9a21-fa9921bc901a",
  customer_name: "Acme Web Services LLC",
  customer_email: "billing@acmeweb.com",
  product_name: "Built Panel Enterprise Edition",
  plan_tier: "enterprise",
  status: "active",
  issued_at: "2026-01-01T00:00:00Z",
  expires_at: "2027-01-01T00:00:00Z",
  last_heartbeat_at: "2026-11-13T12:00:00Z",
  next_refresh_at: "2026-11-14T00:00:00Z",
  signing_key_id: "key-ed25519-prod-1",
  signature_valid: true,
  features: {
    reseller_enabled: true,
    clustering_enabled: true,
    advanced_backups: true,
    commercial_waf: true,
    whitelabel_enabled: true,
    api_access: true,
    max_nodes: 10,
    max_tenants: 100,
    max_domains: 500,
  },
  usage: {
    nodes_used: 3,
    tenants_used: 28,
    domains_used: 64,
  },
};

describe("License Management Module (T17.13)", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  describe("LicenseStatusBanner", () => {
    it("renders active state with verified badge", () => {
      render(
        <LicenseStatusBanner
          license={mockLicense}
          onRevalidate={vi.fn()}
          onOpenActivate={vi.fn()}
          onOpenRecovery={vi.fn()}
          revalidating={false}
        />
      );

      expect(screen.getByTestId("license-banner-active")).toBeDefined();
      expect(screen.getByText(/ACTIVE|সক্রিয়/i)).toBeDefined();
      expect(screen.getByText("Ed25519 Valid")).toBeDefined();
    });

    it("renders grace period banner with countdown and retry action", () => {
      const graceLicense: LicenseState = {
        ...mockLicense,
        status: "grace_period",
        offline_grace_hours_remaining: 48,
      };

      const handleRevalidate = vi.fn();
      render(
        <LicenseStatusBanner
          license={graceLicense}
          onRevalidate={handleRevalidate}
          onOpenActivate={vi.fn()}
          onOpenRecovery={vi.fn()}
          revalidating={false}
        />
      );

      expect(screen.getByTestId("license-banner-grace")).toBeDefined();
      expect(screen.getByText("48h Remaining")).toBeDefined();

      fireEvent.click(screen.getByTestId("btn-revalidate-grace"));
      expect(handleRevalidate).toHaveBeenCalledTimes(1);
    });

    it("renders restricted mode banner with invariant safety notice", () => {
      const restrictedLicense: LicenseState = {
        ...mockLicense,
        status: "restricted",
        restricted_reason: "License heartbeat expired past 72h offline grace.",
      };

      render(
        <LicenseStatusBanner
          license={restrictedLicense}
          onRevalidate={vi.fn()}
          onOpenActivate={vi.fn()}
          onOpenRecovery={vi.fn()}
          revalidating={false}
        />
      );

      expect(screen.getByTestId("license-banner-restricted")).toBeDefined();
      expect(screen.getByText(/Safety Invariant|নিরাপত্তা নীতি/i)).toBeDefined();
    });
  });

  describe("LicenseDetailsCard", () => {
    it("renders installation ID, masked key, and toggles reveal", () => {
      render(
        <LicenseDetailsCard
          license={mockLicense}
          onOpenActivate={vi.fn()}
          onOpenRecovery={vi.fn()}
          onRevalidate={vi.fn()}
          revalidating={false}
        />
      );

      expect(screen.getByTestId("text-installation-id").textContent).toBe(mockLicense.installation_id);

      // Initially masked
      expect(screen.getByTestId("text-license-key").textContent).toContain("•");

      // Toggle show
      fireEvent.click(screen.getByTestId("btn-toggle-show-key"));
      expect(screen.getByTestId("text-license-key").textContent).toBe(mockLicense.license_key);
    });

    it("copies license key to clipboard", async () => {
      const writeTextMock = vi.fn().mockResolvedValue(undefined);
      Object.assign(navigator, {
        clipboard: {
          writeText: writeTextMock,
        },
      });

      render(
        <LicenseDetailsCard
          license={mockLicense}
          onOpenActivate={vi.fn()}
          onOpenRecovery={vi.fn()}
          onRevalidate={vi.fn()}
          revalidating={false}
        />
      );

      fireEvent.click(screen.getByTestId("btn-copy-license-key"));
      expect(writeTextMock).toHaveBeenCalledWith(mockLicense.license_key);
    });
  });

  describe("LicenseEntitlementsCard", () => {
    it("renders capacity quota bars and signed feature badges", () => {
      render(
        <LicenseEntitlementsCard
          features={mockLicense.features}
          usage={mockLicense.usage}
        />
      );

      expect(screen.getByTestId("quota-bar-nodes")).toBeDefined();
      expect(screen.getByTestId("quota-bar-tenants")).toBeDefined();
      expect(screen.getByTestId("quota-bar-domains")).toBeDefined();

      expect(screen.getByTestId("feature-item-reseller_enabled")).toBeDefined();
      expect(screen.getByTestId("feature-item-clustering_enabled")).toBeDefined();
      expect(screen.getByTestId("feature-item-commercial_waf")).toBeDefined();
    });
  });

  describe("ActivateLicenseDialog", () => {
    it("validates license key prefix and calls onActivate", async () => {
      const handleActivate = vi.fn().mockResolvedValue(undefined);
      render(
        <ActivateLicenseDialog
          open={true}
          onClose={vi.fn()}
          onActivate={handleActivate}
        />
      );

      // Try invalid key
      fireEvent.change(screen.getByTestId("input-license-key"), {
        target: { value: "INVALID-KEY-123" },
      });
      fireEvent.click(screen.getByTestId("btn-submit-activate"));

      await waitFor(() => {
        expect(screen.getByTestId("activate-error-message")).toBeDefined();
      });
      expect(handleActivate).not.toHaveBeenCalled();

      // Enter valid key with DHP- prefix
      fireEvent.change(screen.getByTestId("input-license-key"), {
        target: { value: "DHP-ENT-9921-8812-ZZ40" },
      });
      fireEvent.click(screen.getByTestId("btn-submit-activate"));

      await waitFor(() => {
        expect(handleActivate).toHaveBeenCalledWith("DHP-ENT-9921-8812-ZZ40");
      });
    });
  });

  describe("RecoveryRequestDialog", () => {
    it("submits emergency recovery request and displays ticket", async () => {
      const handleSubmit = vi.fn().mockResolvedValue({
        ticket_id: "REC-9104-X",
        status: "pending",
        created_at: new Date().toISOString(),
        message: "Recovery request logged with License Authority.",
      });

      render(
        <RecoveryRequestDialog
          open={true}
          onClose={vi.fn()}
          onSubmit={handleSubmit}
        />
      );

      fireEvent.change(screen.getByTestId("input-recovery-email"), {
        target: { value: "admin@enterprise.io" },
      });
      fireEvent.change(screen.getByTestId("input-recovery-notes"), {
        target: { value: "Recovering from primary SSD migration." },
      });

      fireEvent.click(screen.getByTestId("btn-submit-recovery"));

      await waitFor(() => {
        expect(screen.getByTestId("recovery-success-view")).toBeDefined();
        expect(screen.getByTestId("text-recovery-ticket-id").textContent).toBe("REC-9104-X");
      });
    });
  });
});
