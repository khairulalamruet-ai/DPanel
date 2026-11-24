import React from "react";
import { describe, it, expect, beforeEach } from "vitest";
import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import { ToastProvider } from "@builtpanel/ui";
import { resetMockPreviewLinks } from "@builtpanel/api";
import { PreviewLinkDialog } from "./PreviewLinkDialog";
import { ClientDomainItem } from "./types";

const mockDomain: ClientDomainItem = {
  id: "dom_01",
  name: "alpha-hosting.com",
  type: "primary",
  docroot: "/home/demo/public_html",
  is_primary: true,
  ssl_status: "active",
  dns_status: "active",
  created_at: "2026-11-10T12:00:00Z",
};

const mockExpiredDomain: ClientDomainItem = {
  id: "dom_02",
  name: "blog.alpha-hosting.com",
  type: "subdomain",
  docroot: "/home/demo/public_html/blog",
  is_primary: false,
  ssl_status: "active",
  dns_status: "active",
  created_at: "2026-11-10T12:00:00Z",
};

function renderWithProviders(ui: React.ReactElement) {
  return render(<ToastProvider>{ui}</ToastProvider>);
}

describe("PreviewLinkDialog (T06.14)", () => {
  beforeEach(() => {
    resetMockPreviewLinks();
  });

  it("renders preview dialog and generates a new temporary preview link", async () => {
    renderWithProviders(
      <PreviewLinkDialog open={true} onOpenChange={() => {}} domain={mockDomain} />
    );

    // Initial state: not enabled
    await waitFor(() => {
      expect(screen.getByTestId("btn-generate-preview-link")).toBeInTheDocument();
    });

    // Click Generate
    fireEvent.click(screen.getByTestId("btn-generate-preview-link"));

    // Verify active link is displayed
    await waitFor(() => {
      expect(screen.getByTestId("badge-preview-status-active")).toBeInTheDocument();
      expect(screen.getByTestId("input-preview-url")).toBeInTheDocument();
    });

    const input = screen.getByTestId("input-preview-url") as HTMLInputElement;
    expect(input.value).toMatch(/https:\/\/prv_[a-z0-9]+\.preview\.builtpanel\.net/);
  });

  it("DoD: displays expired state and warning banner for an expired preview link", async () => {
    renderWithProviders(
      <PreviewLinkDialog open={true} onOpenChange={() => {}} domain={mockExpiredDomain} />
    );

    // Pre-seeded dom_02 has an expired preview link
    await waitFor(() => {
      expect(screen.getByTestId("banner-preview-expired")).toBeInTheDocument();
    });

    expect(screen.getByText(/Preview Link Expired|প্রিভিউ লিংকের মেয়াদ শেষ/i)).toBeInTheDocument();
    expect(screen.getByTestId("btn-generate-preview-link")).toBeInTheDocument();
  });

  it("revokes an active preview link", async () => {
    renderWithProviders(
      <PreviewLinkDialog open={true} onOpenChange={() => {}} domain={mockDomain} />
    );

    // Generate active link first
    await waitFor(() => {
      expect(screen.getByTestId("btn-generate-preview-link")).toBeInTheDocument();
    });
    fireEvent.click(screen.getByTestId("btn-generate-preview-link"));

    await waitFor(() => {
      expect(screen.getByTestId("btn-revoke-preview-link")).toBeInTheDocument();
    });

    // Click Revoke
    fireEvent.click(screen.getByTestId("btn-revoke-preview-link"));

    // Returns to generate state
    await waitFor(() => {
      expect(screen.getByTestId("btn-generate-preview-link")).toBeInTheDocument();
    });
  });

  it("makes a new preview link while one is active", async () => {
    renderWithProviders(
      <PreviewLinkDialog open={true} onOpenChange={() => {}} domain={mockDomain} />
    );

    // Generate active link first
    await waitFor(() => {
      expect(screen.getByTestId("btn-generate-preview-link")).toBeInTheDocument();
    });
    fireEvent.click(screen.getByTestId("btn-generate-preview-link"));

    await waitFor(() => {
      expect(screen.getByTestId("btn-extend-preview-link")).toBeInTheDocument();
    });

    fireEvent.click(screen.getByTestId("btn-extend-preview-link"));

    await waitFor(() => {
      expect(screen.getByText(/successfully|সফলভাবে/i)).toBeInTheDocument();
    });
  });
});
