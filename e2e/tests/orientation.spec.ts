import { test, expect } from "@playwright/test";

test("orientation check tab loads from nav", async ({ page }) => {
  await page.goto("/orientation");
  await expect(page.getByRole("heading", { name: "Orientation Check" })).toBeVisible();
  await expect(page.getByText(/photos with no EXIF metadata/)).toBeVisible();
});
