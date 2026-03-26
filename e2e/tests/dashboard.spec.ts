import { test, expect } from "@playwright/test";

test.describe("dashboard", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/");
  });

  test("overview tab shows stat cards", async ({ page }) => {
    for (const label of ["Total Images", "Total Videos", "Total Users"]) {
      await expect(page.getByText(label)).toBeVisible();
    }
  });

  test("all tabs are present and switch content", async ({ page }) => {
    const tabs = [
      { name: "Import", content: /browser upload|server import/i },
      { name: "Backup", content: /p2p|backup|storage/i },
      { name: "System", content: /database|cpu|memory/i },
      { name: "Settings", content: /ai captions|semantic index|face grouping/i },
      { name: "App Setup", content: /android|mobile|qr/i },
      { name: "Users", content: /admin/i },
    ];
    for (const { name, content } of tabs) {
      await page.getByRole("button", { name }).click();
      await expect(page.getByText(content).first()).toBeVisible({ timeout: 5000 });
    }
  });

  test("import tab shows import buttons", async ({ page }) => {
    await page.getByRole("button", { name: "Import" }).click();
    await expect(page.getByRole("button", { name: "Select Files" })).toBeVisible();
    await expect(page.getByRole("button", { name: "Choose Directory" })).toBeVisible();
  });

  test("settings tab shows AI processing checkboxes", async ({ page }) => {
    await page.getByRole("button", { name: "Settings" }).click();
    await expect(page.getByText("AI Captions")).toBeVisible();
    await expect(page.getByText("Semantic Index")).toBeVisible();
    await expect(page.getByText("Face Grouping")).toBeVisible();
    await expect(page.getByRole("button", { name: /save preferences/i })).toBeVisible();
  });

  test("settings can be toggled and saved", async ({ page }) => {
    await page.getByRole("button", { name: "Settings" }).click();

    const checkbox = page.locator('input[type="checkbox"]').first();
    await expect(checkbox).toBeVisible();
    const initial = await checkbox.isChecked();

    await checkbox.click();
    await expect(checkbox).toBeChecked({ checked: !initial });
    await page.getByRole("button", { name: /save preferences/i }).click();

    // Reload and verify persistence
    await page.reload();
    await page.getByRole("button", { name: "Settings" }).click();
    await expect(page.locator('input[type="checkbox"]').first()).toBeChecked({ checked: !initial });

    // Restore
    await page.locator('input[type="checkbox"]').first().click();
    await page.getByRole("button", { name: /save preferences/i }).click();
  });

  test("system tab shows health indicators", async ({ page }) => {
    await page.getByRole("button", { name: "System" }).click();
    await expect(page.getByText(/database/i).first()).toBeVisible();
  });

  test("users tab loads and shows add user button", async ({ page }) => {
    await page.getByRole("button", { name: "Users" }).click();
    await expect(page.getByRole("button", { name: /add user/i })).toBeVisible({ timeout: 5000 });
  });
});
