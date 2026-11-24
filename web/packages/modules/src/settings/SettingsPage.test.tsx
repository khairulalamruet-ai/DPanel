import React from "react";
import { describe, it, expect, beforeEach } from "vitest";
import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import { ToastProvider } from "@builtpanel/ui";
import { resetMockSettings } from "@builtpanel/api";
import { SettingsPage } from "./SettingsPage";

function renderWithProviders(ui: React.ReactElement) {
  return render(<ToastProvider>{ui}</ToastProvider>);
}

describe("SettingsPage (T03.13)", () => {
  beforeEach(() => {
    resetMockSettings();
  });

  it("renders page header and module tabs", async () => {
    renderWithProviders(<SettingsPage />);

    expect(screen.getByText(/Platform Settings|প্ল্যাটফর্ম সেটিংস/i)).toBeInTheDocument();

    await waitFor(() => {
      expect(screen.queryByText(/Loading|লোড হচ্ছে/i)).not.toBeInTheDocument();
    });

    // Check tabs
    expect(screen.getByRole("button", { name: /General|সাধারণ/i })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Security|নিরাপত্তা/i })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Backups|ব্যাকআপ/i })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Notifications|বিজ্ঞপ্তি/i })).toBeInTheDocument();
  });

  it("renders General settings form fields by default", async () => {
    renderWithProviders(<SettingsPage />);

    await waitFor(() => {
      expect(screen.getByLabelText(/Panel Name/i)).toBeInTheDocument();
    });

    expect(screen.getByLabelText(/Support Contact Email/i)).toBeInTheDocument();
    expect(screen.getByLabelText(/Session Timeout/i)).toBeInTheDocument();
    expect(screen.getByLabelText(/Maintenance Mode/i)).toBeInTheDocument();
    expect(screen.getByLabelText(/Default Language/i)).toBeInTheDocument();
  });

  it("switches to Security tab and displays Security schema form", async () => {
    renderWithProviders(<SettingsPage />);

    await waitFor(() => {
      expect(screen.getByRole("button", { name: /Security|নিরাপত্তা/i })).toBeInTheDocument();
    });

    const securityTab = screen.getByRole("button", { name: /Security|নিরাপত্তা/i });
    fireEvent.click(securityTab);

    await waitFor(() => {
      expect(screen.getByLabelText(/Minimum Password Length/i)).toBeInTheDocument();
    });

    expect(screen.getByLabelText(/Max Failed Login Attempts/i)).toBeInTheDocument();
    expect(screen.getByLabelText(/Enforce 2FA for Administrators/i)).toBeInTheDocument();
  });

  it("switches to Backups tab and displays Backups schema form with enum select", async () => {
    renderWithProviders(<SettingsPage />);

    await waitFor(() => {
      expect(screen.getByRole("button", { name: /Backups|ব্যাকআপ/i })).toBeInTheDocument();
    });

    const backupTab = screen.getByRole("button", { name: /Backups|ব্যাকআপ/i });
    fireEvent.click(backupTab);

    await waitFor(() => {
      expect(screen.getByLabelText(/Retention Period/i)).toBeInTheDocument();
    });

    const compressionSelect = screen.getByLabelText(/Compression Algorithm/i);
    expect(compressionSelect).toBeInTheDocument();
    expect(screen.getByRole("option", { name: /Zstandard/i })).toBeInTheDocument();
  });

  it("saves modified settings and displays success feedback", async () => {
    renderWithProviders(<SettingsPage />);

    await waitFor(() => {
      expect(screen.getByLabelText(/Panel Name/i)).toBeInTheDocument();
    });

    const panelNameInput = screen.getByLabelText(/Panel Name/i);
    fireEvent.change(panelNameInput, { target: { value: "Updated Built Panel Pro" } });

    const submitBtn = screen.getByRole("button", { name: /Save Changes|পরিবর্তন সংরক্ষণ করুন/i });
    fireEvent.click(submitBtn);

    await waitFor(() => {
      expect(
        screen.getByText(/Settings for 'General' updated successfully|সেটিংস সফলভাবে সংরক্ষিত হয়েছে/i)
      ).toBeInTheDocument();
    });
  });
});
