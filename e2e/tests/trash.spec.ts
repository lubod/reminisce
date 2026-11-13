import { test, expect } from "@playwright/test";

test.describe("trash", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/trash");
  });

  test("trash page loads", async ({ page }) => {
    // Either shows empty state or a count heading
    const emptyMsg = page.getByText("Trash is empty");
    const countHeading = page.locator("h2").filter({ hasText: /deleted item/i });
    await expect(emptyMsg.or(countHeading).first()).toBeVisible({ timeout: 5000 });
  });

  test("shows items or empty state", async ({ page }) => {
    const emptyMsg = page.getByText("Trash is empty");
    const items = page.locator("img");
    await expect(emptyMsg.or(items).first()).toBeVisible({ timeout: 5000 });
  });

  test("deleted item can be restored", async ({ page }) => {
    // Session auth uses an HttpOnly cookie shared automatically by page.request
    const res = await page.request.get("/api/image_thumbnails?limit=1");
    if (!res.ok()) {
      test.skip();
      return;
    }
    const data = await res.json();
    const hash: string | undefined = data?.thumbnails?.[0]?.hash;
    if (!hash) {
      test.skip();
      return;
    }

    const delRes = await page.request.post(`/api/image/${hash}/delete`);
    expect(delRes.ok()).toBeTruthy();

    await page.reload();

    const targetImg = page.locator(`img[src*="${hash}"]`);
    await expect(targetImg).toBeVisible();
    await targetImg.hover();

    const restoreBtn = page.getByRole("button", { name: /restore/i }).first();
    await expect(restoreBtn).toBeVisible();
    await Promise.all([
      page.waitForResponse((r) => r.url().includes(`/api/image/${hash}/restore`) && r.ok()),
      restoreBtn.click(),
    ]);

    // Item disappears from trash
    await expect(targetImg).not.toBeVisible();
  });
});
