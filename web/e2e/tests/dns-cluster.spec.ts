import { test, expect } from "@playwright/test";

test.describe("DNS Cluster Management UI (T16.04)", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/admin/dns-cluster");
  });

  test("loads DNS cluster overview cards and server list", async ({ page }) => {
    await expect(page.getByText(/DNS Cluster Management|ডিএনএস ক্লাস্টার ব্যবস্থাপনা/i).first()).toBeVisible();

    // Verify initial nameservers in table
    await expect(page.locator("table").getByText("Primary PowerDNS US-East")).toBeVisible();
    await expect(page.locator("table").getByText("Secondary PowerDNS EU-Central")).toBeVisible();
    await expect(page.locator("table").getByText("ns1.builtpanel.internal")).toBeVisible();
  });

  test("DoD T16.04: adds a new DNS server to the cluster", async ({ page }) => {
    const addBtn = page.getByTestId("btn-add-dns-server");
    await addBtn.click();

    const dialog = page.getByRole("dialog");
    await expect(dialog).toBeVisible();

    // Fill form
    await dialog.getByTestId("input-dns-name").fill("APAC High-Availability DNS");
    await dialog.getByTestId("input-dns-hostname").fill("ns5.builtpanel.net");
    await dialog.getByTestId("input-dns-ip").fill("203.0.113.88");

    // Select Secondary role
    await dialog.getByTestId("select-dns-role").selectOption("secondary");

    // Submit
    const saveBtn = dialog.getByTestId("btn-save-dns-server");
    await saveBtn.click();

    await expect(dialog).not.toBeVisible();

    // Verify new server is listed in the table
    const row = page.locator("tr", { hasText: "APAC High-Availability DNS" });
    await expect(row).toBeVisible();
    await expect(row.getByText("ns5.builtpanel.net")).toBeVisible();
  });

  test("DoD T16.04: resolves domain query even when a DNS server is offline", async ({ page }) => {
    // Open failover & resolution test modal
    const testBtn = page.getByTestId("btn-open-dns-failover-test");
    await testBtn.click();

    const dialog = page.getByRole("dialog");
    await expect(dialog).toBeVisible();

    // Select Secondary server to simulate outage
    const outageSelect = dialog.getByTestId("select-simulate-down-server");
    await outageSelect.selectOption("dns_srv_02");

    // Run resolution test
    const runBtn = dialog.getByTestId("btn-run-dns-resolution-test");
    await runBtn.click();

    // Verify DoD: Resolution succeeds via surviving node
    const resultBox = dialog.getByTestId("dns-test-result-box");
    await expect(resultBox).toBeVisible();
    await expect(dialog.getByTestId("failover-status-badge")).toBeVisible();
    await expect(dialog.getByTestId("answering-server-name")).toContainText("Primary PowerDNS US-East");
    await expect(resultBox.getByText("93.184.216.34")).toBeVisible();
  });
});
