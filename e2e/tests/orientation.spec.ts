import { test, expect } from "@playwright/test";

test("orientation check tab loads from nav and toggles wrong-orientation filter", async ({ page }) => {
  await page.goto("/orientation");
  await expect(page.getByRole("heading", { name: "Orientation Check" })).toBeVisible();
  await expect(page.getByText(/photos with no EXIF metadata/)).toBeVisible();

  const toggle = page.getByRole("button", { name: "Wrong orientation only" });
  await expect(toggle).toBeVisible();
  await expect(toggle).toHaveAttribute("aria-pressed", "true");

  await toggle.click();
  await expect(toggle).toHaveAttribute("aria-pressed", "false");
  await expect(page.getByText(/photos with no EXIF metadata/)).toBeVisible();

  await toggle.click();
  await expect(toggle).toHaveAttribute("aria-pressed", "true");
});
