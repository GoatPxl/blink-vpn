import { expect, test } from "@playwright/test";
import { installDockerSettingsMock } from "./fixtures/tauri-mocks";

test.describe("Dashboard Docker provider", () => {
  test.skip(
    process.env.NEXT_PUBLIC_DEV_DOCKER !== "1",
    "Docker dashboard tests require NEXT_PUBLIC_DEV_DOCKER=1 for frontend Docker surfaces.",
  );

  test("shows Docker modal affordances when Docker provider is selected", async ({ page }) => {
    await installDockerSettingsMock(page);

    await page.goto("/dashboard/settings");

    await page.getByRole("button", { name: "Open Key Entry Modal" }).click();
    await expect(page.getByRole("heading", { name: "Provider Credential" })).toBeVisible();

    await page.getByLabel("Provider").selectOption("provider-docker");

    await expect(page.getByLabel("Docker Key")).toBeVisible();
    await expect(page.getByPlaceholder("dev-docker-token")).toBeVisible();
    await expect(
      page.getByText("Docker provider uses a local development key path and can be validated with the same modal flow."),
    ).toBeVisible();
  });

  test("runs Docker readiness validation from the settings modal", async ({ page }) => {
    await installDockerSettingsMock(page, {
      dockerReadiness: {
        installed: true,
        daemon_reachable: true,
        cli_version: "Docker version 27.0.0, build test",
        daemon_version: "27.0.0",
        message: "Docker CLI and daemon are available for development emulation.",
        action_guidance: "You can use Docker development mode workflows from Settings and Node planning surfaces.",
      },
    });

    await page.goto("/dashboard/settings");

    await page.getByRole("button", { name: "Open Key Entry Modal" }).click();
    await page.getByLabel("Provider").selectOption("provider-docker");
    await page.getByPlaceholder("dev-docker-token").fill("dkr_v1_local_dev_token");

    await page.getByRole("button", { name: "Save Token" }).click();
    await expect(page.getByText("Docker faux token saved")).toBeVisible();

    await page.getByRole("button", { name: "Validate Key" }).click();
    await expect(page.getByText("Docker readiness checked")).toBeVisible();
    await expect(page.getByText("Docker CLI and daemon are available for development emulation.")).toBeVisible();
  });

  test("registers Docker token, persists local marker, and keeps diagnostics redacted", async ({ page }) => {
    const dockerToken = "dkr_v1_local_sensitive_token_98765";

    await installDockerSettingsMock(page);

    await page.goto("/dashboard/settings");

    await page.getByRole("button", { name: "Open Key Entry Modal" }).click();
    await page.getByLabel("Provider").selectOption("provider-docker");
    await page.getByPlaceholder("dev-docker-token").fill(dockerToken);

    await page.getByRole("button", { name: "Save Token" }).click();
    await expect(page.getByText("Provider registered with token_ref:")).toBeVisible();

    const dockerRow = page.getByRole("row", { name: /Docker/i });
    await expect(dockerRow).toContainText("Installed");
    await expect(dockerRow).toContainText("do-token...cdef");
    await expect(page.getByText(dockerToken, { exact: true })).toHaveCount(0);

    const storedMarker = await page.evaluate(() => window.localStorage.getItem("goatstare.docker.fauxTokenConfiguredAt"));
    expect(storedMarker).not.toBeNull();

    await page.goto("/dashboard/diagnostics");
    await expect(page.getByText("token_ref removed from diagnostics bundle")).toBeVisible();
    await expect(page.getByText(dockerToken, { exact: true })).toHaveCount(0);
  });

  test("rejects Docker token values without accepted prefixes", async ({ page }) => {
    await installDockerSettingsMock(page);

    await page.goto("/dashboard/settings");

    await page.getByRole("button", { name: "Open Key Entry Modal" }).click();
    await page.getByLabel("Provider").selectOption("provider-docker");
    await page.getByPlaceholder("dev-docker-token").fill("invalid-token-format");
    await page.getByRole("button", { name: "Save Token" }).click();

    await expect(page.getByText("Docker faux token must start with 'dkr_v1_' or 'docker_'.")).toBeVisible();
  });
});
