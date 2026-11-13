import { test, expect } from "@playwright/test";

const ADMIN_USER = process.env.E2E_ADMIN_USER ?? "admin";
const ADMIN_PASSWORD = process.env.E2E_ADMIN_PASSWORD ?? "admin123";

test("login page loads", async ({ page }) => {
  await page.goto("/login");
  await expect(page.getByRole("heading", { name: "Reminisce" })).toBeVisible();
  await expect(page.getByPlaceholder("Enter your username")).toBeVisible();
  await expect(page.getByPlaceholder("Enter your password")).toBeVisible();
  await expect(page.getByRole("button", { name: "Sign In" })).toBeVisible();
});

test("login with valid credentials redirects to dashboard", async ({ page }) => {
  await page.goto("/login");
  await page.getByPlaceholder("Enter your username").fill(ADMIN_USER);
  await page.getByPlaceholder("Enter your password").fill(ADMIN_PASSWORD);
  await page.getByRole("button", { name: "Sign In" }).click();
  await expect(page).toHaveURL("/");
  // Logout button is visible after successful login
  await expect(page.getByTitle("Logout")).toBeVisible();
});

test("logout clears session and redirects to login", async ({ page }) => {
  await page.goto("/login");
  await page.getByPlaceholder("Enter your username").fill(ADMIN_USER);
  await page.getByPlaceholder("Enter your password").fill(ADMIN_PASSWORD);
  await page.getByRole("button", { name: "Sign In" }).click();
  await expect(page).toHaveURL("/");

  await page.getByTitle("Logout").click();
  await expect(page).toHaveURL("/login");

  // Navigating to a protected route after logout redirects back to login
  await page.goto("/media");
  await expect(page).toHaveURL("/login");
});

test("login with invalid credentials shows error", async ({ page }) => {
  await page.goto("/login");
  await page.getByPlaceholder("Enter your username").fill(ADMIN_USER);
  await page.getByPlaceholder("Enter your password").fill("wrongpassword");
  await page.getByRole("button", { name: "Sign In" }).click();
  await expect(page.locator(".text-red-400")).toBeVisible();
});

test("unauthenticated access redirects to login", async ({ page }) => {
  await page.goto("/");
  await expect(page).toHaveURL("/login");
});
