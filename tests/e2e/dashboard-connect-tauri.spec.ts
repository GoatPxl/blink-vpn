import { expect, test } from "./fixtures/tauri-real";
import {
  APP_URL,
  createDockerNode,
  expectSuccess,
  readDiagnostics,
  resetNodes,
  saveDockerCredential,
  settleToasts,
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

    await tauriPage.goto(`${APP_URL}/dashboard/nodes`);
    await expect(tauriPage.getByRole("heading", { name: "Server Locations" }).first()).toBeVisible({
      timeout: 30_000,
    });

    // By id, not by name: the row leads with location and only shows the node name to
    // disambiguate servers sharing a region.
    const nodeRow = tauriPage.locator(`tr[data-node-id="${nodeId}"]`);
    await expect(nodeRow).toBeVisible({ timeout: 30_000 });

    await settleToasts(tauriPage);
    await nodeRow.getByRole("button", { name: "Connect", exact: true }).click();
    await expectSuccess(tauriPage, "Tunnel connected", 120_000);

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
});
