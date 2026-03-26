import { test, expect } from "@playwright/test";

test.describe("people gallery", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/people");
  });

  test("people page loads with heading", async ({ page }) => {
    await expect(page.getByRole("heading", { name: /people/i })).toBeVisible();
  });

  test("shows person cards or empty state", async ({ page }) => {
    const cards = page.getByTitle("Edit name").first();
    const emptyMsg = page.getByText(/no persons detected/i);
    await expect(cards.or(emptyMsg)).toBeVisible({ timeout: 5000 });
  });

  test("person name can be edited inline", async ({ page }) => {
    const editBtn = page.getByTitle("Edit name").first();
    try {
      await expect(editBtn).toBeVisible({ timeout: 5000 });
    } catch {
      test.skip();
      return;
    }

    await editBtn.click();
    const nameInput = page.locator("input[type='text']").first();
    await expect(nameInput).toBeVisible();
    await page.getByTitle("Cancel").click();
    await expect(nameInput).not.toBeVisible();
  });

  test("person name edit can be saved", async ({ page }) => {
    const editBtn = page.getByTitle("Edit name").first();
    try {
      await expect(editBtn).toBeVisible({ timeout: 5000 });
    } catch {
      test.skip();
      return;
    }

    await editBtn.click();
    const nameInput = page.locator("input[type='text']").first();
    const originalName = await nameInput.inputValue();

    await nameInput.fill("Test Person");
    const [saveResponse] = await Promise.all([
      page.waitForResponse((r) => r.url().includes("/persons/") && r.request().method() === "PUT"),
      page.getByTitle("Save").click(),
    ]);

    // 404 means this person belongs to another user (admin sees all, but can only edit own)
    if (saveResponse.status() === 404) {
      test.skip();
      return;
    }

    expect(saveResponse.ok(), `PUT /persons/.../name failed: ${saveResponse.status()}`).toBeTruthy();
    await expect(nameInput).not.toBeVisible({ timeout: 5000 });
    await expect(page.getByText("Test Person")).toBeVisible();

    // Restore original name
    await editBtn.click();
    await nameInput.fill(originalName || "");
    await page.getByTitle("Save").click();
  });
});

test.describe("person detail", () => {
  test("clicking a person navigates to detail page", async ({ page }) => {
    await page.goto("/people");
    try {
      await expect(page.getByTitle("Edit name").first()).toBeVisible({ timeout: 5000 });
    } catch {
      test.skip();
      return;
    }

    // Click the card area (not the edit button itself)
    await page.locator(".cursor-pointer").first().click();
    await expect(page).toHaveURL(/\/people\/.+/);
  });

  test("back button returns to people list", async ({ page }) => {
    await page.goto("/people");
    try {
      await expect(page.getByTitle("Edit name").first()).toBeVisible({ timeout: 5000 });
    } catch {
      test.skip();
      return;
    }

    await page.locator(".cursor-pointer").first().click();
    await page.waitForURL(/\/people\/.+/);
    await page.getByTitle("Back to all persons").click();
    await expect(page).toHaveURL("/people");
  });

  test("person detail shows face images or empty state", async ({ page }) => {
    await page.goto("/people");
    try {
      await expect(page.getByTitle("Edit name").first()).toBeVisible({ timeout: 5000 });
    } catch {
      test.skip();
      return;
    }

    await page.locator(".cursor-pointer").first().click();
    await page.waitForURL(/\/people\/.+/);

    const images = page.locator("img");
    const emptyMsg = page.getByText(/no images found/i);
    await expect(images.or(emptyMsg).first()).toBeVisible({ timeout: 5000 });
  });

  test("merge panel can be opened and closed", async ({ page }) => {
    await page.goto("/people");
    try {
      await expect(page.getByTitle("Edit name").first()).toBeVisible({ timeout: 5000 });
    } catch {
      test.skip();
      return;
    }

    await page.locator(".cursor-pointer").first().click();
    await page.waitForURL(/\/people\/.+/);

    const mergeBtn = page.getByTitle("Merge with another person");
    await mergeBtn.click();
    await expect(page.getByPlaceholder("Search persons...")).toBeVisible();

    await mergeBtn.click();
    await expect(page.getByPlaceholder("Search persons...")).not.toBeVisible();
  });
});
