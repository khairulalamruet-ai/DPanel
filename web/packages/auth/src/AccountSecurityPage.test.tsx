import { describe, it, expect, vi, beforeEach } from "vitest";
import React from "react";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { AccountSecurityPage } from "./AccountSecurityPage";
import * as builtpanelApi from "@builtpanel/api";
const { api } = builtpanelApi;

vi.mock("@builtpanel/i18n", () => ({
  useTranslation: () => ({
    t: (key: string) => {
      const translations: Record<string, string> = {
        "security.title": "Account Security",
        "security.desc": "Manage your password, two-factor authentication, active sessions, and review login history.",
        "security.tabs.password": "Password",
        "security.tabs.mfa": "Two-Factor Auth",
        "security.tabs.sessions": "Active Sessions",
        "security.tabs.history": "Login History",
        "security.password.changeTitle": "Change Password",
        "security.password.desc": "Ensure your account is using a long password.",
        "security.password.current": "Current Password",
        "security.password.currentPlaceholder": "Enter current password",
        "security.password.new": "New Password",
        "security.password.newPlaceholder": "At least 8 characters",
        "security.password.confirm": "Confirm New Password",
        "security.password.confirmPlaceholder": "Re-enter new password",
        "security.password.updateBtn": "Update Password",
        "security.password.updating": "Updating...",
        "security.password.success": "Password updated successfully",
        "security.password.mismatch": "New passwords do not match",
        "security.password.tooShort": "Password must be at least 12 characters",
        "security.mfa.title": "Two-Factor Authentication (TOTP)",
        "security.mfa.desc": "Add an extra layer of security.",
        "security.mfa.status": "2FA Status",
        "security.mfa.enabled": "Enabled",
        "security.mfa.disabled": "Disabled",
        "security.mfa.enableBtn": "Enable 2FA",
        "security.mfa.disableBtn": "Disable 2FA",
        "security.mfa.scanQr": "Scan this QR code with your authenticator app:",
        "security.mfa.secretKey": "Secret Key",
        "security.mfa.enterCode": "Enter 6-digit TOTP code",
        "security.mfa.verifyAndEnable": "Verify & Enable",
        "security.mfa.verifying": "Verifying...",
        "security.mfa.backupCodes": "Backup Codes",
        "security.mfa.backupCodesDesc": "Store these backup codes safely.",
        "security.mfa.copyAll": "Copy All",
        "security.mfa.copied": "Copied!",
        "security.mfa.regenerateCodes": "Regenerate Backup Codes",
        "security.mfa.cancel": "Cancel",
        "security.mfa.invalidCode": "Invalid 6-digit TOTP code.",
        "security.sessions.title": "Active Sessions",
        "security.sessions.desc": "These devices are currently logged in.",
        "security.sessions.device": "Device / Browser",
        "security.sessions.ip": "IP Address",
        "security.sessions.lastActive": "Last Active",
        "security.sessions.current": "Current Session",
        "security.sessions.revoke": "Revoke",
        "security.sessions.revokeOthers": "Revoke All Other Sessions",
        "security.sessions.revokedSuccess": "Session revoked successfully",
        "security.sessions.revokeOthersSuccess": "All other sessions have been revoked",
        "security.history.title": "Login Events History",
        "security.history.desc": "Recent authentication attempts.",
        "security.history.date": "Timestamp",
        "security.history.ip": "IP Address",
        "security.history.client": "Client / Browser",
        "security.history.status": "Status",
        "security.history.success": "Successful",
        "security.history.failed": "Failed",
        "security.history.prompt": "2FA Prompt",
        "common.error": "An error occurred",
        "common.loading": "Loading...",
      };
      return translations[key] || key;
    },
  }),
  useFormatters: () => ({
    formatDate: (d: Date) => d.toISOString(),
  }),
}));

describe("AccountSecurityPage", () => {
  beforeEach(() => {
    vi.restoreAllMocks();

    vi.spyOn(builtpanelApi, "apiFetch").mockImplementation(async (url: string) => {
      if (url === "/api/v1/auth/password") {
        return { other_sessions_ended: 0 };
      }
      if (url === "/api/v1/auth/me") {
        return { id: "acc_1", kind: "client" };
      }
      if (url.startsWith("/api/v1/client/activity")) {
        return {
          items: [
            { id: "evt_1", action: "auth.login", ip: "127.0.0.1", outcome: "success", at: new Date().toISOString(), by_you: true },
          ],
        };
      }
      return {};
    });

    vi.spyOn(api, "GET").mockImplementation(async (url: any) => {
      if (url === "/api/v1/auth/mfa" || url === "/api/v1/account/2fa/status") {
        return { data: { mfa_enabled: false, backup_codes_remaining: 0 }, response: new Response() } as any;
      }
      if (url === "/api/v1/auth/sessions" || url === "/api/v1/account/sessions") {
        return {
          data: {
            sessions: [
              {
                id: "sess_curr",
                ip: "127.0.0.1",
                user_agent: "Chrome 130",
                created_at: new Date().toISOString(),
                last_seen_at: new Date().toISOString(),
                current: true,
              },
              {
                id: "sess_other",
                ip: "10.0.0.2",
                user_agent: "Firefox 131",
                created_at: new Date().toISOString(),
                last_seen_at: new Date().toISOString(),
                current: false,
              },
            ],
          },
          response: new Response(),
        } as any;
      }
      if (url === "/api/v1/account/login-history") {
        return {
          data: {
            events: [
              {
                id: "evt_1",
                ip_address: "127.0.0.1",
                user_agent: "Chrome 130",
                status: "success",
                created_at: new Date().toISOString(),
              },
            ],
          },
          response: new Response(),
        } as any;
      }
      return { data: null, response: new Response() } as any;
    });

    vi.spyOn(api, "POST").mockImplementation(async (url: any, _opts: any) => {
      if (url === "/api/v1/account/password") {
        return { data: { success: true }, response: new Response() } as any;
      }
      if (url === "/api/v1/auth/mfa/totp/enroll" || url === "/api/v1/account/2fa/enroll") {
        return {
          data: {
            secret: "TESTSECRETKEY123",
            otpauth_uri: "otpauth://totp/Built%20Panel:test?secret=TESTSECRETKEY123",
          },
          response: new Response(),
        } as any;
      }
      if (url === "/api/v1/auth/mfa/totp/confirm" || url === "/api/v1/account/2fa/confirm") {
        return {
          data: {
            backup_codes: ["code1-1111", "code2-2222"],
          },
          response: new Response(),
        } as any;
      }
      return { data: { success: true }, response: new Response() } as any;
    });

    vi.spyOn(api, "DELETE").mockResolvedValue({ data: { success: true }, response: new Response() } as any);
  });

  it("renders tab navigation and default password tab", async () => {
    render(<AccountSecurityPage />);

    expect(screen.getByRole("tab", { name: /^Password$/i })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: /Two-Factor Auth/i })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: /^Active Sessions$/i })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: /^Login History$/i })).toBeInTheDocument();

    expect(screen.getByLabelText(/^Current Password$/i)).toBeInTheDocument();
    expect(screen.getByLabelText(/^New Password$/i)).toBeInTheDocument();
    expect(screen.getByLabelText(/^Confirm New Password$/i)).toBeInTheDocument();
  });

  it("validates password length and matching confirmation", async () => {
    render(<AccountSecurityPage />);

    const currentInput = screen.getByLabelText(/^Current Password$/i);
    const newInput = screen.getByLabelText(/^New Password$/i);
    const confirmInput = screen.getByLabelText(/^Confirm New Password$/i);
    const submitBtn = screen.getByRole("button", { name: /^Update Password$/i });

    // Too short
    fireEvent.change(currentInput, { target: { value: "old-secret" } });
    fireEvent.change(newInput, { target: { value: "short" } });
    fireEvent.change(confirmInput, { target: { value: "short" } });
    fireEvent.click(submitBtn);

    expect(screen.getByText("Password must be at least 12 characters")).toBeInTheDocument();

    // Mismatched
    fireEvent.change(newInput, { target: { value: "valid-password-1234" } });
    fireEvent.change(confirmInput, { target: { value: "different-password" } });
    fireEvent.click(submitBtn);

    expect(screen.getByText("New passwords do not match")).toBeInTheDocument();

    // Valid
    fireEvent.change(confirmInput, { target: { value: "valid-password-1234" } });
    fireEvent.click(submitBtn);

    await waitFor(() => {
      expect(screen.getByText("Password updated successfully")).toBeInTheDocument();
    });
  });

  it("switches to 2FA tab, starts enrollment, confirms code, and displays backup codes", async () => {
    render(<AccountSecurityPage />);

    const mfaTab = screen.getByRole("tab", { name: /Two-Factor Auth/i });
    fireEvent.click(mfaTab);

    // Initial 2FA state is Disabled
    const enableBtn = await screen.findByRole("button", { name: /^Enable 2FA$/i });
    fireEvent.click(enableBtn);

    // Shows Secret Key and QR step
    await screen.findByText("TESTSECRETKEY123");
    expect(screen.getByText("Scan this QR code with your authenticator app:")).toBeInTheDocument();

    // Fill 6-digit TOTP code
    const totpInput = screen.getByLabelText(/^Enter 6-digit TOTP code$/i);
    fireEvent.change(totpInput, { target: { value: "123456" } });

    const verifyBtn = screen.getByRole("button", { name: /^Verify & Enable$/i });
    fireEvent.click(verifyBtn);

    // Displays backup codes
    await screen.findByText("code1-1111");
    expect(screen.getByText("code2-2222")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Copy All/i })).toBeInTheDocument();
  });

  it("switches to Active Sessions tab and handles session revocation", async () => {
    render(<AccountSecurityPage />);

    const sessionsTab = screen.getByRole("tab", { name: /^Active Sessions$/i });
    fireEvent.click(sessionsTab);

    await screen.findByText("127.0.0.1");
    expect(screen.getByText("Current Session")).toBeInTheDocument();
    expect(screen.getByText("10.0.0.2")).toBeInTheDocument();

    // Revoke other session
    const revokeBtn = screen.getByRole("button", { name: /^Revoke$/i });
    fireEvent.click(revokeBtn);

    await waitFor(() => {
      expect(api.DELETE).toHaveBeenCalledWith("/api/v1/auth/sessions/{id}", expect.anything());
    });
  });

  it("switches to Login History tab and displays events", async () => {
    render(<AccountSecurityPage />);

    const historyTab = screen.getByRole("tab", { name: /^Login History$/i });
    fireEvent.click(historyTab);

    await screen.findByText("127.0.0.1");
    expect(screen.getByText("Successful")).toBeInTheDocument();
  });
});
