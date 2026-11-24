import React from "react";
import { describe, it, expect, beforeEach } from "vitest";
import { render, screen, waitFor, fireEvent, within } from "@testing-library/react";
import { ToastProvider } from "@builtpanel/ui";
import { resetMockDns } from "@builtpanel/api";
import { AdminDnsClusterPage } from "./AdminDnsClusterPage";

function renderWithProviders(ui: React.ReactElement) {
  return render(<ToastProvider>{ui}</ToastProvider>);
}

describe("AdminDnsClusterPage (T16.04)", () => {
  beforeEach(() => {
    resetMockDns();
  });

  it("renders cluster overview metrics and initial DNS servers", async () => {
    renderWithProviders(<AdminDnsClusterPage />);

    expect(screen.getByRole("heading", { level: 1 })).toBeInTheDocument();

    await waitFor(() => {
      expect(screen.getByText("Primary PowerDNS US-East")).toBeInTheDocument();
    });

    expect(screen.getByText("Secondary PowerDNS EU-Central")).toBeInTheDocument();
    expect(screen.getByText("Edge Replica PowerDNS AP-South")).toBeInTheDocument();

    // Verify hostnames
    expect(screen.getByText("ns1.builtpanel.internal")).toBeInTheDocument();
    expect(screen.getByText("ns2.builtpanel.internal")).toBeInTheDocument();

    // Verify metrics
    expect(screen.getByTestId("metric-total-dns-servers")).toHaveTextContent("3");
  });

  it("adds a new DNS server to the cluster via UI dialog", async () => {
    renderWithProviders(<AdminDnsClusterPage />);

    await waitFor(() => {
      expect(screen.getByText("Primary PowerDNS US-East")).toBeInTheDocument();
    });

    // Click "Add DNS Server"
    const addBtn = screen.getByTestId("btn-add-dns-server");
    fireEvent.click(addBtn);

    const dialog = await screen.findByRole("dialog");
    expect(dialog).toBeInTheDocument();

    // Fill form
    const nameInput = within(dialog).getByTestId("input-dns-name");
    fireEvent.change(nameInput, { target: { value: "Frankfurt Secondary DNS" } });

    const hostInput = within(dialog).getByTestId("input-dns-hostname");
    fireEvent.change(hostInput, { target: { value: "ns4.builtpanel.net" } });

    const ipInput = within(dialog).getByTestId("input-dns-ip");
    fireEvent.change(ipInput, { target: { value: "198.51.100.55" } });

    const roleSelect = within(dialog).getByTestId("select-dns-role");
    fireEvent.change(roleSelect, { target: { value: "secondary" } });

    // Submit
    const saveBtn = within(dialog).getByTestId("btn-save-dns-server");
    fireEvent.click(saveBtn);

    // Verify new server appears in the table
    await waitFor(() => {
      expect(screen.getByText("Frankfurt Secondary DNS")).toBeInTheDocument();
    });
    expect(screen.getByText("ns4.builtpanel.net")).toBeInTheDocument();
  });

  it("toggles server offline and verifies resolution succeeds from surviving nodes (DoD T16.04)", async () => {
    renderWithProviders(<AdminDnsClusterPage />);

    await waitFor(() => {
      expect(screen.getByText("Secondary PowerDNS EU-Central")).toBeInTheDocument();
    });

    // Toggle Secondary server offline
    const toggleBtn = screen.getByTestId("toggle-dns-status-dns_srv_02");
    fireEvent.click(toggleBtn);

    // Open resolution & failover test modal
    const testBtn = screen.getByTestId("btn-open-dns-failover-test");
    fireEvent.click(testBtn);

    const dialog = await screen.findByRole("dialog");
    expect(dialog).toBeInTheDocument();

    // Select dns_srv_02 as simulated down
    const outageSelect = within(dialog).getByTestId("select-simulate-down-server");
    fireEvent.change(outageSelect, { target: { value: "dns_srv_02" } });

    // Run resolution test
    const runBtn = within(dialog).getByTestId("btn-run-dns-resolution-test");
    fireEvent.click(runBtn);

    // Verify DoD: Even though a DNS server is offline, query resolves via surviving node
    await waitFor(() => {
      expect(within(dialog).getByTestId("dns-test-result-box")).toBeInTheDocument();
    });

    expect(within(dialog).getByTestId("failover-status-badge")).toBeInTheDocument();
    expect(within(dialog).getByTestId("answering-server-name")).toHaveTextContent(
      "Primary PowerDNS US-East"
    );
    expect(within(dialog).getByText(/93\.184\.216\.34/)).toBeInTheDocument();
  });
});
