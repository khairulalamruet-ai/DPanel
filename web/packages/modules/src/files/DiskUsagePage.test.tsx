import React from "react";
import { describe, it, expect, vi } from "vitest";
import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import { ToastProvider } from "@builtpanel/ui";
import { DiskUsagePage } from "./DiskUsagePage";

function renderWithProviders(ui: React.ReactElement) {
  return render(<ToastProvider>{ui}</ToastProvider>);
}

describe("DiskUsagePage (T08.10)", () => {
  it("renders page title, quota metric cards, and recalculate button", async () => {
    renderWithProviders(<DiskUsagePage />);

    await waitFor(() => {
      expect(screen.getByTestId("disk-usage-title")).toBeInTheDocument();
    });

    expect(screen.getByTestId("btn-recalculate-disk")).toBeInTheDocument();
    expect(screen.getByTestId("card-disk-quota")).toBeInTheDocument();
    expect(screen.getByTestId("card-inodes-quota")).toBeInTheDocument();

    await waitFor(() => {
      expect(screen.getByTestId("metric-disk-used")).toHaveTextContent("1.2 GB");
      expect(screen.getByTestId("metric-inodes-used")).toHaveTextContent("12,450");
    });
  });

  it("renders exactly 10 largest folders meeting DoD requirement", async () => {
    renderWithProviders(<DiskUsagePage />);

    await waitFor(() => {
      expect(screen.getByTestId("top-directories-heading")).toBeInTheDocument();
    });

    // Check all 10 rows exist (#1 to #10)
    for (let i = 0; i < 10; i++) {
      expect(screen.getByTestId(`top-dir-row-${i}`)).toBeInTheDocument();
    }

    // Check row #0 details
    expect(screen.getByTestId("dir-path-0")).toHaveTextContent("/public_html/wp-content/uploads");
    expect(screen.getByTestId("dir-size-0")).toHaveTextContent("450.0 MB");

    // Check row #9 details
    expect(screen.getByTestId("dir-path-9")).toHaveTextContent("/tmp");
    expect(screen.getByTestId("dir-size-9")).toHaveTextContent("1.0 MB");
  });

  it("triggers recalculate disk usage action", async () => {
    renderWithProviders(<DiskUsagePage />);

    await waitFor(() => {
      expect(screen.getByTestId("btn-recalculate-disk")).toBeInTheDocument();
    });

    fireEvent.click(screen.getByTestId("btn-recalculate-disk"));

    await waitFor(() => {
      expect(screen.getByTestId("metric-disk-used")).toBeInTheDocument();
    });
  });
});
