import { test, expect } from "@playwright/test";

test.describe("User Create Wizard (T04.16)", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/users/new");
  });

  test("DoD: completes full user creation wizard and provisions running container", async ({ page }) => {
    // Verify wizard page header
    await expect(page.getByText(/Create Tenant Account|নতুন টেন্যান্ট অ্যাকাউন্ট তৈরি/i).first()).toBeVisible();

    // Step 1: Account info
    const usernameInput = page.locator("#wizard-username");
    await usernameInput.fill("e2etenant");

    const emailInput = page.locator("#wizard-email");
    await emailInput.fill("e2etenant@example.com");

    const passwordInput = page.locator("#wizard-password");
    await passwordInput.fill("StrongPassword123!");

    // Advance to Step 2
    const nextBtn = page.getByRole("button", { name: /Next Step|পরবর্তী ধাপ/i });
    await nextBtn.click();

    // Step 2: Plan
    await expect(page.getByText(/Select Hosting Plan|হোস্টিং প্যাকেজ নির্বাচন/i)).toBeVisible();
    const proPlan = page.getByText("Pro Developer");
    await proPlan.click();
    await nextBtn.click();

    // Step 3: Node
    await expect(page.getByText(/Cluster Node Assignment|ক্লাস্টার নোড নির্ধারণ/i)).toBeVisible();
    await nextBtn.click();

    // Step 4: Primary Domain
    await expect(
      page.getByRole("heading", { level: 3, name: /Primary Domain|প্রাথমিক ডোমেইন/i })
    ).toBeVisible();
    const domainInput = page.locator("#wizard-domain");
    await domainInput.fill("e2etenant-site.com");
    await nextBtn.click();

    // Step 5: Review & Confirm
    await expect(
      page.getByRole("heading", {
        level: 3,
        name: /Review & Confirm Provisioning|পর্যালোচনা ও প্রভিশনিং নিশ্চিতকরণ/i,
      })
    ).toBeVisible();
    await expect(page.getByText("e2etenant", { exact: true })).toBeVisible();
    await expect(page.getByText("e2etenant-site.com", { exact: true })).toBeVisible();

    // Submit: Create User & Launch Container
    const confirmBtn = page.getByRole("button", {
      name: /Create User & Launch Container|অ্যাকাউন্ট তৈরি ও কন্টেইনার চালু করুন/i,
    });
    await confirmBtn.click();

    // Verify Operation Progress and DoD: "container running"
    await expect(page.getByText("container running")).toBeVisible({ timeout: 10000 });
    await expect(page.getByText("bp-tenant-e2etenant")).toBeVisible();
    await expect(
      page.getByText(/Tenant Created & Container Running!|টেন্যান্ট তৈরি সম্পন্ন এবং কন্টেইনার চলছে!/i)
    ).toBeVisible();
  });
});
