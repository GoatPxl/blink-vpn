import { expect, test } from "@playwright/test";
import { installSettingsLifecycleMock } from "./fixtures/tauri-mocks";

test.describe("Dashboard settings lifecycle", () => {
  test("shows dependency-aware row actions and keeps providers visible after delete", async ({ page }) => {
    await installSettingsLifecycleMock(page, { attachNodeToDigitalOcean: true });

    await page.goto("/dashboard/settings");

    const digitalOceanRow = page.getByRole("row", { name: /DigitalOcean/i });
    const dockerRow = page.getByRole("row", { name: /Docker/i });

    await expect(digitalOceanRow.getByText("1", { exact: true })).toBeVisible();
    await expect(digitalOceanRow.getByText("Destroy nodes first", { exact: true })).toBeVisible();
    await expect(digitalOceanRow.getByRole("button", { name: "Delete", exact: true })).toBeDisabled();

    page.once("dialog", async (dialog) => {
      await dialog.accept();
    });
    await digitalOceanRow.getByRole("button", { name: "Suspend", exact: true }).click();
    await expect(digitalOceanRow.getByText("Suspended", { exact: true })).toBeVisible();

    page.once("dialog", async (dialog) => {
      await dialog.accept();
    });
    await digitalOceanRow.getByRole("button", { name: "Resume", exact: true }).click();
    await expect(digitalOceanRow.getByText("Active", { exact: true })).toBeVisible();

    await expect(dockerRow.getByText("0", { exact: true })).toBeVisible();
    page.once("dialog", async (dialog) => {
      await dialog.accept();
    });
    await dockerRow.getByRole("button", { name: "Delete", exact: true }).click();

    await expect(dockerRow).toContainText("Missing key");
    await expect(dockerRow).toContainText("Not configured");
    await expect(dockerRow.getByRole("button", { name: "Add Key", exact: true })).toBeVisible();
    await expect(dockerRow.getByRole("button", { name: "Delete", exact: true })).toBeEnabled();
  });

  test("allows deleting the DigitalOcean key when no nodes are attached", async ({ page }) => {
    await installSettingsLifecycleMock(page, { attachNodeToDigitalOcean: false });

    await page.goto("/dashboard/settings");

    const digitalOceanRow = page.getByRole("row", { name: /DigitalOcean/i });

    await expect(digitalOceanRow.getByText("0", { exact: true })).toBeVisible();
    await expect(digitalOceanRow.getByRole("button", { name: "Delete", exact: true })).toBeEnabled();

    page.once("dialog", async (dialog) => {
      await dialog.accept();
    });
    await digitalOceanRow.getByRole("button", { name: "Delete", exact: true }).click();

    await expect(digitalOceanRow).toContainText("Missing key");
    await expect(digitalOceanRow).toContainText("Not configured");
    await expect(digitalOceanRow.getByRole("button", { name: "Add Key", exact: true })).toBeVisible();
  });
});
