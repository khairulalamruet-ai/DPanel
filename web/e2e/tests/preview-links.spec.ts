import { test, expect } from "@playwright/test";

test.describe("Client Temporary Preview Link UI (T06.14)", () => {
  test.beforeEach(async ({ page }) => {
    // Navigate to client domains page
    await page.goto("/client/domains");
  });

  test("opens preview link dialog from domain row and generates temporary link", async ({ page }) => {
    // Wait for domains table to load
    await expect(page.getByTestId("btn-preview-link-dom_01")).toBeVisible();

    // Click preview link action on primary domain
    await page.getByTestId("btn-preview-link-dom_01").click();

    // Dialog opens
    await expect(page.getByRole("heading", { name: /Temporary Preview Link|অস্থায়ী প্রিভিউ লিংক/i })).toBeVisible();

    // Click 48 Hours TTL
    const ttl48 = page.getByTestId("btn-ttl-48");
    await expect(ttl48).toBeVisible();
    await ttl48.click();

    // Generate link
    const generateBtn = page.getByTestId("btn-generate-preview-link");
    await expect(generateBtn).toBeVisible();
    await generateBtn.click();

    // Verify active link state
    await expect(page.getByTestId("badge-preview-status-active")).toBeVisible();
    const input = page.getByTestId("input-preview-url");
    await expect(input).toBeVisible();
    await expect(input).toHaveValue(/https:\/\/prv_[a-z0-9]+\.preview\.builtpanel\.net/);

    // Verify copy button
    await expect(page.getByTestId("btn-copy-preview-url")).toBeVisible();

    // Close dialog
    await page.getByTestId("btn-close-preview-dialog").click();
  });

  test("DoD: displays expired state and rejection notice for elapsed preview link", async ({ page }) => {
    // Pre-seeded domain dom_02 has an expired preview link
    await expect(page.getByTestId("btn-preview-link-dom_02")).toBeVisible();
    await page.getByTestId("btn-preview-link-dom_02").click();

    // Verify expired warning banner is displayed
    await expect(page.getByTestId("banner-preview-expired")).toBeVisible();
    await expect(
      page.getByText(/Preview Link Expired|প্রিভিউ লিংকের মেয়াদ শেষ/i)
    ).toBeVisible();

    // Generate button is present to generate a fresh link
    await expect(page.getByTestId("btn-generate-preview-link")).toBeVisible();
  });

  test("revokes an active preview link", async ({ page }) => {
    await expect(page.getByTestId("btn-preview-link-dom_01")).toBeVisible();
    await page.getByTestId("btn-preview-link-dom_01").click();

    await expect(page.getByRole("dialog")).toBeVisible();
    const generateBtn = page.getByTestId("btn-generate-preview-link");
    const revokeBtn = page.getByTestId("btn-revoke-preview-link");
    await expect(generateBtn.or(revokeBtn)).toBeVisible({ timeout: 10000 });

    if (await generateBtn.isVisible()) {
      await generateBtn.click();
    }

    await expect(revokeBtn).toBeVisible({ timeout: 10000 });
    await revokeBtn.click();

    // Returns to initial generate state
    await expect(generateBtn).toBeVisible();
  });
});
