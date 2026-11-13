import { test, expect } from "@playwright/test";

test.describe("map view", () => {
    test("map/grid toggle shows an interactive leaflet map", async ({ page }) => {
        await page.goto("/media");

        const toggle = page.getByRole("button", { name: /map/i });
        await expect(toggle).toBeVisible({ timeout: 10000 });
        await toggle.click();

        await expect(page.locator(".leaflet-container")).toBeVisible({ timeout: 10000 });

        // Switching back to the grid is possible.
        const back = page.getByRole("button", { name: /grid/i });
        await expect(back).toBeVisible();
        await back.click();
        await expect(page.locator(".leaflet-container")).not.toBeVisible();
    });

    test("map toggle is disabled while searching", async ({ page }) => {
        await page.goto("/media");
        const toggle = page.getByRole("button", { name: /map/i });
        await expect(toggle).toBeEnabled({ timeout: 10000 });

        const filtersToggle = page.getByRole("button", { name: /filters & search/i });
        await filtersToggle.click();
        const searchInput = page.getByPlaceholder("Search collection...");
        await expect(searchInput).toBeVisible();

        await searchInput.fill("sunset");
        await Promise.all([
            page.waitForResponse((r) => r.url().includes("/api/search")),
            searchInput.press("Enter"),
        ]);
        await expect(toggle).toBeDisabled();

        await page.getByText("Reset Filters").click();
        await expect(toggle).toBeEnabled();
    });
});
