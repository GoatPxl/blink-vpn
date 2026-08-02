import { expect, test } from "@playwright/test";
import {
  installProviderStateMutationMock,
  installSingleProviderTokenMock,
} from "./fixtures/tauri-mocks";

test.describe("Dashboard settings provider token", () => {
  test("opens provider modal from card, saves token, and shows updated connected provider details", async ({ page }) => {
    const apiToken = process.env.DO_API_TOKEN ?? "dop_v1_test_token_1234567890abcdef";
    await installSingleProviderTokenMock(page, apiToken);

    await page.goto("/dashboard/settings");

    await expect(page.getByRole("heading", { name: "Provider API Keys" })).toBeVisible();
    await expect(page.getByText("Installed API keys by provider", { exact: true })).toBeVisible();
    await expect(page.getByRole("row", { name: /DigitalOcean/i }).getByText("Missing", { exact: true })).toBeVisible();

    await page.getByRole("button", { name: "Open Key Entry Modal" }).click();
    await expect(page.getByRole("heading", { name: "Provider Credential" })).toBeVisible();
    await page.getByPlaceholder("dop_v1_...").fill(apiToken);
    await page.getByRole("button", { name: "Save Token", exact: true }).click();

    await expect(page.getByRole("row", { name: /DigitalOcean/i })).toContainText("Installed");
    await expect(page.getByRole("row", { name: /DigitalOcean/i }).getByText("do-token...f6bf", { exact: true })).toBeVisible();
    await expect(page.getByText(/Provider registered with token_ref:/).first()).toBeVisible();
    await expect(page.getByPlaceholder("dop_v1_...")).toHaveValue("");
  });

  test("updates the provider list when suspending or deleting a provider", async ({ page }) => {
    await installProviderStateMutationMock(page);

    await page.goto("/dashboard/settings");

    const digitalOceanRow = page.getByRole("row", { name: /DigitalOcean/i });
    await expect(digitalOceanRow.getByText("Connected", { exact: true })).toBeVisible();

    page.once("dialog", async (dialog) => {
      await dialog.accept();
    });
    await digitalOceanRow.getByRole("button", { name: "Suspend" }).click();

    await expect(digitalOceanRow).toContainText("Suspended");
    await expect(digitalOceanRow.getByRole("button", { name: "Suspended" })).toBeVisible();

    page.once("dialog", async (dialog) => {
      await dialog.accept();
    });
    await digitalOceanRow.getByRole("button", { name: "Delete" }).click();

    await expect(page.getByRole("row", { name: /DigitalOcean/i })).toHaveCount(0);
    await expect(page.getByRole("row", { name: /Docker/i })).toBeVisible();
  });
});