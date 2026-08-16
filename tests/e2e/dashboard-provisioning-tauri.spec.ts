import { expect, test } from "./fixtures/tauri-real";
import {
  APP_URL,
  createDockerNode,
  readDiagnostics,
  resetNodes,
  saveDockerCredential,
} from "./fixtures/dashboard";

/**
 * Provisioning through the real app against the real Docker daemon.
 *
 * One test covering one flow, because each test spawns its own app instance and provisioning
 * takes most of a minute. Connecting a tunnel needs elevation and lives in
 * dashboard-connect-tauri.spec.ts.
 */
test.use({ mode: "cdp" });

test.describe("Docker provisioning (real backend)", () => {
  test("saves a credential, provisions a node, and records a reachable endpoint", async ({
    tauriPage,
  }) => {
    const suffix = Date.now();
    const nodeId = `node-e2e-${suffix}`;
    const nodeName = `E2E Node ${suffix}`;

    await resetNodes(tauriPage);
    await saveDockerCredential(tauriPage);
    await createDockerNode(tauriPage, nodeId, nodeName);

    const diagnostics = await readDiagnostics(tauriPage);
    const node = (diagnostics.nodes ?? []).find((candidate) => candidate.id === nodeId);

    expect(node, `node ${nodeId} should exist in diagnostics`).toBeTruthy();
    expect(node?.lifecycle_status).toBe("ready");

    // The server keypair is generated locally and baked into the payload, so its public key is
    // known before the container exists and must be recorded without any read-back.
    expect(node?.server_public_key, "server public key should be recorded").toBeTruthy();

    // Docker publishes an ephemeral host port. Recording the container's own 51820 would mean the
    // client dials a port nothing is listening on.
    expect(node?.endpoint_port, "an endpoint port should be recorded").toBeTruthy();
    expect(node?.endpoint_port).not.toBe(51820);

    // A loopback endpoint is unreachable when the daemon runs inside WSL2, which is what made
    // every handshake disappear. The adapter must report an address the client can actually dial.
    expect(node?.public_ipv4, "an endpoint address should be recorded").toBeTruthy();
    expect(node?.public_ipv4).not.toBe("0.0.0.0");

    // Bootstrap readiness is resolved from provider metadata, not by reading the server.
    const bootstrapReady = (diagnostics.init_runs ?? []).some(
      (run) =>
        run.node_id === nodeId
        && run.provider_slug === "docker"
        && run.stage === "bootstrap.wait"
        && run.outcome === "success",
    );
    expect(bootstrapReady, "bootstrap.wait should have succeeded").toBe(true);

    // The node the backend reports must actually reach the fleet list. Located by id rather than
    // by name: the row leads with location and only shows the name to disambiguate same-region
    // servers, so a name match would pass or fail depending on what else is provisioned.
    await tauriPage.goto(`${APP_URL}/dashboard/nodes`);
    await expect(tauriPage.getByRole("heading", { name: "Server Locations" }).first()).toBeVisible({
      timeout: 30_000,
    });

    const nodeRow = tauriPage.locator(`tr[data-node-id="${nodeId}"]`);
    await expect(nodeRow).toBeVisible({ timeout: 30_000 });
    // A freshly provisioned node is usable and not connected. "Ready" is the only correct badge;
    // accepting any status would let a broken provision pass.
    await expect(nodeRow.getByText("Ready", { exact: true })).toBeVisible({ timeout: 30_000 });

    // The name still has to be reachable, just from the details drawer rather than the row.
    await nodeRow.click();
    await expect(tauriPage.getByRole("heading", { name: nodeName })).toBeVisible({ timeout: 15_000 });
  });
});
