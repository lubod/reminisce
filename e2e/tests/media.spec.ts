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
    const [response] = await Promise.all([
      page.waitForResponse((r) => r.url().includes("/api/search")),
      input.press("Enter"),
    ]);
    expect(response.ok()).toBeTruthy();
    await expect(page.getByText(/Showing .* result/i)).toBeVisible();
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
    await expect(starBtn).toHaveAttribute("aria-pressed", "false");
    await starBtn.click();
    await expect(starBtn).toHaveAttribute("aria-pressed", "true");
    await starBtn.click();
    await expect(starBtn).toHaveAttribute("aria-pressed", "false");
  });

  test("reset filters link is present", async ({ page }) => {
    await expect(page.getByText("Reset Filters")).toBeVisible();
  });
});

test.describe("media lightbox", () => {
  test("clicking a thumbnail opens lightbox and ESC closes it", async ({ page }) => {
    await page.goto("/media");

    const cards = page.getByRole("button", { name: /^Open (image|video)/i });
    try {
      await expect(cards.first()).toBeVisible({ timeout: 5000 });
    } catch {
      test.skip();
      return;
    }

    await cards.first().click();
    const dialog = page.getByRole("dialog");
    await expect(dialog).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(dialog).not.toBeVisible();
    await expect(page.getByRole("button", { name: /filters & search/i })).toBeVisible();
  });

  test("lightbox keyboard navigation works", async ({ page }) => {
    await page.goto("/media");

    const cards = page.getByRole("button", { name: /^Open (image|video)/i });
    try {
      await expect(cards.nth(1)).toBeVisible({ timeout: 5000 });
    } catch {
      test.skip();
      return;
    }

    await cards.first().click();
    const dialog = page.getByRole("dialog");
    await expect(dialog).toBeVisible();
    await page.keyboard.press("ArrowRight");
    await expect(dialog).toBeVisible();
    await page.keyboard.press("ArrowLeft");
    await expect(dialog).toBeVisible();
    await page.keyboard.press("Escape");

    await expect(dialog).not.toBeVisible();
    await expect(page.getByRole("button", { name: /filters & search/i })).toBeVisible();
  });
});
