import React from "react";
import { describe, it, expect } from "vitest";
import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import { ToastProvider } from "@builtpanel/ui";
import { AdminDatabasesPage } from "./AdminDatabasesPage";

function renderWithProviders(ui: React.ReactElement) {
  return render(<ToastProvider>{ui}</ToastProvider>);
}

describe("AdminDatabasesPage (T07.11)", () => {
  it("renders page title, summary metrics, and node distributions", async () => {
    renderWithProviders(<AdminDatabasesPage />);

    await waitFor(() => {
      expect(screen.getByTestId("admin-db-title")).toBeInTheDocument();
    });

    // Summary Metrics
    expect(screen.getByTestId("metric-total-dbs")).toHaveTextContent("48");
    expect(screen.getByTestId("metric-total-storage")).toHaveTextContent("1.85 GB");
    expect(screen.getByTestId("metric-total-users")).toHaveTextContent("52");
    expect(screen.getByTestId("metric-active-nodes")).toHaveTextContent("3");

    // DoD: Node-wise database count and size
    expect(screen.getByTestId("card-db-node-node_01")).toBeInTheDocument();
    expect(screen.getByTestId("node-count-node_01")).toHaveTextContent("24");
    expect(screen.getByTestId("node-size-node_01")).toHaveTextContent("1000.0 MB");

    expect(screen.getByTestId("card-db-node-node_02")).toBeInTheDocument();
    expect(screen.getByTestId("node-count-node_02")).toHaveTextContent("16");
    expect(screen.getByTestId("node-size-node_02")).toHaveTextContent("650.0 MB");

    expect(screen.getByTestId("card-db-node-node_03")).toBeInTheDocument();
    expect(screen.getByTestId("node-count-node_03")).toHaveTextContent("8");
    expect(screen.getByTestId("node-size-node_03")).toHaveTextContent("244.5 MB");
  });

  it("DoD: displays largest databases table with top consumption items", async () => {
    renderWithProviders(<AdminDatabasesPage />);

    await waitFor(() => {
      expect(screen.getByTestId("largest-db-row-ecomm_magento_prod")).toBeInTheDocument();
    });

    expect(screen.getByTestId("largest-db-row-portal_lms_db")).toBeInTheDocument();
    expect(screen.getByTestId("largest-db-row-crm_enterprise_leads")).toBeInTheDocument();
    expect(screen.getByTestId("largest-db-row-demo_wordpress")).toBeInTheDocument();

    expect(screen.getByTestId("db-size-ecomm_magento_prod")).toHaveTextContent("500.0 MB");
  });

  it("filters largest databases by search input and node selector", async () => {
    renderWithProviders(<AdminDatabasesPage />);

    await waitFor(() => {
      expect(screen.getByTestId("largest-db-row-ecomm_magento_prod")).toBeInTheDocument();
    });

    // Search for "magento"
    const searchInput = screen.getByTestId("input-search-largest-dbs");
    fireEvent.change(searchInput, { target: { value: "magento" } });

    expect(screen.getByTestId("largest-db-row-ecomm_magento_prod")).toBeInTheDocument();
    expect(screen.queryByTestId("largest-db-row-portal_lms_db")).not.toBeInTheDocument();

    // Clear search and filter by node_02
    fireEvent.change(searchInput, { target: { value: "" } });
    fireEvent.change(screen.getByTestId("select-filter-node"), { target: { value: "node_02" } });

    expect(screen.getByTestId("largest-db-row-crm_enterprise_leads")).toBeInTheDocument();
    expect(screen.queryByTestId("largest-db-row-ecomm_magento_prod")).not.toBeInTheDocument();
  });
});
