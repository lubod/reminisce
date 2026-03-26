import { test as setup } from "@playwright/test";
import path from "path";

const authFile = path.join(__dirname, "../.auth/admin.json");

setup("authenticate as admin", async ({ page }) => {
  await page.goto("/login");
  await page.getByPlaceholder("Enter your username").fill("admin");
  await page.getByPlaceholder("Enter your password").fill("admin123");
  await page.getByRole("button", { name: "Sign In" }).click();
  await page.waitForURL("/");
  await page.context().storageState({ path: authFile });
});
