import { expect, test } from "@playwright/test";
import { installDockerHostInstallAndConnectMock } from "./fixtures/tauri-mocks";

const DOCKER_API_TOKEN = "dkr_v1_local_dev_mock_token_2026";

test.describe("Dashboard Docker host install and connect", () => {
  test.skip(
    process.env.NEXT_PUBLIC_DEV_DOCKER !== "1",
    "Docker host install/connect tests require NEXT_PUBLIC_DEV_DOCKER=1 for frontend Docker surfaces.",
  );

  test("installs the Docker host then connects the local WireGuard client", async ({ page }) => {
    await installDockerHostInstallAndConnectMock(page, DOCKER_API_TOKEN);

    await page.goto("/dashboard/nodes");

    const nodeRow = page.getByRole("row", { name: /Docker Host Node/i });
    await expect(nodeRow).toBeVisible();

    const installHostButton = nodeRow.getByRole("button", { name: "Install Host", exact: true });
    await expect(installHostButton).toBeVisible();
    await installHostButton.click();

    await expect(page.getByText("Host installed")).toBeVisible();
    await expect(nodeRow).toContainText("ready");
    await expect(nodeRow).toContainText("127.0.0.1");
    await expect(nodeRow.getByRole("button", { name: "No Tunnel", exact: true })).toBeVisible();

    const postInstallState = await page.evaluate(() => {
      return {
        commandCounts: (window as unknown as {
          __TEST_DOCKER_HOST_CONNECT_COUNTS__?: {
            create_vpn_node_to_ready: number;
            create_and_activate_client_tunnel: number;
            connect_tunnel: number;
            get_health_panel_data: number;
          };
        }).__TEST_DOCKER_HOST_CONNECT_COUNTS__ ?? null,
        hostCreateRequest: (window as unknown as {
          __TEST_DOCKER_HOST_CREATE_REQUEST__?: Record<string, unknown> | null;
        }).__TEST_DOCKER_HOST_CREATE_REQUEST__ ?? null,
      };
    });

    expect(postInstallState.commandCounts?.create_vpn_node_to_ready).toBe(1);
    expect(postInstallState.hostCreateRequest).toEqual({
      nodeId: "node-docker-host-1",
      providerId: "provider-docker",
      nodeName: "Docker Host Node",
      region: "docker-local",
      planKey: "docker-small",
    });

    await page.getByRole("link", { name: /Dashboard Overview/i }).click();

    const connectButton = page.getByRole("button", { name: "Connect", exact: true });
    await expect(connectButton).toBeVisible();
    await connectButton.click();

    await expect(page.getByText("Tunnel connected")).toBeVisible();

    const postConnectState = await page.evaluate(() => {
      return {
        commandCounts: (window as unknown as {
          __TEST_DOCKER_HOST_CONNECT_COUNTS__?: {
            create_vpn_node_to_ready: number;
            create_and_activate_client_tunnel: number;
            connect_tunnel: number;
            get_health_panel_data: number;
          };
        }).__TEST_DOCKER_HOST_CONNECT_COUNTS__ ?? null,
        tunnelCreateRequest: (window as unknown as {
          __TEST_DOCKER_TUNNEL_CREATE_REQUEST__?: Record<string, unknown> | null;
        }).__TEST_DOCKER_TUNNEL_CREATE_REQUEST__ ?? null,
        nodeState: (window as unknown as { __TEST_DOCKER_CURRENT_NODE_STATE__?: string }).__TEST_DOCKER_CURRENT_NODE_STATE__ ?? null,
      };
    });

    expect(postConnectState.commandCounts?.create_and_activate_client_tunnel).toBe(1);
    expect(postConnectState.commandCounts?.connect_tunnel).toBeLessThanOrEqual(1);
    expect(postConnectState.tunnelCreateRequest?.nodeId).toBe("node-docker-host-1");
    expect(postConnectState.nodeState).toBe("connected");
  });
});
