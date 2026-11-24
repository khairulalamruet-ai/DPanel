import React from "react";
import { describe, it, expect, vi } from "vitest";
import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import { ToastProvider } from "@builtpanel/ui";
import { LogViewerPage } from "./LogViewerPage";

function renderWithProviders(ui: React.ReactElement) {
  return render(<ToastProvider>{ui}</ToastProvider>);
}

describe("LogViewerPage (T10.06)", () => {
  it("renders page title, log type tabs, and virtualized log lines", async () => {
    renderWithProviders(<LogViewerPage />);

    await waitFor(() => {
      expect(screen.getByTestId("log-viewer-title")).toBeInTheDocument();
    });

    expect(screen.getByTestId("tab-log-access")).toBeInTheDocument();
    expect(screen.getByTestId("tab-log-error")).toBeInTheDocument();
    expect(screen.getByTestId("tab-log-php")).toBeInTheDocument();
    expect(screen.getByTestId("btn-download-log")).toBeInTheDocument();
    expect(screen.getByTestId("virtualized-log-container")).toBeInTheDocument();

    await waitFor(() => {
      expect(screen.getByTestId("metric-loaded-lines")).toHaveTextContent("250");
      expect(screen.getByTestId("virtualized-log-line-0")).toBeInTheDocument();
    });
  });

  it("switches to Error Log and PHP Error Log tabs", async () => {
    renderWithProviders(<LogViewerPage />);

    await waitFor(() => {
      expect(screen.getByTestId("tab-log-error")).toBeInTheDocument();
    });

    fireEvent.click(screen.getByTestId("tab-log-error"));

    await waitFor(() => {
      expect(screen.getByTestId("metric-loaded-lines")).toHaveTextContent("50");
    });

    fireEvent.click(screen.getByTestId("tab-log-php"));

    await waitFor(() => {
      expect(screen.getByTestId("metric-loaded-lines")).toHaveTextContent("40");
    });
  });

  it("filters logs instantly via search input", async () => {
    renderWithProviders(<LogViewerPage />);

    await waitFor(() => {
      expect(screen.getByTestId("virtualized-log-line-0")).toBeInTheDocument();
    });

    const searchInput = screen.getByTestId("input-log-search");
    fireEvent.change(searchInput, { target: { value: "wp-login.php" } });

    await waitFor(() => {
      expect(screen.getByTestId("virtualized-log-container")).toBeInTheDocument();
    });
  });

  it("toggles Live Tail mode", async () => {
    renderWithProviders(<LogViewerPage />);

    await waitFor(() => {
      expect(screen.getByTestId("btn-toggle-live-mode")).toBeInTheDocument();
    });

    fireEvent.click(screen.getByTestId("btn-toggle-live-mode"));

    await waitFor(() => {
      expect(screen.getByTestId("badge-live-tail-status")).toHaveTextContent(/Live Stream Active|লাইভ স্ট্রিম সক্রিয়/i);
    });

    // Toggle off
    fireEvent.click(screen.getByTestId("btn-toggle-live-mode"));

    await waitFor(() => {
      expect(screen.getByTestId("badge-live-tail-status")).toHaveTextContent(/Live Stream Paused|লাইভ স্ট্রিম স্থগিত/i);
    });
  });
});
