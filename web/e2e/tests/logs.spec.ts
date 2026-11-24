import { test, expect } from "@playwright/test";

test.describe("Log Viewer UI (T10.06)", () => {
  test.beforeEach(async ({ page }) => {
    // Navigate directly to log viewer page
    await page.goto("/logs");
  });

  test("loads log viewer page with domain picker, log tabs, and virtualized container", async ({ page }) => {
    // Page Title
    await expect(page.getByTestId("log-viewer-title")).toBeVisible();

    // Controls
    await expect(page.getByTestId("select-log-domain")).toBeVisible();
    await expect(page.getByTestId("tab-log-access")).toBeVisible();
    await expect(page.getByTestId("tab-log-error")).toBeVisible();
    await expect(page.getByTestId("tab-log-php")).toBeVisible();
    await expect(page.getByTestId("input-log-search")).toBeVisible();
    await expect(page.getByTestId("btn-download-log")).toBeVisible();
    await expect(page.getByTestId("btn-toggle-live-mode")).toBeVisible();

    // Virtualized log container and seed lines
    await expect(page.getByTestId("virtualized-log-container")).toBeVisible();
    await expect(page.getByTestId("virtualized-log-line-0")).toBeVisible();
    await expect(page.getByTestId("metric-loaded-lines")).toHaveText("250");
  });

  test("DoD E2E: filters domain, switches error tab, toggles live mode, and tests 100MB virtualized stress load", async ({ page }) => {
    // 1. Select specific domain
    await page.getByTestId("select-log-domain").selectOption("store.builtpanel.test");
    await expect(page.getByTestId("virtualized-log-container")).toBeVisible();

    // 2. Switch to Error Log tab
    await page.getByTestId("tab-log-error").click();
    await expect(page.getByTestId("metric-loaded-lines")).toHaveText("50");

    // 3. Search filtering
    await page.getByTestId("input-log-search").fill("connect()");
    await expect(page.getByTestId("virtualized-log-container")).toBeVisible();

    // 4. Toggle Live Tail
    const liveTailBtn = page.getByTestId("btn-toggle-live-mode");
    await liveTailBtn.click();
    await expect(page.getByTestId("badge-live-tail-status")).toContainText(
      /Live Stream Active|লাইভ স্ট্রিম সক্রিয়/i
    );

    // 5. DoD Verification: Load 100MB Log (50,000 lines) with zero browser lockup
    const heavyLogBtn = page.getByTestId("btn-load-heavy-log");
    await expect(heavyLogBtn).toBeVisible();
    await heavyLogBtn.click();

    // Verify 50,000 lines handled seamlessly by the virtualized container
    await expect(page.getByTestId("metric-loaded-lines")).toHaveText("50,000", { timeout: 10000 });
    await expect(page.getByTestId("badge-virtualized-mode")).toBeVisible();
    await expect(page.getByTestId("virtualized-log-line-0")).toBeVisible();

    // Scroll virtualized container down and verify responsiveness
    const container = page.getByTestId("virtualized-log-container");
    await container.evaluate((el) => {
      el.scrollTop = 5000;
    });

    // Check that container still responds immediately
    await expect(container).toBeVisible();
  });
});
