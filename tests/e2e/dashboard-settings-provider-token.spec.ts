import { expect, test } from "@playwright/test";
import {
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

});