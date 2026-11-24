import { test, expect } from "@playwright/test";

test.describe("Branding & Live Theming (T18.06)", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/admin/branding");
  });

  test("loads branding settings page and live interactive preview", async ({ page }) => {
    await expect(page.getByTestId("branding-page")).toBeVisible();
    await expect(page.getByText(/Branding & White Labeling|ব্র্যান্ডিং ও হোয়াইট লেবেলিং/i).first()).toBeVisible();

    // Verify key inputs
    await expect(page.getByTestId("branding-name-input")).toBeVisible();
    await expect(page.getByTestId("branding-logo-input")).toBeVisible();
    await expect(page.getByTestId("branding-favicon-input")).toBeVisible();
    await expect(page.getByTestId("branding-accent-input")).toBeVisible();
    await expect(page.getByTestId("branding-login-title-input")).toBeVisible();
    await expect(page.getByTestId("branding-login-subtitle-input")).toBeVisible();
    await expect(page.getByTestId("branding-nameservers-input")).toBeVisible();
    await expect(page.getByTestId("branding-css-input")).toBeVisible();

    // Verify live preview card
    await expect(page.getByTestId("branding-live-preview")).toBeVisible();
    await expect(page.getByTestId("preview-brand-name")).toBeVisible();
  });

  test("DoD: updating branding immediately updates navbar and CSS live without page reload", async ({ page }) => {
    // Wait for initial fetch to complete and settle
    const nameInput = page.getByTestId("branding-name-input");
    await expect(nameInput).toHaveValue("Built Panel");

    // Set a window variable to strictly prove no page reload occurred
    await page.evaluate(() => {
      (window as any).__NO_RELOAD_TEST_MARKER__ = "ACTIVE_SESSION_WITHOUT_RELOAD";
    });

    await nameInput.fill("Hyperion Cloud Panel");

    // Add sanitized custom CSS
    const cssInput = page.getByTestId("branding-css-input");
    await cssInput.fill(".builtpanel-custom-branding-test { border: 4px dashed rgb(255, 0, 0); }");

    // Live preview updates immediately while typing
    await expect(page.getByTestId("preview-brand-name")).toHaveText("Hyperion Cloud Panel");

    // Save & apply
    const saveBtn = page.getByTestId("branding-save-btn");
    await saveBtn.click();

    // Toast confirmation
    await expect(
      page.getByText(/Branding Updated|ব্র্যান্ডিং হালনাগাদ হয়েছে/i).first()
    ).toBeVisible();

    // Verify navbar title updated live in the AppShell DOM
    const appBrandTitle = page.getByTestId("app-brand-title");
    await expect(appBrandTitle).toHaveText("Hyperion Cloud Panel");

    // Verify injected style tag in DOM
    const liveStyleTag = page.locator("#builtpanel-live-custom-css");
    await expect(liveStyleTag).toBeAttached();
    const styleContent = await liveStyleTag.innerHTML();
    expect(styleContent).toContain(".builtpanel-custom-branding-test");

    // Verify window marker is still intact (NO reload was triggered)
    const markerValue = await page.evaluate(() => (window as any).__NO_RELOAD_TEST_MARKER__);
    expect(markerValue).toBe("ACTIVE_SESSION_WITHOUT_RELOAD");
  });

  test("sanitizes custom CSS to neutralize script tags and javascript pseudo-protocols", async ({ page }) => {
    // Wait for initial fetch to complete and settle
    await expect(page.getByTestId("branding-name-input")).toHaveValue("Built Panel");

    const cssInput = page.getByTestId("branding-css-input");
    await cssInput.fill("</style><script>alert('xss')</script>body { background: url('javascript:void(0)'); }");

    // Warning alert should be shown in UI
    await expect(
      page.getByText(/Dangerous tags\/scripts\/imports will be stripped|নিরাপত্তার স্বার্থে ক্ষতিকারক ট্যাগ/i).first()
    ).toBeVisible();

    const saveBtn = page.getByTestId("branding-save-btn");
    await saveBtn.click();

    // Verify the injected style element in DOM does NOT contain <script> or javascript:
    const liveStyleTag = page.locator("#builtpanel-live-custom-css");
    await expect(liveStyleTag).toBeAttached();
    const styleContent = await liveStyleTag.innerHTML();
    expect(styleContent).not.toContain("<script>");
    expect(styleContent).not.toContain("alert('xss')");
    expect(styleContent).not.toContain("javascript:");
    expect(styleContent).toContain("blocked-js:");
  });
});
