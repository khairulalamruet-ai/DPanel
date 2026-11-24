import React from "react";
import { describe, it, expect, beforeEach } from "vitest";
import { render, screen, waitFor, fireEvent, within } from "@testing-library/react";
import { ToastProvider } from "@builtpanel/ui";
import { AdminIpPoolsPage } from "./AdminIpPoolsPage";

function renderWithProviders(ui: React.ReactElement) {
  return render(<ToastProvider>{ui}</ToastProvider>);
}

describe("AdminIpPoolsPage (T16.05)", () => {
  it("renders IP metrics and configured IP addresses across nodes", async () => {
    renderWithProviders(<AdminIpPoolsPage />);

    expect(screen.getByRole("heading", { level: 1 })).toBeInTheDocument();

    await waitFor(() => {
      expect(screen.getByText("167.104.101.81")).toBeInTheDocument();
    });

    expect(screen.getByText("167.104.101.82")).toBeInTheDocument();
    expect(screen.getByText("167.104.101.95")).toBeInTheDocument();
    expect(screen.getByText("167.104.101.85")).toBeInTheDocument();

    // Verify nodes are listed
    expect(screen.getAllByText("node-master-01").length).toBeGreaterThan(0);
    expect(screen.getAllByText("node-worker-02").length).toBeGreaterThan(0);

    // Verify metrics
    expect(screen.getByTestId("metric-total-ips")).toHaveTextContent("6");
  });

  it("filters IP list by node selector", async () => {
    renderWithProviders(<AdminIpPoolsPage />);

    await waitFor(() => {
      expect(screen.getByText("167.104.101.81")).toBeInTheDocument();
    });

    const nodeFilter = screen.getByTestId("filter-node-select");
    fireEvent.change(nodeFilter, {
      target: { value: "00000000-0000-0000-0000-0000000000bb" },
    });

    await waitFor(() => {
      expect(screen.getByText("167.104.101.85")).toBeInTheDocument();
    });
    expect(screen.queryByText("167.104.101.81")).not.toBeInTheDocument();
  });

  it("allocates an available dedicated IP to a tenant (DoD T16.05: IP বরাদ্দ করা)", async () => {
    renderWithProviders(<AdminIpPoolsPage />);

    await waitFor(() => {
      expect(screen.getByText("167.104.101.96")).toBeInTheDocument();
    });

    // ip_04 is available dedicated IP
    const allocateBtn = screen.getByTestId("btn-allocate-ip-ip_04");
    expect(allocateBtn).toBeInTheDocument();
    fireEvent.click(allocateBtn);

    const dialog = await screen.findByRole("dialog");
    expect(dialog).toBeInTheDocument();

    // Select tenant
    const tenantSelect = within(dialog).getByTestId("select-allocate-tenant");
    fireEvent.change(tenantSelect, { target: { value: "usr_0001" } });

    // Optional domain
    const domainInput = within(dialog).getByTestId("input-allocate-domain");
    fireEvent.change(domainInput, { target: { value: "vip.alexstore.com" } });

    // Confirm allocation
    const confirmBtn = within(dialog).getByTestId("btn-confirm-allocate-ip");
    fireEvent.click(confirmBtn);

    // Verify IP is now assigned to tenant
    await waitFor(() => {
      expect(screen.getByTestId("assigned-tenant-ip_04")).toHaveTextContent("@alpha");
    });
    expect(screen.getByText("vip.alexstore.com")).toBeInTheDocument();
  });

  it("releases an assigned dedicated IP back to node pool (DoD T16.05: IP মুক্ত করা)", async () => {
    renderWithProviders(<AdminIpPoolsPage />);

    await waitFor(() => {
      expect(screen.getByText("167.104.101.95")).toBeInTheDocument();
    });

    // Mock confirm
    window.confirm = () => true;

    // ip_03 is assigned to @alex
    expect(screen.getByTestId("assigned-tenant-ip_03")).toHaveTextContent("@alex");

    const releaseBtn = screen.getByTestId("btn-release-ip-ip_03");
    fireEvent.click(releaseBtn);

    // Verify IP is released back to available pool
    await waitFor(() => {
      expect(screen.getByTestId("unassigned-label-ip_03")).toBeInTheDocument();
    });
    expect(screen.queryByTestId("assigned-tenant-ip_03")).not.toBeInTheDocument();
  });

  it("adds a new IP address to a node pool via dialog", async () => {
    renderWithProviders(<AdminIpPoolsPage />);

    await waitFor(() => {
      expect(screen.getByText("167.104.101.81")).toBeInTheDocument();
    });

    const addBtn = screen.getByTestId("btn-add-ip");
    fireEvent.click(addBtn);

    const dialog = await screen.findByRole("dialog");
    expect(dialog).toBeInTheDocument();

    const ipInput = within(dialog).getByTestId("input-ip-address");
    fireEvent.change(ipInput, { target: { value: "167.104.101.120" } });

    const typeSelect = within(dialog).getByTestId("select-ip-type");
    fireEvent.change(typeSelect, { target: { value: "dedicated" } });

    const saveBtn = within(dialog).getByTestId("btn-save-ip");
    fireEvent.click(saveBtn);

    await waitFor(() => {
      expect(screen.getByText("167.104.101.120")).toBeInTheDocument();
    });
  });
});
