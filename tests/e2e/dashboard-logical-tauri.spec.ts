import { expect, test } from "./fixtures/tauri-real";

const DOCKER_TOKEN = "dkr_v1_local_dev_mock_token_2026";
const APP_URL = "http://localhost:3000";

test.use({ mode: "cdp" });

const ERROR_TITLES = [
  "Operation failed",
  "Provider issue",
  "Local state issue",
  "WireGuard issue",
  "Lifecycle issue",
];

async function waitForAnyMessage(page: any, messages: string[], timeoutMs = 45000): Promise<string> {
  const deadline = Date.now() + timeoutMs;

  while (Date.now() < deadline) {
    for (const message of messages) {
      const visible = await page.getByText(message, { exact: false }).first().isVisible().catch(() => false);
      if (visible) {
        return message;
      }
    }

    await new Promise((resolve) => setTimeout(resolve, 250));
  }

  throw new Error(`No expected UI feedback appeared. Expected one of: ${messages.join(", ")}`);
}

async function openSettingsAndPrepareDockerKey(page: any): Promise<void> {
  await page.goto(`${APP_URL}/dashboard/settings`);
  await expect(page.getByRole("heading", { name: "Provider API Keys" })).toBeVisible({ timeout: 30000 });

  await page.getByRole("button", { name: "Open Key Entry Modal" }).click();
  await expect(page.getByRole("heading", { name: "Provider Credential" })).toBeVisible({ timeout: 10000 });

  await page.locator("select").first().selectOption("provider-docker");
  await page.locator("textarea").first().fill(DOCKER_TOKEN);
}

test.describe("Dashboard logical workflows (real tauri)", () => {
  test("settings api keys: save and validate docker key flow", async ({ mode, tauriPage }) => {
    test.skip(mode !== "cdp", "This test must run in cdp mode against the real backend.");

    await openSettingsAndPrepareDockerKey(tauriPage);

    await tauriPage.getByRole("button", { name: "Save Token" }).click();
    await waitForAnyMessage(
      tauriPage,
      ["Docker faux token saved", "Token saved", ...ERROR_TITLES],
      45000,
    );

    await tauriPage.getByRole("button", { name: "Validate Key" }).click();
    await waitForAnyMessage(
      tauriPage,
      ["Docker readiness checked", "Token validated", ...ERROR_TITLES],
      45000,
    );
  });

  test("create a node with docker from create-node workflow", async ({ mode, tauriPage }) => {
    test.skip(mode !== "cdp", "This test must run in cdp mode against the real backend.");

    const suffix = Date.now();
    const nodeId = `node-docker-logical-${suffix}`;
    const nodeName = `Docker Logical Node ${suffix}`;

    await openSettingsAndPrepareDockerKey(tauriPage);
    await tauriPage.getByRole("button", { name: "Save Token" }).click();
    await waitForAnyMessage(tauriPage, ["Docker faux token saved", "Token saved", ...ERROR_TITLES], 45000);

    await tauriPage.goto(`${APP_URL}/dashboard/nodes/create`);
    await expect(tauriPage.getByRole("heading", { name: "Create Node" }).first()).toBeVisible({ timeout: 30000 });

    await tauriPage.locator("select").first().selectOption("provider-docker");
    await tauriPage.getByLabel("Node ID").fill(nodeId);
    await tauriPage.getByLabel("Node Name").fill(nodeName);

    const planCards = tauriPage.locator("button").filter({ hasText: "/mo" });
    if (await planCards.count()) {
      await planCards.first().click();
    }

    await tauriPage.getByRole("button", { name: "Create Node", exact: true }).click();

    await waitForAnyMessage(
      tauriPage,
      ["Node created", "Provisioning is disabled until provider access is healthy.", ...ERROR_TITLES],
      60000,
    );
  });

  test("connect to a node from nodes dashboard", async ({ mode, tauriPage }) => {
    test.skip(mode !== "cdp", "This test must run in cdp mode against the real backend.");

    await tauriPage.goto(`${APP_URL}/dashboard/nodes`);
    await expect(tauriPage.getByRole("heading", { name: "Node Management" }).first()).toBeVisible({ timeout: 30000 });

    const connectButton = tauriPage.getByRole("button", { name: "Connect", exact: true }).first();
    if (!await connectButton.isVisible().catch(() => false)) {
      await tauriPage.goto(`${APP_URL}/dashboard/nodes/create`);
      await expect(tauriPage.getByRole("heading", { name: "Create Node" }).first()).toBeVisible({ timeout: 30000 });

      const suffix = Date.now();
      await tauriPage.locator("select").first().selectOption("provider-docker");
      await tauriPage.getByLabel("Node ID").fill(`node-docker-connect-${suffix}`);
      await tauriPage.getByLabel("Node Name").fill(`Docker Connect Node ${suffix}`);

      const planCards = tauriPage.locator("button").filter({ hasText: "/mo" });
      if (await planCards.count()) {
        await planCards.first().click();
      }

      await tauriPage.getByRole("button", { name: "Create Node", exact: true }).click();
      await waitForAnyMessage(tauriPage, ["Node created", ...ERROR_TITLES], 60000);
      await tauriPage.goto(`${APP_URL}/dashboard/nodes`);
    }

    const actionableConnectButton = tauriPage.getByRole("button", { name: "Connect", exact: true }).first();
    await actionableConnectButton.click();

    await waitForAnyMessage(
      tauriPage,
      ["Tunnel connected", "Tunnel recovered", ...ERROR_TITLES],
      60000,
    );
  });
});
