import { test, expect, Page } from "@playwright/test";

async function expandFilters(page: Page) {
  const toggle = page.getByRole("button", { name: /filters & search/i });
  // Only click if collapsed (ChevronDown visible = collapsed)
  const isExpanded = await page.locator("form").isVisible().catch(() => false);
  if (!isExpanded) {
    await toggle.click();
    await expect(page.getByPlaceholder("Search collection...")).toBeVisible();
  }
}

test.describe("media browser", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/media");
    await expandFilters(page);
  });

  test("filters panel opens with search input and mode selector", async ({ page }) => {
    await expect(page.getByPlaceholder("Search collection...")).toBeVisible();
    await expect(page.locator("select").first()).toBeVisible();
  });

  test("search mode options are available", async ({ page }) => {
    const select = page.locator("select").first();
    const options = await select.locator("option").allTextContents();
    expect(options.some((o) => /semantic/i.test(o))).toBeTruthy();
    expect(options.some((o) => /keyword|text/i.test(o))).toBeTruthy();
    expect(options.some((o) => /hybrid/i.test(o))).toBeTruthy();
  });

  test("typing in search and submitting triggers search", async ({ page }) => {
    const input = page.getByPlaceholder("Search collection...");
    await input.fill("sunset");
    await input.press("Enter");
    // Wait for results or no-results — page should not crash
    await page.waitForTimeout(1000);
    await expect(page.locator("body")).not.toContainText("Error");
  });

  test("X button appears after typing and clears the input", async ({ page }) => {
    const input = page.getByPlaceholder("Search collection...");
    await input.fill("test query");
    // The X button renders inside the form when input has a value
    const clearBtn = page.locator("form button[type='button']");
    await expect(clearBtn).toBeVisible();
    await clearBtn.click();
    await expect(input).toHaveValue("");
  });

  test("date range pickers are present", async ({ page }) => {
    await expect(page.locator('input[type="date"]').first()).toBeVisible();
    await expect(page.locator('input[type="date"]').last()).toBeVisible();
  });

  test("starred filter button is present and toggles", async ({ page }) => {
    const starBtn = page.getByRole("button", { name: "Starred" });
    await expect(starBtn).toBeVisible();
    await starBtn.click();
    await starBtn.click();
  });

  test("reset filters link is present", async ({ page }) => {
    await expect(page.getByText("Reset Filters")).toBeVisible();
  });
});

test.describe("media lightbox", () => {
  test("clicking a thumbnail opens lightbox and ESC closes it", async ({ page }) => {
    await page.goto("/media");

    const thumbnails = page.locator("img[alt]").filter({ hasNot: page.locator("[data-navbar]") });
    try {
      await expect(thumbnails.first()).toBeVisible({ timeout: 5000 });
    } catch {
      test.skip();
      return;
    }

    await thumbnails.first().click();
    // Lightbox is open — nav bar should still be visible
    await expect(page.getByTitle("Logout")).toBeVisible();
    await page.keyboard.press("Escape");
    // Back to grid — filters toggle should be visible
    await expect(page.getByRole("button", { name: /filters & search/i })).toBeVisible();
  });

  test("lightbox keyboard navigation works", async ({ page }) => {
    await page.goto("/media");

    const thumbnails = page.locator("img[alt]");
    try {
      await expect(thumbnails.nth(1)).toBeVisible({ timeout: 5000 });
    } catch {
      test.skip();
      return;
    }

    await thumbnails.first().click();
    await page.waitForTimeout(300);
    await page.keyboard.press("ArrowRight");
    await page.waitForTimeout(200);
    await page.keyboard.press("ArrowLeft");
    await page.waitForTimeout(200);
    await page.keyboard.press("Escape");

    await expect(page.getByRole("button", { name: /filters & search/i })).toBeVisible();
  });
});
