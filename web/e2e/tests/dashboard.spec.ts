import { test, expect } from "@playwright/test";

test.describe("Admin Dashboard & Layout Customization (T18.01)", () => {
  test("Admin: views metrics widgets, customizes widget order & visibility, and verifies layout persistence across reload", async ({
    page,
  }) => {
    // 1. Navigate to Admin Dashboard
    await page.goto("/");

    await expect(page.getByTestId("dashboard-page-title")).toBeVisible();
    await expect(page.getByTestId("dashboard-widgets-grid")).toBeVisible();

    // 2. Verify all 6 initial widgets are rendered
    await expect(page.getByTestId("widget-nodes_status")).toBeVisible();
    await expect(page.getByTestId("stat-nodes-online")).toContainText("3 / 3");

    await expect(page.getByTestId("widget-tenants_count")).toBeVisible();
    await expect(page.getByTestId("stat-tenants-count")).toContainText("24");

    await expect(page.getByTestId("widget-resource_usage")).toBeVisible();
    await expect(page.getByTestId("widget-recent_operations")).toBeVisible();

    await expect(page.getByTestId("widget-security_alerts")).toBeVisible();
    await expect(page.getByTestId("stat-blocked-threats")).toContainText("142");

    await expect(page.getByTestId("widget-license_status")).toBeVisible();
    await expect(page.getByTestId("stat-license-tier")).toContainText("Built Panel Enterprise");

    // 3. Open Layout Customization Dialog
    const customizeBtn = page.getByTestId("btn-customize-dashboard");
    await customizeBtn.click();

    await expect(page.getByTestId("dashboard-layout-dialog")).toBeVisible();
    await expect(page.getByTestId("layout-widget-list")).toBeVisible();

    // 4. Toggle visibility: Hide security alerts widget
    const secCheckbox = page.getByTestId("checkbox-widget-security_alerts");
    await secCheckbox.click();

    // 5. Reorder: Move nodes widget down
    const moveDownBtn = page.getByTestId("btn-move-down-nodes_status");
    await moveDownBtn.click();

    // Apply layout
    const saveLayoutBtn = page.getByTestId("btn-save-dashboard-layout");
    await saveLayoutBtn.click();

    await expect(page.getByTestId("dashboard-layout-dialog")).not.toBeVisible();

    // Verify security alerts widget is now hidden
    await expect(page.getByTestId("widget-security_alerts")).not.toBeVisible();
    await expect(page.getByTestId("widget-nodes_status")).toBeVisible();

    // 6. Persistence check: Reload page (T18.01 DoD: Widget-এর অবস্থান সংরক্ষিত থাকে)
    await page.reload();

    await expect(page.getByTestId("dashboard-widgets-grid")).toBeVisible();
    await expect(page.getByTestId("widget-security_alerts")).not.toBeVisible();
    await expect(page.getByTestId("widget-nodes_status")).toBeVisible();

    // 7. Reset to default layout
    await page.getByTestId("btn-customize-dashboard").click();
    await expect(page.getByTestId("dashboard-layout-dialog")).toBeVisible();

    await page.getByTestId("btn-reset-dashboard-layout").click();
    await expect(page.getByTestId("dashboard-layout-dialog")).not.toBeVisible();

    // Verify security alerts is restored
    await expect(page.getByTestId("widget-security_alerts")).toBeVisible();
  });
});
