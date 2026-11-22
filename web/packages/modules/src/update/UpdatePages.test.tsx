import React from "react";
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, fireEvent, waitFor, act } from "@testing-library/react";
import { ToastProvider } from "@builtpanel/ui";
import { UpdatePage } from "./UpdatePage";
import { PanelUpdateStatus } from "./types";

const offer = {
  version: "0.2.0",
  release_date: "2026-11-20T10:00:00Z",
  channel: "stable" as const,
  ed25519_signature: "sig",
  key_id: "release-1",
  checksum_sha256: "",
  changelog: ["Per-domain address blocking", "A one-click update"],
  required_migrations: [],
  can_rollback: true,
};

let status: PanelUpdateStatus;
const calls: { url: string; body?: unknown }[] = [];

const mockApiFetch = vi.fn(async (url: string, opts?: any) => {
  calls.push({ url, body: opts?.body ? JSON.parse(opts.body) : undefined });
  if (url === "/api/v1/admin/update/status") return status;
  if (url === "/api/v1/admin/update/check") return { checked: true, available: true, update: offer };
  if (url === "/api/v1/auth/sudo") return {};
  if (url === "/api/v1/admin/settings/system") {
    status = { ...status, auto: { ...status.auto, enabled: JSON.parse(opts.body).values.auto_update, scheduled_for: null } };
    return {};
  }
  if (url === "/api/v1/admin/update/apply") {
    status = {
      ...status,
      run: {
        state: "completed",
        step: "completed",
        version: "0.2.0",
        from_version: "0.1.0",
        message: "Updated from 0.1.0 to 0.2.0.",
        updated_at: new Date(Date.now() + 1000).toISOString(),
      },
    };
    return { ok: true, status: "started", version: "0.2.0" };
  }
  return {};
});

vi.mock("@builtpanel/api", () => ({
  apiFetch: (url: string, opts?: any) => mockApiFetch(url, opts),
}));

const renderPage = () =>
  render(
    <ToastProvider>
      <UpdatePage />
    </ToastProvider>
  );

describe("Panel update (one click)", () => {
  beforeEach(() => {
    calls.length = 0;
    status = {
      current_version: "0.1.0",
      channel: "stable",
      last_checked_at: "2026-11-22T10:00:00Z",
      available_update: offer,
      previous_version: null,
      can_rollback: false,
      can_apply: true,
      run: null,
      auto: { enabled: true, delay_days: 3, hour_utc: 3, scheduled_for: "2026-11-26T03:00:00Z" },
      history: [],
    };
  });
  afterEach(() => vi.useRealTimers());

  it("shows the installed version, the release on offer and its changelog", async () => {
    renderPage();
    await waitFor(() => expect(screen.getByTestId("current-version-display")).toHaveTextContent("0.1.0"));
    expect(screen.getByTestId("update-available-banner")).toHaveTextContent("0.2.0");
    expect(screen.getByText("Per-domain address blocking")).toBeInTheDocument();
    expect(screen.getByTestId("update-service-state")).toHaveTextContent(/Ready|প্রস্তুত/);
  });

  it("asks the release site when checking for updates", async () => {
    renderPage();
    await waitFor(() => expect(screen.getByTestId("btn-check-updates")).toBeInTheDocument());
    fireEvent.click(screen.getByTestId("btn-check-updates"));
    await waitFor(() => expect(calls.some((c) => c.url === "/api/v1/admin/update/check")).toBe(true));
  });

  it("needs the password, then starts the update and follows it to the end", async () => {
    renderPage();
    await waitFor(() => expect(screen.getByTestId("btn-update-now")).toBeInTheDocument());
    fireEvent.click(screen.getByTestId("btn-update-now"));
    // Nothing is sent before the password is given.
    expect(calls.some((c) => c.url === "/api/v1/admin/update/apply")).toBe(false);
    expect(screen.getByTestId("btn-confirm-update")).toBeDisabled();
    fireEvent.change(document.getElementById("update-password") as HTMLInputElement, { target: { value: "correct horse" } });
    fireEvent.click(screen.getByTestId("btn-confirm-update"));
    await waitFor(() => expect(calls.some((c) => c.url === "/api/v1/admin/update/apply")).toBe(true));
    const order = calls.map((c) => c.url).filter((u) => u === "/api/v1/auth/sudo" || u === "/api/v1/admin/update/apply");
    expect(order).toEqual(["/api/v1/auth/sudo", "/api/v1/admin/update/apply"]);
    expect(calls.find((c) => c.url === "/api/v1/admin/update/apply")?.body).toEqual({ version: "0.2.0" });
    // The page waits for the service, then shows what it reports.
    await waitFor(() => expect(screen.getByTestId("update-step-message")).toBeInTheDocument());
    await waitFor(() => expect(screen.getByTestId("update-step-message")).toHaveTextContent("Updated from 0.1.0 to 0.2.0."), { timeout: 6000 });
  });

  it("cannot update on a server without the update service", async () => {
    status = { ...status, can_apply: false };
    renderPage();
    await waitFor(() => expect(screen.getByTestId("btn-update-now")).toBeDisabled());
    expect(screen.getByTestId("update-service-state")).toHaveTextContent(/installer|ইনস্টলার/);
  });

  it("shows the automatic update and switches it off after the password", async () => {
    renderPage();
    await waitFor(() => expect(screen.getByTestId("auto-update-state")).toHaveTextContent(/on|চালু/i));
    expect(screen.getByTestId("auto-update-scheduled")).toHaveTextContent("0.2.0");
    fireEvent.click(screen.getByTestId("btn-toggle-auto"));
    expect(screen.getByTestId("btn-confirm-auto")).toBeDisabled();
    fireEvent.change(document.getElementById("auto-update-password") as HTMLInputElement, { target: { value: "correct horse" } });
    fireEvent.click(screen.getByTestId("btn-confirm-auto"));
    await waitFor(() => expect(calls.some((c) => c.url === "/api/v1/admin/settings/system")).toBe(true));
    expect(calls.find((c) => c.url === "/api/v1/admin/settings/system")?.body).toEqual({ values: { auto_update: false } });
    await waitFor(() => expect(screen.getByTestId("auto-update-state")).toHaveTextContent(/off|বন্ধ/i));
    expect(screen.queryByTestId("auto-update-scheduled")).not.toBeInTheDocument();
  });

  it("says the panel is up to date when nothing newer was found", async () => {
    status = { ...status, available_update: null };
    renderPage();
    await waitFor(() => expect(screen.getByText(/up to date|হালনাগাদ/i)).toBeInTheDocument());
    expect(screen.queryByTestId("btn-update-now")).not.toBeInTheDocument();
  });
});
