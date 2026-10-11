import { expect, test } from "./fixtures/tauri-real";
import {
  createDockerNode,
  invokeCommand,
  readDiagnostics,
  resetNodes,
  saveDockerCredential,
} from "./fixtures/dashboard";

type SavedLocation = {
  id: string;
  name: string;
  region: string;
  plan_key: string;
  node_id: string | null;
  node_status: string | null;
};

/**
 * Turning a server off and back on through the real app and the real Docker daemon.
 *
 * Asserts backend state, not UI text: whether a server exists and which location it belongs to are
 * facts the UI only summarizes. Each step checks the specific outcome — "off" means the node row is
 * gone *and* the location survived, not merely that no error appeared.
 */
test.use({ mode: "cdp" });

test.describe("Saved locations (real backend)", () => {
  test("a server is saved on ready, turns off, and turns back on", async ({ tauriPage }) => {
    const suffix = Date.now();
    const nodeName = `E2E Location ${suffix}`;

    await resetNodes(tauriPage);
    await saveDockerCredential(tauriPage);
    const nodeId = await createDockerNode(tauriPage, nodeName);

    const findMine = async () => {
      const locations = await invokeCommand<SavedLocation[]>(tauriPage, "list_saved_locations");
      return locations.filter((location) => location.name === nodeName);
    };

    // Saved automatically when the server reached ready, linked to it.
    const [saved] = await findMine();
    expect(saved, "reaching ready should save the location").toBeTruthy();
    expect(saved.node_id).toBe(nodeId);
    expect(saved.node_status).toBe("ready");

    // Off: the server is gone, the location is not.
    await invokeCommand(tauriPage, "turn_off_saved_location", { savedLocationId: saved.id });
    const afterOff = await findMine();
    expect(afterOff).toHaveLength(1);
    expect(afterOff[0].node_id).toBeNull();
    expect(afterOff[0].node_status).toBeNull();
    const nodesAfterOff = (await readDiagnostics(tauriPage)).nodes ?? [];
    expect(nodesAfterOff.some((node) => node.id === nodeId), "the server should be deleted").toBe(false);

    // On: a new server, linked to the same location rather than a second one.
    const secondNodeId = `${nodeId}-again`;
    await invokeCommand(tauriPage, "turn_on_saved_location", {
      savedLocationId: saved.id,
      nodeId: secondNodeId,
    });
    const afterOn = await findMine();
    expect(afterOn).toHaveLength(1);
    expect(afterOn[0].id).toBe(saved.id);
    expect(afterOn[0].node_id).toBe(secondNodeId);
    expect(afterOn[0].node_status).toBe("ready");

    // Leave nothing running, and nothing saved, behind.
    await invokeCommand(tauriPage, "turn_off_saved_location", { savedLocationId: saved.id });
    await invokeCommand(tauriPage, "forget_saved_location", { savedLocationId: saved.id });
    expect(await findMine()).toHaveLength(0);
  });
});
