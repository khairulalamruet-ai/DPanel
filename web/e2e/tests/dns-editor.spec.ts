import { test, expect } from "@playwright/test";

test.describe("DNS Zone Editor UI (T05.11)", () => {
  test.beforeEach(async ({ page }) => {
    // Navigate directly to the DNS editor page
    await page.goto("/dns");
  });

  test("loads DNS editor page with nameserver banner, zone selector, and initial records", async ({
    page,
  }) => {
    // Page Header
    await expect(
      page.getByRole("heading", { level: 1, name: /DNS Zone Editor|ডিএনএস জোন এডিটর/i })
    ).toBeVisible();

    // Zone Selector has alpha-hosting.com
    await expect(page.getByTestId("select-active-zone")).toBeVisible();

    // Nameserver Info Banner
    await expect(page.getByText(/ns1\.builtpanel\.net/)).toBeVisible();
    await expect(page.getByTestId("btn-verify-ns")).toBeVisible();

    // Table of initial records
    await expect(page.getByText("192.168.1.100")).toBeVisible();
    await expect(page.getByText("alpha-hosting.com.", { exact: true })).toBeVisible();
    await expect(page.getByText("mail.alpha-hosting.com.", { exact: true })).toBeVisible();
    await expect(page.getByText("v=spf1 a mx ~all")).toBeVisible();
  });

  test("filters records by record type pill and search query", async ({ page }) => {
    await expect(page.getByText("192.168.1.100")).toBeVisible();

    // Filter by MX type pill button
    await page.getByTestId("btn-filter-type-MX").click();

    // MX record is visible, A and CNAME are hidden
    await expect(page.getByText("mail.alpha-hosting.com.")).toBeVisible();
    await expect(page.getByText("192.168.1.100")).not.toBeVisible();

    // Reset to ALL
    await page.getByTestId("btn-filter-type-ALL").click();
    await expect(page.getByText("192.168.1.100")).toBeVisible();

    // Search keyword
    const searchInput = page.getByTestId("input-search-dns-records");
    await searchInput.fill("spf1");

    await expect(page.getByText("v=spf1 a mx ~all")).toBeVisible();
    await expect(page.getByText("192.168.1.100")).not.toBeVisible();
  });

  test("DoD: creates a new record through the form dialog", async ({ page }) => {
    await expect(page.getByText("192.168.1.100")).toBeVisible();

    // Click Add Record
    await page.getByTestId("btn-add-dns-record").click();

    // Dialog appears
    await expect(page.getByTestId("btn-submit-dns-record")).toBeVisible();

    // Fill form
    await page.getByTestId("input-record-name").fill("e2e-api");
    await page.getByTestId("input-record-content").fill("192.168.1.240");

    // Submit
    await page.getByTestId("btn-submit-dns-record").click();

    // Verify new record appears in table
    await expect(page.getByText("192.168.1.240")).toBeVisible();
    await expect(page.getByText("e2e-api", { exact: true })).toBeVisible();
  });

  test("DoD: performs inline editing on a DNS record", async ({ page }) => {
    await expect(page.getByText("192.168.1.100")).toBeVisible();

    // Click inline edit on rec_01
    await page.getByTestId("btn-edit-record-rec_01").click();

    // Inline row is active
    await expect(page.getByTestId("inline-edit-row-rec_01")).toBeVisible();

    // Edit content
    const contentInput = page.getByTestId("input-inline-content-rec_01");
    await contentInput.fill("192.168.1.205");

    // Click inline save (Check)
    await page.getByTestId("btn-save-inline-rec_01").click();

    // Record updated in table
    await expect(page.getByText("192.168.1.205")).toBeVisible();
    await expect(page.getByTestId("inline-edit-row-rec_01")).not.toBeVisible();
  });

  test("DoD: exports and imports BIND zone file", async ({ page }) => {
    await expect(page.getByText("192.168.1.100")).toBeVisible();

    // Click BIND Zone button
    await page.getByTestId("btn-bind-export-import").click();

    // Dialog appears with exported BIND zone text
    await expect(page.getByTestId("pre-bind-export")).toBeVisible();
    await expect(page.getByTestId("pre-bind-export")).toContainText("$ORIGIN alpha-hosting.com.");

    // Switch to Import Tab
    await page.getByTestId("tab-bind-import").click();
    await expect(page.getByTestId("textarea-bind-import")).toBeVisible();

    // Fill BIND records
    await page
      .getByTestId("textarea-bind-import")
      .fill("portal IN A 192.168.1.215\ncdn IN CNAME alpha-hosting.com.");

    // Submit import
    await page.getByTestId("btn-submit-bind-import").click();

    // Success notice displayed
    await expect(page.getByTestId("bind-import-success-notice")).toBeVisible();
  });
});
