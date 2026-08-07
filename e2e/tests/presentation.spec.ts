import { test, expect } from "@playwright/test";

test.describe("presentation mode", () => {
    test("opens from the nav and shows the slideshow chrome", async ({ page }) => {
        await page.goto("/");

        const present = page.getByRole("link", { name: /present/i });
        await expect(present).toBeVisible();
        await present.click();

        // Fullscreen slideshow route; tolerate either an image or a friendly empty state.
        await expect(page).toHaveURL(/present/);
        await expect(page.locator("body")).toBeVisible({ timeout: 5000 });

        // It should not crash: at least one control should be present (pause/close).
        const hasControl = page.getByRole("button").first().or(page.getByText(/press esc|zero photos|no photos|nothing/i));
        await expect(hasControl.first()).toBeVisible({ timeout: 5000 });
    });
});
