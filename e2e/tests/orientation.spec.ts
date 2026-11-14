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

test("orientation viewer keeps rotation controls uncovered", async ({ page }) => {
  await page.goto("/orientation");
  await expect(page.getByRole("heading", { name: "Orientation Check" })).toBeVisible();

  const thumb = page.locator('img[src*="/api/thumbnail/"]').first();
  if (!(await thumb.isVisible({ timeout: 3000 }).catch(() => false))) {
    const toggle = page.getByRole("button", { name: "Wrong orientation only" });
    await toggle.click();
    await expect(toggle).toHaveAttribute("aria-pressed", "false");
  }

  if (!(await thumb.isVisible({ timeout: 5000 }).catch(() => false))) {
    test.skip();
    return;
  }

  await thumb.click();

  const dialog = page.getByRole("dialog", { name: "Image orientation viewer" });
  await expect(dialog).toBeVisible();

  for (const label of ["Rotate 90° CCW", "Rotate 90° CW"]) {
    const btn = dialog.getByRole("button", { name: label });
    await expect(btn).toBeVisible();
    await btn.click({ trial: true });
    const box = await btn.boundingBox();
    expect(box).not.toBeNull();
    expect(box!.width).toBeGreaterThan(0);
    expect(box!.height).toBeGreaterThan(0);
    const uncovered = await btn.evaluate((el) => {
      const rect = el.getBoundingClientRect();
      const hit = document.elementFromPoint(
        rect.left + rect.width / 2,
        rect.top + rect.height / 2,
      );
      return !!hit && el.contains(hit);
    });
    expect(uncovered).toBe(true);
  }
});
