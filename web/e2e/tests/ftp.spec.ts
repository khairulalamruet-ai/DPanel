import { test, expect } from "@playwright/test";

test.describe("FTP Accounts Management UI (T08.13)", () => {
  test.beforeEach(async ({ page }) => {
    // Navigate directly to FTP accounts page
    await page.goto("/ftp");
  });

  test("loads FTP accounts page with connection settings and accounts list", async ({ page }) => {
    // Page Title
    await expect(page.getByTestId("ftp-page-title")).toBeVisible();

    // Connection settings card
    await expect(page.getByTestId("ftp-connection-card")).toBeVisible();
    await expect(page.getByTestId("ftp-info-host")).toContainText("ftp.demo.builtpanel.test");
    await expect(page.getByTestId("ftp-info-port")).toContainText("21");
    await expect(page.getByTestId("ftp-info-protocol")).toContainText("FTPS");

    // Copy buttons
    await expect(page.getByTestId("btn-copy-host")).toBeVisible();
    await expect(page.getByTestId("btn-copy-port")).toBeVisible();

    // Seed accounts
    await expect(page.getByTestId("ftp-username-demo_deploy")).toBeVisible();
    await expect(page.getByTestId("ftp-username-demo_backup")).toBeVisible();
  });

  test("DoD E2E: creates a new FTP account, modifies password and quota, and deletes account", async ({ page }) => {
    // Handle window.confirm for delete
    page.on("dialog", (dialog) => dialog.accept());

    // 1. Create a new FTP account
    await page.getByTestId("btn-open-create-ftp").click();
    await expect(page.getByTestId("create-ftp-dialog-title")).toBeVisible();

    const usernameSuffix = "e2etest";
    const fullUsername = "demo_e2etest";

    await page.getByTestId("input-ftp-username").fill(usernameSuffix);
    await page.getByTestId("btn-generate-ftp-pw").click();

    // Submit
    await page.getByTestId("btn-submit-create-ftp").click();

    // Verify account created in table
    await expect(page.getByTestId(`ftp-username-${fullUsername}`)).toBeVisible();

    // 2. Change password
    await page.getByTestId(`btn-change-pw-${fullUsername}`).click();
    await expect(page.getByTestId("change-ftp-password-title")).toBeVisible();

    await page.getByTestId("input-new-ftp-password").fill("NewSecureFtpPass2026!");
    await page.getByTestId("btn-submit-change-ftp-pw").click();

    await expect(page.getByTestId("change-ftp-password-title")).not.toBeVisible();
    await expect(page.getByText(/Password changed successfully|পাসওয়ার্ড সফলভাবে পরিবর্তন/i)).toBeVisible();

    // 3. Change quota
    await page.getByTestId(`btn-change-quota-${fullUsername}`).click();
    await expect(page.getByTestId("change-ftp-quota-title")).toBeVisible();

    await page.getByTestId("edit-radio-quota-limited").click();
    await page.getByTestId("edit-input-ftp-quota-mb").fill("750");
    await page.getByTestId("btn-submit-change-quota").click();

    await expect(page.getByTestId("change-ftp-quota-title")).not.toBeVisible();
    await expect(page.getByText(/Quota updated successfully|কোটা সফলভাবে পরিবর্তন/i)).toBeVisible();

    // 4. Delete the account
    await page.getByTestId(`btn-delete-ftp-${fullUsername}`).click();
    await expect(page.getByTestId(`ftp-username-${fullUsername}`)).not.toBeVisible();
  });
});
