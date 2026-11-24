import { test, expect } from "@playwright/test";

test.describe("Built Panel Admin Application", () => {
  test("loads the admin dashboard with branding and language toggle", async ({ page }) => {
    await page.goto("/");
    // Check page title and header
    await expect(page).toHaveTitle(/Built Panel Admin/);

    // Verify System Health card is displayed
    await expect(page.getByText(/0.1.0/)).toBeVisible();

    // Check language toggle button exists
    const langBtn = page.getByRole("button", { name: /ভাষা|Language/i });
    await expect(langBtn).toBeVisible();

    // Toggle language and verify
    await langBtn.click();
    await expect(page.getByText(/Dashboard|ড্যাশবোর্ড/i).first()).toBeVisible();
  });
});
