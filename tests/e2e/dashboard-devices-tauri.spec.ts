import { expect, test } from "./fixtures/tauri-real";
import {
  createDockerNode,
  invokeCommand,
  resetNodes,
  saveDockerCredential,
} from "./fixtures/dashboard";

type NodePeer = {
  id: string;
  node_id: string;
  slot_index: number;
  label: string | null;
  client_public_key: string;
  client_address_cidr: string;
  claimed_at: string | null;
};

type AppPreferences = {
  close_to_tray: boolean;
  mobile_devices: string[];
};

/**
 * Runs `body` with the saved device list set to `devices`, then restores whatever was saved before.
 *
 * Preferences live in the app's real database and persist across runs, so a test that depends on
 * them must set them explicitly — and must not leave its own list behind for the user.
 */
async function withSavedDevices(
  page: Parameters<typeof invokeCommand>[0],
  devices: string[],
  body: () => Promise<void>,
): Promise<void> {
  const original = await invokeCommand<AppPreferences>(page, "get_app_preferences");
  await invokeCommand(page, "set_app_preferences", {
    closeToTray: original.close_to_tray,
    mobileDevices: devices,
  });
  try {
    await body();
  } finally {
    await invokeCommand(page, "set_app_preferences", {
      closeToTray: original.close_to_tray,
      mobileDevices: original.mobile_devices,
    });
  }
}

/**
 * Device slots through the real app against the real Docker daemon.
 *
 * Asserts backend state rather than UI text, because the interesting facts here — how many slots a
 * node was born with, and which addresses they hold — are only summarized on screen.
 *
 * The QR itself is deliberately *not* exercised: a Docker node publishes on a loopback address, and
 * `get_peer_config` refuses those on purpose. That refusal is the last assertion below.
 */
test.use({ mode: "cdp" });

test.describe("Device slots (real backend)", () => {
  test("provisions a full slate of slots, claims one, and refuses a config for a local endpoint", async ({
    tauriPage,
  }) => {
    const suffix = Date.now();
    const nodeId = `node-e2e-${suffix}`;

    await resetNodes(tauriPage);
    await saveDockerCredential(tauriPage);
    // No saved devices, so every spare slot starts free.
    await withSavedDevices(tauriPage, [], () =>
      createDockerNode(tauriPage, nodeId, `E2E Devices ${suffix}`),
    );

    const peers = await invokeCommand<NodePeer[]>(tauriPage, "list_node_peers", { nodeId });

    // Every slot the node will ever have is baked at create time; it cannot gain more.
    expect(peers.length, "a node should be provisioned with a full slate of slots").toBe(5);
    expect(peers.map((peer) => peer.slot_index)).toEqual([0, 1, 2, 3, 4]);

    // Slot 0 is this machine and arrives claimed. Everything above it is free.
    expect(peers[0].label).toBe("This PC");
    expect(peers[0].claimed_at).toBeTruthy();
    expect(peers.slice(1).every((peer) => peer.label === null)).toBe(true);

    // Distinct addresses and keys, or two devices would collide on the server.
    const addresses = new Set(peers.map((peer) => peer.client_address_cidr));
    const keys = new Set(peers.map((peer) => peer.client_public_key));
    expect(addresses.size, "each slot needs its own address").toBe(5);
    expect(keys.size, "each slot needs its own key").toBe(5);
    expect(peers[0].client_address_cidr).toBe("10.66.0.2/32");
    expect(peers[1].client_address_cidr).toBe("10.66.0.3/32");

    // Claiming is local bookkeeping: it names an existing slot and contacts nothing.
    const claimed = await invokeCommand<NodePeer>(tauriPage, "claim_node_peer", {
      nodeId,
      peerId: peers[1].id,
      label: "E2E Phone",
    });
    expect(claimed.label).toBe("E2E Phone");
    expect(claimed.claimed_at).toBeTruthy();
    // The address is fixed at provisioning and a claim must not move it.
    expect(claimed.client_address_cidr).toBe(peers[1].client_address_cidr);

    const afterClaim = await invokeCommand<NodePeer[]>(tauriPage, "list_node_peers", { nodeId });
    expect(afterClaim.filter((peer) => peer.label !== null).length).toBe(2);

    // A Docker node is reachable only from this machine. A QR for it would import cleanly on a
    // phone and then never hand shake, so the command refuses rather than handing out a config
    // that cannot work.
    const refusal = await tauriPage.evaluate(
      async ([id, peerId]) => {
        try {
          await (
            window as unknown as {
              __TAURI_INTERNALS__: {
                invoke: (command: string, args: Record<string, unknown>) => Promise<unknown>;
              };
            }
          ).__TAURI_INTERNALS__.invoke("get_peer_config", { nodeId: id, peerId });
          return { refused: false, kind: null as string | null };
        } catch (error) {
          const payload = error as { kind?: string };
          return { refused: true, kind: payload?.kind ?? null };
        }
      },
      [nodeId, peers[1].id],
    );

    expect(refusal.refused, "a loopback endpoint must not produce a device config").toBe(true);
    // Keyed on the structured kind, never on message text.
    expect(refusal.kind).toBe("invalid-input");
  });

  test("names a new server's spare slots after the saved devices", async ({ tauriPage }) => {
    const suffix = Date.now();
    const nodeId = `node-e2e-saved-${suffix}`;

    await resetNodes(tauriPage);
    await saveDockerCredential(tauriPage);
    await withSavedDevices(tauriPage, ["E2E Phone", "E2E Tablet"], () =>
      createDockerNode(tauriPage, nodeId, `E2E Saved ${suffix}`),
    );

    const peers = await invokeCommand<NodePeer[]>(tauriPage, "list_node_peers", { nodeId });

    // Saved names take the spare slots in order and arrive claimed; the rest stay free.
    expect(peers.map((peer) => peer.label)).toEqual([
      "This PC",
      "E2E Phone",
      "E2E Tablet",
      null,
      null,
    ]);
    expect(peers.slice(0, 3).every((peer) => peer.claimed_at !== null)).toBe(true);
    expect(peers.slice(3).every((peer) => peer.claimed_at === null)).toBe(true);
  });
});
