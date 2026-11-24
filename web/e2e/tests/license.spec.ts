import { test, expect } from "@playwright/test";

test.describe("License Authority & Entitlement UI (T17.13)", () => {
  test("Admin: inspects license status, toggles key reveal, revalidates, activates new key, and submits recovery request", async ({
    page,
  }) => {
    // 1. Navigate to License page
    await page.goto("/admin/license");

    await expect(page.getByTestId("license-page-title")).toBeVisible();
    await expect(page.getByTestId("license-banner-active")).toBeVisible();
    await expect(page.getByText("Ed25519 Valid")).toBeVisible();

    // 2. License Details & Identity Card
    await expect(page.getByTestId("license-details-card")).toBeVisible();
    await expect(page.getByTestId("text-installation-id")).toContainText("8c4d9102");
    await expect(page.getByTestId("text-customer-name")).toContainText("Apex Cloud Services LLC");
    await expect(page.getByTestId("text-product-name")).toContainText("Built Panel Enterprise Edition");

    // Mask / Unmask key
    const keyDisplay = page.getByTestId("text-license-key");
    await expect(keyDisplay).toContainText("•");
    await page.getByTestId("btn-toggle-show-key").click();
    await expect(keyDisplay).toContainText("DHP-ENT-8492-9182-AA19");

    // Copy actions
    await expect(page.getByTestId("btn-copy-license-key")).toBeVisible();
    await expect(page.getByTestId("btn-copy-installation-id")).toBeVisible();

    // 3. Entitlements & Feature Gates
    await expect(page.getByTestId("license-entitlements-card")).toBeVisible();
    await expect(page.getByTestId("quota-bar-nodes")).toBeVisible();
    await expect(page.getByTestId("quota-bar-tenants")).toBeVisible();
    await expect(page.getByTestId("quota-bar-domains")).toBeVisible();
    await expect(page.getByTestId("feature-item-reseller_enabled")).toBeVisible();
    await expect(page.getByTestId("feature-item-clustering_enabled")).toBeVisible();
    await expect(page.getByTestId("feature-item-commercial_waf")).toBeVisible();

    // 4. Revalidate action
    const revalidateBtn = page.getByTestId("btn-revalidate-card");
    await revalidateBtn.click();
    await expect(
      page.getByText(/License Validated|লাইসেন্স যাচাই সম্পন্ন/i).first()
    ).toBeVisible();

    // 5. Activate New License Key
    const changeKeyBtn = page.getByTestId("btn-change-license-key");
    await changeKeyBtn.click();

    await expect(page.getByTestId("activate-license-dialog")).toBeVisible();

    // Attempt invalid key without DHP- prefix
    await page.getByTestId("input-license-key").fill("BADKEY-12345");
    await page.getByTestId("btn-submit-activate").click();
    await expect(page.getByTestId("activate-error-message")).toBeVisible();

    // Enter valid commercial key
    await page.getByTestId("input-license-key").fill("DHP-ENT-9021-4412-BB20");
    await page.getByTestId("btn-submit-activate").click();

    await expect(
      page.getByText(/License Activated|লাইসেন্স সক্রিয় হয়েছে/i).first()
    ).toBeVisible();
    await expect(page.getByTestId("activate-license-dialog")).not.toBeVisible();

    // 6. Emergency Recovery Request
    const recoveryBtn = page.getByTestId("btn-recovery-card");
    await recoveryBtn.click();

    await expect(page.getByTestId("recovery-request-dialog")).toBeVisible();
    await page.getByTestId("input-recovery-email").fill("devops@apexcloud.io");
    await page.getByTestId("input-recovery-notes").fill("Replacing faulty NVMe drive in primary datacenter.");
    await page.getByTestId("btn-submit-recovery").click();

    await expect(page.getByTestId("recovery-success-view")).toBeVisible();
    await expect(page.getByTestId("text-recovery-ticket-id")).toContainText("REC-");

    await page.getByTestId("btn-close-recovery-success").click();
    await expect(page.getByTestId("recovery-request-dialog")).not.toBeVisible();
  });
});
