import { test, expect } from "@playwright/test";

test.describe("duplicates", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/duplicates");
  });

  test("duplicates page loads with heading", async ({ page }) => {
    await expect(page.getByRole("heading", { name: /duplicate/i })).toBeVisible();
  });

  test("shows duplicate groups or empty state after scan completes", async ({ page }) => {
    // Wait for the heading to stop showing "Scanning..."
    await expect(page.locator("h2")).not.toContainText("Scanning", { timeout: 60000 });
    // Now it shows either "X duplicate group(s) found" or "No duplicates found"
    const heading = page.locator("h2");
    await expect(heading).toContainText(/duplicate|found/i);
  });

  test("threshold slider is present and adjustable", async ({ page }) => {
    const slider = page.locator('input[type="range"]');
    await expect(slider).toBeVisible();

    const initial = await slider.inputValue();
    await slider.fill("90");
    await expect(slider).toHaveValue("90");
    await slider.fill(initial);
  });

  test("refresh button is present", async ({ page }) => {
    // Use exact: true to distinguish from nav's "Refresh duplicates" button
    await expect(page.getByRole("button", { name: "Refresh", exact: true })).toBeVisible();
  });
});
