import { expect, test } from "./fixtures/tauri-real";

const DOCKER_TOKEN = "dkr_v1_local_dev_mock_token_2026";
const APP_URL = "http://localhost:3000";

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
    await expect(tauriPage.getByText("Docker faux token saved")).toBeVisible({ timeout: 30000 });

    await tauriPage.getByRole("button", { name: "Validate Key" }).click();
    await expect(tauriPage.getByText("Docker readiness checked")).toBeVisible({ timeout: 30000 });

    await tauriPage.goto(`${APP_URL}/dashboard/nodes/create`);

    const providerSelect = tauriPage.locator("select").first();
    await providerSelect.selectOption("provider-docker");

    await tauriPage.locator("input").nth(0).fill(nodeId);
    await tauriPage.locator("input").nth(1).fill(nodeName);

    await tauriPage.getByRole("button", { name: "Create Node" }).click();
    await expect(tauriPage.getByText("Node created")).toBeVisible({ timeout: 60000 });

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
    await expect(tauriPage.getByText("Tunnel connected")).toBeVisible({ timeout: 60000 });

    const postConnectDiagnostics = await tauriPage.evaluate<{
      tunnels?: Array<{ node_id: string; status: string }>;
    }>("(async () => await window.__TAURI_INTERNALS__.invoke('get_diagnostics_bundle'))()");

    const tunnel = (postConnectDiagnostics.tunnels ?? []).find((item) => item.node_id === nodeId) ?? null;
    expect(tunnel).not.toBeNull();
    expect(tunnel?.status.toLowerCase()).toBe("connected");
  });
});
