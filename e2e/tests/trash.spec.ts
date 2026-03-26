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
    // Soft-delete an image via API so trash has an item
    const token = await page.evaluate(() => localStorage.getItem("token"));
    const headers = token ? { Authorization: `Bearer ${token}` } : {};
    const res = await page.request.get("/api/image_thumbnails?limit=1", { headers });
    if (!res.ok()) { test.skip(); return; }
    const data = await res.json();
    if (!data?.items?.length) { test.skip(); return; }

    const hash = data.items[0].hash;
    await page.request.post(`/api/image/${hash}/delete`, { headers });

    await page.reload();

    const thumbnail = page.locator("img").first();
    await expect(thumbnail).toBeVisible();
    await thumbnail.hover();

    const restoreBtn = page.getByRole("button", { name: /restore/i });
    await expect(restoreBtn).toBeVisible();
    await restoreBtn.click();

    // Item disappears from trash
    await expect(page.locator(`img[src*="${hash}"]`)).not.toBeVisible();
  });
});
