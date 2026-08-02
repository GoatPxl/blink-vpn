import { expect, test } from "@playwright/test";
import { installDockerDestroyCleanupMock } from "./fixtures/tauri-mocks";

test.describe("Dashboard Docker destroy and force cleanup", () => {
  test.skip(
    process.env.NEXT_PUBLIC_DEV_DOCKER !== "1",
    "Docker destroy/cleanup tests require NEXT_PUBLIC_DEV_DOCKER=1 for frontend Docker surfaces.",
  );

  test("destroys and force-cleans Docker nodes with idempotent reruns", async ({ page }) => {
    await installDockerDestroyCleanupMock(page);

    await page.goto("/dashboard/nodes");

    const destroyRow = page.getByRole("row", { name: /Docker Destroy Node/i });
    await expect(destroyRow).toBeVisible();

    page.once("dialog", (dialog) => dialog.accept());
    await destroyRow.getByRole("button", { name: "Destroy", exact: true }).click();

    await expect(page.getByText("Node destroyed")).toBeVisible();
    await expect(destroyRow).toContainText("deleting");

    const secondDestroy = await page.evaluate(async () => {
      const internals = (window as unknown as {
        __TAURI_INTERNALS__?: { invoke?: (command: string, args?: Record<string, unknown>) => Promise<unknown> };
      }).__TAURI_INTERNALS__;

      if (!internals?.invoke) {
        return { ok: false, reason: "missing invoke" };
      }

      try {
        await internals.invoke("destroy_vpn_node", { nodeId: "node-docker-destroy-1" });
        return { ok: true };
      } catch (error) {
        return { ok: false, reason: error instanceof Error ? error.message : String(error) };
      }
    });

    expect(secondDestroy).toEqual({ ok: true });

    const forceCleanupRuns = await page.evaluate(async () => {
      const internals = (window as unknown as {
        __TAURI_INTERNALS__?: { invoke?: (command: string, args?: Record<string, unknown>) => Promise<unknown> };
      }).__TAURI_INTERNALS__;

      if (!internals?.invoke) {
        return { first: null, second: null, error: "missing invoke" };
      }

      try {
        const first = await internals.invoke("force_cleanup_vpn_node", { nodeId: "node-docker-recovery-1" });
        const second = await internals.invoke("force_cleanup_vpn_node", { nodeId: "node-docker-recovery-1" });
        return { first, second, error: null };
      } catch (error) {
        return {
          first: null,
          second: null,
          error: error instanceof Error ? error.message : String(error),
        };
      }
    });

    expect(forceCleanupRuns.error).toBeNull();
    expect(forceCleanupRuns.first).toMatchObject({
      node_id: "node-docker-recovery-1",
      local_tunnels_removed: 1,
      remote_droplet_delete_attempted: true,
      remote_droplet_delete_succeeded: true,
      node_record_removed: false,
    });
    expect(forceCleanupRuns.second).toMatchObject({
      node_id: "node-docker-recovery-1",
      local_tunnels_removed: 0,
      remote_droplet_delete_attempted: true,
      remote_droplet_delete_succeeded: true,
      node_record_removed: false,
    });

    await page.getByRole("button", { name: "Refresh dashboard" }).click();
    await expect(page.getByRole("row", { name: /Docker Recovery Node/i })).toContainText("error");

    const commandCounts = await page.evaluate(() => {
      return (window as unknown as {
        __TEST_DOCKER_DESTROY_CLEANUP_COUNTS__?: {
          destroy_vpn_node: number;
          force_cleanup_vpn_node: number;
          list_vpn_nodes: number;
        };
      }).__TEST_DOCKER_DESTROY_CLEANUP_COUNTS__ ?? null;
    });

    expect(commandCounts).not.toBeNull();
    expect(commandCounts?.destroy_vpn_node).toBe(2);
    expect(commandCounts?.force_cleanup_vpn_node).toBe(2);
    expect(commandCounts?.list_vpn_nodes).toBeGreaterThanOrEqual(1);
  });
});
