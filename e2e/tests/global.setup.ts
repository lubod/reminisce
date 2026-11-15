import { test as setup } from "@playwright/test";
import path from "path";

const authFile = path.join(__dirname, "../.auth/admin.json");

const ADMIN_USER = process.env.E2E_ADMIN_USER ?? "admin";
const ADMIN_PASSWORD = process.env.E2E_ADMIN_PASSWORD ?? "admin123";

setup("authenticate as admin", async ({ page }) => {
  await page.goto("/login");
  await page.getByPlaceholder("Enter your username").fill(ADMIN_USER);
  await page.getByPlaceholder("Enter your password").fill(ADMIN_PASSWORD);
  await page.getByRole("button", { name: "Sign In" }).click();
  await page.waitForURL("/");

  // Seed minimal EXIF-less PNG fixtures if the E2E library has fewer than 2 images so
  // media lightbox, orientation viewer, and trash restore specs do not skip on empty DBs.
  try {
    const listRes = await page.request.get("/api/image_thumbnails?limit=2");
    if (listRes.ok()) {
      const data = await listRes.json();
      if ((data?.thumbnails?.length ?? 0) < 2) {
        const fixtures = [
          {
            name: "e2e-seed-1.png",
            hash: "7f186464404ea5cc5c233423ce2d411c90c4a488d0f8139c1190273fc5447e15",
            hex: "89504e470d0a1a0a0000000d49484452000000100000000c0802000000e485aad60000001549444154789c6338a1a141126218d530aa013b0200b46bd201f4e165240000000049454e44ae426082",
            orientation: 6,
          },
          {
            name: "e2e-seed-2.png",
            hash: "fd4ce9653ad73c74fddef08a7e5210d95cb645de03456e3b3101946491f3b81a",
            hex: "89504e470d0a1a0a0000000d49484452000000100000000c0802000000e485aad60000001549444154789c63d0a8b843126218d530aa013b0200dc661d1088e447390000000049454e44ae426082",
            orientation: 1,
          },
        ];
        for (const fixture of fixtures) {
          await page.request.post("/api/upload/image", {
            multipart: {
              hash: fixture.hash,
              name: fixture.name,
              device_id: "e2e-seed",
              image: {
                name: fixture.name,
                mimeType: "image/png",
                buffer: Buffer.from(fixture.hex, "hex"),
              },
            },
          });
          await page.request.post(`/api/image/${fixture.hash}/orientation`, {
            data: { orientation: fixture.orientation },
          });
        }
      }
    }
  } catch {
    // Keep global setup resilient if seeding is unavailable.
  }

  await page.context().storageState({ path: authFile });
});
