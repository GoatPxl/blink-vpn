import { expect, test } from "./fixtures/tauri-real";

const DOCKER_TOKEN = "dkr_v1_local_dev_mock_token_2026";
const APP_URL = "http://localhost:3000";
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

test.use({ mode: "cdp" });

test.describe("Docker real tauri flow", () => {
  test("saves docker key, provisions host, records init, and connects tunnel", async ({ mode, tauriPage }) => {
    test.skip(mode !== "cdp", "This test must run in cdp mode against the real backend.");

    const suffix = Date.now();
    const nodeId = `node-docker-real-${suffix}`;
    const nodeName = `Docker Real Node ${suffix}`;

    await tauriPage.goto(`${APP_URL}/dashboard/settings`);

    await tauriPage.getByRole("button", { name: "Open Key Entry Modal" }).click();
    await tauriPage.locator("select").first().selectOption("provider-docker");
    await tauriPage.locator("textarea").first().fill(DOCKER_TOKEN);
    await tauriPage.getByRole("button", { name: "Save Token" }).click();
    await waitForAnyMessage(tauriPage, ["Docker faux token saved", "Token saved", ...ERROR_TITLES], 45000);

    await tauriPage.getByRole("button", { name: "Validate Key" }).click();
    await waitForAnyMessage(tauriPage, ["Docker readiness checked", "Token validated", ...ERROR_TITLES], 45000);

    await tauriPage.goto(`${APP_URL}/dashboard/nodes/create`);

    const providerSelect = tauriPage.locator("select").first();
    await providerSelect.selectOption("provider-docker");

    await tauriPage.locator("input").nth(0).fill(nodeId);
    await tauriPage.locator("input").nth(1).fill(nodeName);

    await tauriPage.getByRole("button", { name: "Create Node" }).click();
    await waitForAnyMessage(tauriPage, ["Node created", "Provisioning is disabled until provider access is healthy.", ...ERROR_TITLES], 60000);

    await tauriPage.goto(`${APP_URL}/dashboard/nodes`);

    const nodeRow = tauriPage.getByRole("row", { name: nodeName });
    await expect(nodeRow).toBeVisible({ timeout: 60000 });

    const diagnostics = await tauriPage.evaluate<{
      init_runs?: Array<{ node_id: string; provider_slug: string; stage: string; outcome: string }>;
    }>("(async () => await window.__TAURI_INTERNALS__.invoke('get_diagnostics_bundle'))()");

    const initRuns = diagnostics.init_runs ?? [];
    const initReadyFound = initRuns.some(
      (run) =>
        run.node_id === nodeId
        && run.provider_slug === "docker"
        && run.stage === "bootstrap.wait"
        && run.outcome === "success",
    );
    expect(initReadyFound).toBe(true);

    await nodeRow.getByRole("button", { name: "Connect", exact: true }).click();
    await waitForAnyMessage(tauriPage, ["Tunnel connected", "Tunnel recovered", ...ERROR_TITLES], 60000);

    const postConnectDiagnostics = await tauriPage.evaluate<{
      tunnels?: Array<{ node_id: string; status: string }>;
    }>("(async () => await window.__TAURI_INTERNALS__.invoke('get_diagnostics_bundle'))()");

    const tunnel = (postConnectDiagnostics.tunnels ?? []).find((item) => item.node_id === nodeId) ?? null;
    expect(tunnel).not.toBeNull();
    expect(tunnel?.status.toLowerCase()).toBe("connected");
  });
});
