import { test, expect } from "@playwright/test";

test.describe("Node Management UI (T02.16)", () => {
  test.beforeEach(async ({ page }) => {
    // Navigate directly to the nodes page
    await page.goto("/nodes");
  });

  test("loads cluster nodes page and displays enrolled nodes", async ({ page }) => {
    // Verify header
    await expect(page.getByText(/Cluster Nodes|ক্লাস্টার নোডসমূহ/i).first()).toBeVisible();

    // Verify node items from mock data
    await expect(page.getByText("node-master-01").first()).toBeVisible();
    await expect(page.getByText("node-worker-02").first()).toBeVisible();

    // Verify status badges
    await expect(page.getByText(/Online|সচল/i).first()).toBeVisible();
  });

  test("opens Add Node wizard, inputs name, generates enrollment token, and displays install command", async ({ page }) => {
    // Click Add Node button
    const addBtn = page.getByRole("button", { name: /Add Node|নতুন নোড যুক্ত করুন/i });
    await expect(addBtn).toBeVisible();
    await addBtn.click();

    // Verify wizard dialog title
    await expect(page.getByText(/Enroll New Node|নতুন নোড অন্তর্ভুক্তকরণ/i)).toBeVisible();

    // Input node name
    const nameInput = page.locator("#node-name-input");
    await expect(nameInput).toBeVisible();
    await nameInput.fill("node-worker-04");

    // Click Generate Token
    const generateBtn = page.getByRole("button", { name: /Generate Enrollment Token|এনরোলমেন্ট টোকেন তৈরি করুন/i });
    await expect(generateBtn).toBeVisible();
    await generateBtn.click();

    // Verify token is displayed
    await expect(page.getByText("node-worker-04")).toBeVisible();

    // Verify command block appears
    const codeBlock = page.locator("pre code");
    await expect(codeBlock).toBeVisible();
    await expect(codeBlock).toContainText("curl -sSL https://get.builtpanel.com/agent.sh");

    // Click copy command button
    const copyBtn = page.getByRole("button", { name: /Copy Command|কমান্ড কপি করুন/i });
    await expect(copyBtn).toBeVisible();
    await copyBtn.click();

    // Verify copy feedback
    await expect(page.getByText(/Copied!|কমান্ড কপি হয়েছে!|কপি করা হয়েছে!/i)).toBeVisible();

    // Close wizard
    const doneBtn = page.getByRole("button", { name: /Done & Refresh|সম্পন্ন ও রিফ্রেশ/i });
    await doneBtn.click();
    await expect(page.getByText(/Enroll New Node|নতুন নোড অন্তর্ভুক্তকরণ/i)).not.toBeVisible();
  });

  test("opens node specifications dialog and displays system details", async ({ page }) => {
    const viewSpecsBtns = page.getByRole("button", { name: /View Specs|বিবরণ/i });
    await expect(viewSpecsBtns.first()).toBeVisible();
    await viewSpecsBtns.first().click();

    // Verify modal content
    await expect(page.getByText(/Docker Engine|ডকার ইঞ্জিন/i)).toBeVisible();
    await expect(page.getByText(/CPU Model|সিপিইউ মডেল/i)).toBeVisible();
    await expect(page.getByText(/Certificate Fingerprint|সার্টিফিকেট ফিঙ্গারপ্রিন্ট/i)).toBeVisible();

    // Close dialog
    const closeBtn = page.getByRole("button", { name: /Close|বন্ধ করুন/i });
    await closeBtn.click();
    await expect(page.getByText(/Docker Engine|ডকার ইঞ্জিন/i)).not.toBeVisible();
  });

  test("opens revoke confirmation dialog and requires typed node name", async ({ page }) => {
    const revokeBtns = page.getByRole("button", { name: /^Revoke$|^বাতিল করুন$/i });
    await expect(revokeBtns.first()).toBeVisible();
    await revokeBtns.first().click();

    // Verify revoke dialog
    await expect(page.getByText(/Revoke Node Certificate|নোড সার্টিফিকেট বাতিল করুন/i)).toBeVisible();

    // The confirm button should be disabled before typing
    const confirmBtn = page.getByRole("button", { name: /^Confirm$|^নোড বাতিল করুন$/i });
    await expect(confirmBtn).toBeDisabled();

    // Fill the node name
    const input = page.locator("input[placeholder='node-master-01']");
    await expect(input).toBeVisible();
    await input.fill("node-master-01");

    // The confirm button should now be enabled
    await expect(confirmBtn).toBeEnabled();
    await confirmBtn.click();

    // Verify modal closes
    await expect(page.getByText(/Revoke Node Certificate|নোড সার্টিফিকেট বাতিল করুন/i)).not.toBeVisible();
  });

  test("rejects agent self-update when binary signature is invalid (T16.08)", async ({ page }) => {
    // Look for Update Agent button on node-master-01
    const updateBtn = page.getByTestId("btn-update-agent-00000000-0000-0000-0000-0000000000aa");
    await expect(updateBtn).toBeVisible();
    await updateBtn.click();

    // Verify dialog opens
    const dialog = page.getByRole("dialog");
    await expect(dialog).toBeVisible();
    await expect(dialog.getByText(/Signed Agent Self-Update|সাইন্ড এজেন্ট সেলফ-আপডেট/i)).toBeVisible();

    // Toggle corrupt signature
    const corruptCheckbox = page.getByTestId("toggle-corrupt-signature");
    await expect(corruptCheckbox).toBeVisible();
    await corruptCheckbox.check();

    // Click Apply Signed Update
    const applyBtn = page.getByTestId("btn-apply-agent-update");
    await applyBtn.click();

    // Verify rejection alert (DoD T16.08)
    const rejectAlert = page.getByTestId("agent-update-rejected-alert");
    await expect(rejectAlert).toBeVisible();
    await expect(rejectAlert).toContainText(/ERR_SIGNATURE_MISMATCH|স্বাক্ষর যাচাই ব্যর্থ/i);
    await expect(page.getByTestId("badge-signature-rejected")).toBeVisible();
  });

  test("applies agent self-update when Ed25519 signature is valid (T16.08)", async ({ page }) => {
    const updateBtn = page.getByTestId("btn-update-agent-00000000-0000-0000-0000-0000000000aa");
    await expect(updateBtn).toBeVisible();
    await updateBtn.click();

    const dialog = page.getByRole("dialog");
    await expect(dialog).toBeVisible();

    // Click Apply Signed Update without corruption
    const applyBtn = page.getByTestId("btn-apply-agent-update");
    await applyBtn.click();

    // Verify success banner and verified status
    const successAlert = page.getByTestId("agent-update-success-alert");
    await expect(successAlert).toBeVisible();
    await expect(page.getByTestId("badge-update-verified")).toBeVisible();
  });

  test("displays node fullness/saturation percentage in table and cluster capacity overview (T16.01 DoD)", async ({ page }) => {
    // Verify Cluster Capacity Overview card is rendered
    const clusterOverview = page.getByTestId("cluster-capacity-overview");
    await expect(clusterOverview).toBeVisible();
    await expect(clusterOverview.getByText(/Cluster Fullness|ক্লাস্টার পূর্ণতা/i)).toBeVisible();

    // Verify Fullness (%) column header (DoD requirement: Admin UI shows node saturation/fullness %)
    await expect(page.getByText(/Fullness \(%\)|পূর্ণতা \(%\)/i)).toBeVisible();

    // Verify node saturation percentages are visible
    await expect(page.getByText("68%").first()).toBeVisible();
    await expect(page.getByText("85%").first()).toBeVisible();

    // Verify saturation progress bar exists
    const saturationBars = page.getByTestId("node-saturation-bar");
    await expect(saturationBars.first()).toBeVisible();
  });

  test("configures placement scheduler and toggles drain mode (DoD T16.02)", async ({ page }) => {
    // Open placement scheduler dialog
    const schedulerBtn = page.getByTestId("configure-scheduler-btn");
    await expect(schedulerBtn).toBeVisible();
    await schedulerBtn.click();

    // Verify dialog content and simulation card
    await expect(page.getByText(/Placement Scheduler & Policies|প্লেসমেন্ট শিডিউলার ও নীতি/i)).toBeVisible();
    await expect(page.getByTestId("placement-simulation-card")).toBeVisible();

    // Verify drainage invariant is mentioned and draining nodes are disqualified
    await expect(page.getByText(/Disqualified Draining \/ Offline Nodes|অযোগ্য ড্রেনিং \/ অফলাইন নোডসমূহ/i)).toBeVisible();

    // Save policy
    const saveBtn = page.getByTestId("save-scheduler-policy-btn");
    await expect(saveBtn).toBeVisible();
    await saveBtn.click();

    // Modal closes
    await expect(page.getByText(/Placement Scheduler & Policies|প্লেসমেন্ট শিডিউলার ও নীতি/i)).not.toBeVisible();

    // Toggle drain on online node
    const drainBtn = page.getByRole("button", { name: /^Drain$|^ড্রেন$/i }).first();
    await expect(drainBtn).toBeVisible();
    await drainBtn.click();

    await expect(
      page.getByText(/Node set to draining mode|নোডটি ড্রেনিং মোডে নেওয়া হয়েছে/i).first()
    ).toBeVisible();

    // Toggle undrain on draining node
    const undrainBtn = page.getByRole("button", { name: /^Undrain$|^সচল করুন$/i }).first();
    await expect(undrainBtn).toBeVisible();
    await undrainBtn.click();

    await expect(
      page.getByText(/Node restored to active cluster service|নোডটি পুনরায় ক্লাস্টার সার্ভিসে সচল করা হয়েছে|Node restored to active service/i).first()
    ).toBeVisible();
  });

  test("opens oversell configuration dialog, modifies ratio, and saves (T16.01)", async ({ page }) => {
    // Click capacity / oversell button
    const editBtn = page.getByRole("button", { name: /Capacity \/ Oversell|ধারণক্ষমতা \/ ওভারসেল/i }).first();
    await expect(editBtn).toBeVisible();
    await editBtn.click();

    // Verify dialog opens
    await expect(page.getByText(/Oversell Settings|ওভারসেল সেটিংস/i)).toBeVisible();
    await expect(page.getByText(/CPU Oversell Multiplier|সিপিইউ ওভারসেল মাল্টিপ্লায়ার/i)).toBeVisible();

    // Modify CPU ratio
    const cpuInput = page.locator("#cpu-ratio-input");
    await expect(cpuInput).toBeVisible();
    await cpuInput.fill("2.5");

    // Click Save
    const saveBtn = page.getByRole("button", { name: /Save|সংরক্ষণ করুন/i });
    await expect(saveBtn).toBeVisible();
    await saveBtn.click();

    // Verify toast feedback
    await expect(
      page.getByText(/Node capacity & oversell settings updated|নোড ধারণক্ষমতা ও ওভারসেল সেটিংস সফলভাবে সংরক্ষিত হয়েছে/i).first()
    ).toBeVisible();
  });

  test("displays node offline alert banner and initiates emergency failover (T16.07)", async ({ page }) => {
    // Verify alert banner is visible
    const alertBanner = page.getByTestId("node-offline-alert-banner");
    await expect(alertBanner).toBeVisible();
    await expect(alertBanner).toContainText(/Heartbeat missed|হার্টবিট বন্ধ/i);

    // Verify offline badge on node row
    await expect(page.getByTestId("node-offline-badge").first()).toBeVisible();

    // Click trigger failover
    const triggerBtn = page.getByTestId("btn-trigger-failover");
    await expect(triggerBtn).toBeVisible();
    await triggerBtn.click();

    // Verify failover modal opens
    const dialog = page.getByRole("dialog");
    await expect(dialog).toBeVisible();
    await expect(dialog.getByText(/Emergency Node Failover|জরুরি নোড ফেলওভার/i)).toBeVisible();

    // Confirm failover
    const confirmBtn = page.getByTestId("btn-confirm-failover");
    await expect(confirmBtn).toBeVisible();
    await confirmBtn.click();

    // Dialog closes
    await expect(dialog).not.toBeVisible();
  });
});
