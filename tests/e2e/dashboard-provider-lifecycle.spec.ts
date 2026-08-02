import { expect, test } from "@playwright/test";
import { installProviderLifecycleMock } from "./fixtures/tauri-mocks";

test.describe("Dashboard provider lifecycle", () => {
  test("keeps settings and nodes aligned across suspend and resume", async ({ page }) => {
    await installProviderLifecycleMock(page);

    await page.goto("/dashboard/settings");

    const digitalOceanRow = page.getByRole("row", { name: /DigitalOcean/i });
    await expect(digitalOceanRow.getByText("Connected", { exact: true })).toBeVisible();

    page.once("dialog", async (dialog) => {
      await dialog.accept();
    });
    await digitalOceanRow.getByRole("button", { name: "Suspend" }).click();

    await expect(digitalOceanRow).toContainText("Suspended");
    await expect(digitalOceanRow.getByRole("button", { name: "Resume" })).toBeVisible();

    await page.goto("/dashboard/nodes");
    await expect(page.getByText("Provider access will be checked on the create page.")).toBeVisible();
    await expect(page.getByText(/Provider DigitalOcean is suspended\./)).toBeVisible();
    await expect(page.getByText("Primary Node", { exact: true })).toBeVisible();

    await page.goto("/dashboard/nodes/create");
    await expect(page.getByText("Provisioning is disabled until provider access is healthy.")).toBeVisible();
    await expect(page.getByRole("button", { name: "Create Node", exact: true })).toBeDisabled();

    page.once("dialog", async (dialog) => {
      await dialog.accept();
    });
    await page.goto("/dashboard/settings");
    await digitalOceanRow.getByRole("button", { name: "Resume" }).click();

    await expect(digitalOceanRow).toContainText("Connected");
    await expect(digitalOceanRow.getByRole("button", { name: "Suspend" })).toBeVisible();

    await page.goto("/dashboard/nodes/create");
    await expect(page.getByText(/Provider access: Ready/).first()).toBeVisible();
    await expect(page.getByRole("button", { name: "Create Node", exact: true })).toBeEnabled();
  });
});
