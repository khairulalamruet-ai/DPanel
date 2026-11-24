import { test, expect } from "@playwright/test";

test.describe("Settings UI (T03.13)", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/settings");
  });

  test("loads platform settings page with module tabs", async ({ page }) => {
    // Verify page header
    await expect(page.getByText(/Platform Settings|প্ল্যাটফর্ম সেটিংস/i).first()).toBeVisible();

    // Verify module tabs
    await expect(page.getByRole("button", { name: /General|সাধারণ/i })).toBeVisible();
    await expect(page.getByRole("button", { name: /Security|নিরাপত্তা/i })).toBeVisible();
    await expect(page.getByRole("button", { name: /Backups|ব্যাকআপ/i })).toBeVisible();
    await expect(page.getByRole("button", { name: /Notifications|বিজ্ঞপ্তি/i })).toBeVisible();
  });

  test("displays dynamic JSON schema form fields on General tab", async ({ page }) => {
    await expect(page.getByLabel(/Panel Name/i)).toBeVisible();
    await expect(page.getByLabel(/Support Contact Email/i)).toBeVisible();
    await expect(page.getByLabel(/Session Timeout/i)).toBeVisible();
    await expect(page.getByLabel(/Maintenance Mode/i)).toBeVisible();
    await expect(page.getByLabel(/Default Language/i)).toBeVisible();
  });

  test("switches between module tabs and renders corresponding schema forms", async ({ page }) => {
    // Switch to Security
    await page.getByRole("button", { name: /Security|নিরাপত্তা/i }).click();
    await expect(page.getByLabel(/Minimum Password Length/i)).toBeVisible();
    await expect(page.getByLabel(/Max Failed Login Attempts/i)).toBeVisible();

    // Switch to Backups
    await page.getByRole("button", { name: /Backups|ব্যাকআপ/i }).click();
    await expect(page.getByLabel(/Retention Period/i)).toBeVisible();
    await expect(page.getByLabel(/Compression Algorithm/i)).toBeVisible();

    // Switch to Notifications
    await page.getByRole("button", { name: /Notifications|বিজ্ঞপ্তি/i }).click();
    await expect(page.getByLabel(/Alert Recipient Email/i)).toBeVisible();
  });

  test("updates settings and receives success feedback", async ({ page }) => {
    const panelNameInput = page.getByLabel(/Panel Name/i);
    await expect(panelNameInput).toBeVisible();

    await panelNameInput.fill("Built Panel Enterprise Cloud");

    const saveBtn = page.getByRole("button", { name: /Save Changes|পরিবর্তন সংরক্ষণ করুন/i });
    await saveBtn.click();

    // Verify success toast notification
    await expect(
      page.getByText(/Settings for 'General' updated successfully|সেটিংস সফলভাবে সংরক্ষিত হয়েছে/i)
    ).toBeVisible();
  });
});
