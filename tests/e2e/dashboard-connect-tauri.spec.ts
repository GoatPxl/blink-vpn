import { expect, test } from "./fixtures/tauri-real";
import {
  APP_URL,
  createDockerNode,
  expectSuccess,
  invokeCommand,
  readDiagnostics,
  resetNodes,
  saveDockerCredential,
  setPreferences,
  settleToasts,
  type AppPreferences,
} from "./fixtures/dashboard";

/**
 * Connecting a real tunnel, end to end.
 *
 * **Requires elevation.** Installing and removing a WireGuard tunnel service is an administrator
 * operation. Launch the app from an elevated shell and it completes with no prompts, because the
 * backend attempts each privileged command directly and only escalates when one is refused. Run
 * it unelevated and Windows shows a consent dialog that Playwright cannot answer, so the test
 * will hang until someone approves it.
 */
test.use({ mode: "cdp" });

test.describe("Docker tunnel connect (real backend, needs elevation)", () => {
  test("connects a node and completes a WireGuard handshake", async ({ tauriPage }) => {
    const suffix = Date.now();
    const nodeId = `node-connect-${suffix}`;
    const nodeName = `Connect Node ${suffix}`;

    await resetNodes(tauriPage);
    await saveDockerCredential(tauriPage);
    await createDockerNode(tauriPage, nodeId, nodeName);
    // The explainer is on, as on a fresh install, so this connect must pass through it.
    await setPreferences(tauriPage, { explain_uac: true });

    await tauriPage.goto(`${APP_URL}/dashboard/map`);
    await expect(tauriPage.getByRole("heading", { name: "Server Locations" }).first()).toBeVisible({
      timeout: 30_000,
    });

    // By id, not by name: the row leads with location and only shows the node name to
    // disambiguate servers sharing a region.
    const nodeRow = tauriPage.locator(`tr[data-node-id="${nodeId}"]`);
    await expect(nodeRow).toBeVisible({ timeout: 30_000 });

    await settleToasts(tauriPage);
    await nodeRow.getByRole("button", { name: "Connect", exact: true }).click();

    // Nothing may connect before the explainer is answered, and answering it with the box ticked
    // turns it off for good.
    const explainer = tauriPage.getByRole("dialog", { name: "One Windows prompt is coming" });
    await expect(explainer).toBeVisible({ timeout: 10_000 });
    await explainer.getByRole("button", { name: "Continue", exact: true }).click();
    await expectSuccess(tauriPage, "Tunnel connected", 120_000);
    const preferences = await invokeCommand<AppPreferences>(tauriPage, "get_app_preferences");
    expect(preferences.explain_uac).toBe(false);
    expect(preferences.first_connect_done).toBe(true);

    // The row must report the connection, not merely that the container is powered on — the badge
    // used to render the provider's VM state, which reads "active" either way.
    await expect(nodeRow.getByText("Connected", { exact: true })).toBeVisible({ timeout: 30_000 });

    const connected = await readDiagnostics(tauriPage);
    const tunnel = (connected.tunnels ?? []).find((candidate) => candidate.node_id === nodeId);

    expect(tunnel, "a tunnel should exist for the connected node").toBeTruthy();
    expect(tunnel?.status).toBe("connected");

    // The real proof. A tunnel service can install, start, and report "connected" while never
    // completing a handshake — which is exactly what happened when the endpoint was unreachable.
    // Only a handshake age shows the server actually answered.
    expect(
      tunnel?.last_handshake_age_seconds,
      "no handshake completed: the server never answered, so the tunnel is up but carries nothing",
    ).not.toBeNull();
    expect(tunnel?.last_handshake_age_seconds ?? Number.MAX_SAFE_INTEGER).toBeLessThan(180);

    // A handshake moves bytes in both directions.
    expect(tunnel?.rx_bytes ?? 0).toBeGreaterThan(0);
    expect(tunnel?.tx_bytes ?? 0).toBeGreaterThan(0);

    await settleToasts(tauriPage);
    await nodeRow.getByRole("button", { name: "Disconnect", exact: true }).click();
    await expectSuccess(tauriPage, "Tunnel disconnected", 120_000);

    const afterDisconnect = await readDiagnostics(tauriPage);
    const disconnected = (afterDisconnect.tunnels ?? []).find(
      (candidate) => candidate.node_id === nodeId,
    );
    expect(disconnected?.status).toBe("disconnected");
  });

  test("disconnects from the dashboard button after connecting a fresh node there", async ({ tauriPage }) => {
    const suffix = Date.now();
    const nodeId = `node-dash-connect-${suffix}`;
    const nodeName = `Dash Connect Node ${suffix}`;

    await resetNodes(tauriPage);
    await saveDockerCredential(tauriPage);
    // Fresh node with no tunnel row: the first connect creates it. That is the case that broke —
    // the dashboard never reloaded its tunnel list afterwards, so "Disconnect" had nothing to stop
    // and fell through to connecting again.
    await createDockerNode(tauriPage, nodeId, nodeName);
    await setPreferences(tauriPage, { explain_uac: false });

    await tauriPage.goto(`${APP_URL}/dashboard`);
    const main = tauriPage.locator("main");
    const connectButton = main.getByRole("button", { name: "Connect", exact: true });
    await expect(connectButton).toBeEnabled({ timeout: 30_000 });

    await settleToasts(tauriPage);
    await connectButton.click();
    await expectSuccess(tauriPage, "Tunnel connected", 120_000);

    const disconnectButton = main.getByRole("button", { name: "Disconnect", exact: true });
    await expect(disconnectButton).toBeEnabled({ timeout: 30_000 });

    const connectStartsFor = (bundle: Awaited<ReturnType<typeof readDiagnostics>>) =>
      (bundle.activity ?? []).filter(
        (event) => event.node_id === nodeId && event.stage === "connect.start",
      ).length;
    const connectStartsBefore = connectStartsFor(await readDiagnostics(tauriPage));

    await settleToasts(tauriPage);
    await disconnectButton.click();
    // The specific toast of the tunnel-scoped disconnect, not the node-wide fallback's — this
    // proves the tunnel list was actually reloaded after the connect created it.
    await expectSuccess(tauriPage, "Tunnel disconnected", 120_000);

    const afterDisconnect = await readDiagnostics(tauriPage);
    const tunnel = (afterDisconnect.tunnels ?? []).find((candidate) => candidate.node_id === nodeId);
    expect(tunnel?.status).toBe("disconnected");

    // The broken build reported success here too, by reconnecting an already-running tunnel. A
    // new connect.start, or a missing disconnect.completed, is that regression.
    expect(connectStartsFor(afterDisconnect)).toBe(connectStartsBefore);
    expect(
      (afterDisconnect.activity ?? []).some(
        (event) => event.node_id === nodeId && event.stage === "disconnect.completed",
      ),
    ).toBe(true);

    await expect(connectButton).toBeEnabled({ timeout: 30_000 });
  });

  test("connects by itself when a server it built is ready", async ({ tauriPage }) => {
    const suffix = Date.now();
    const nodeId = `node-auto-connect-${suffix}`;
    const nodeName = `Auto Connect Node ${suffix}`;

    await resetNodes(tauriPage);
    await saveDockerCredential(tauriPage);
    await tauriPage.goto(`${APP_URL}/dashboard`);
    await setPreferences(tauriPage, { explain_uac: false });

    // No Connect click anywhere in this test: the build alone must end in a working tunnel.
    await createDockerNode(tauriPage, nodeId, nodeName, { autoConnect: true });
    await expectSuccess(tauriPage, "Tunnel connected", 120_000);

    const connected = await readDiagnostics(tauriPage);
    const tunnel = (connected.tunnels ?? []).find((candidate) => candidate.node_id === nodeId);
    expect(tunnel?.status).toBe("connected");
    expect(tunnel?.rx_bytes ?? 0, "a connected tunnel must have received traffic").toBeGreaterThan(0);

    await tauriPage.goto(`${APP_URL}/dashboard`);
    await settleToasts(tauriPage);
    await tauriPage.locator("main").getByRole("button", { name: "Disconnect", exact: true }).click();
    await expectSuccess(tauriPage, "Tunnel disconnected", 120_000);
  });
});
