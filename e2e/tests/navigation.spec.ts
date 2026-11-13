import { test, expect } from "@playwright/test";

const NAV_LINKS = [
  { name: "Dashboard", url: "/" },
  { name: "Media", url: "/media" },
  { name: "People", url: "/people" },
  { name: "Present", url: "/present" },
  { name: "Duplicates", url: "/duplicates" },
  { name: "Trash", url: "/trash" },
];

test.describe("navigation", () => {
  test("all nav links are present and route correctly", async ({ page }) => {
    await page.goto("/");
    for (const { name, url } of NAV_LINKS) {
      await page.getByRole("link", { name }).first().click();
      await expect(page).toHaveURL(url);
    }
  });

  test("refresh button is visible on each page", async ({ page }) => {
    for (const { url } of NAV_LINKS.filter((l) => l.url !== "/present")) {
      await page.goto(url);
      await expect(page.getByTitle(/refresh/i)).toBeVisible();
    }
  });
});

test.describe("auth guards", () => {
  test.use({ storageState: { cookies: [], origins: [] } });

  test("unauthenticated user is redirected from all protected routes", async ({
    page,
  }) => {
    // Clear session cookies explicitly before testing protected route redirects
    await page.context().clearCookies();

    const protectedRoutes = ["/", "/media", "/people", "/duplicates", "/trash"];
    for (const route of protectedRoutes) {
      await page.goto(route);
      await expect(page).toHaveURL("/login", { timeout: 5000 });
    }
  });
});
