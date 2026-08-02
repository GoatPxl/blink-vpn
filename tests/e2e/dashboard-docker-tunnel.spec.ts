import { expect, test } from "@playwright/test";
import { installDockerTunnelLifecycleMock } from "./fixtures/tauri-mocks";

test.describe("Dashboard Docker tunnel lifecycle", () => {
  test.skip(
    process.env.NEXT_PUBLIC_DEV_DOCKER !== "1",
    "Docker tunnel tests require NEXT_PUBLIC_DEV_DOCKER=1 for frontend Docker surfaces.",
  );

  test("connects and disconnects a Docker tunnel while health panel stats transition", async ({ page }) => {
    await installDockerTunnelLifecycleMock(page);

    await page.goto("/dashboard/nodes");

    const nodeRow = page.getByRole("row", { name: /Docker Tunnel Node/i });
    await expect(nodeRow).toBeVisible();

    const connectButton = nodeRow.getByRole("button", { name: "Connect", exact: true });
    await expect(connectButton).toBeVisible();
    await connectButton.click();

    await expect(page.getByText("Tunnel connected")).toBeVisible();
    await expect(nodeRow.getByRole("button", { name: "Disconnect", exact: true })).toBeVisible();

    const postConnectState = await page.evaluate(() => {
      const status = (window as unknown as { __TEST_DOCKER_TUNNEL_STATUS__?: string }).__TEST_DOCKER_TUNNEL_STATUS__;
      const counts = (window as unknown as {
        __TEST_DOCKER_TUNNEL_COUNTS__?: {
          connect_tunnel: number;
          disconnect_tunnel: number;
          get_health_panel_data: number;
        };
      }).__TEST_DOCKER_TUNNEL_COUNTS__;

      return { status, counts };
    });

    expect(postConnectState.status).toBe("connected");
    expect(postConnectState.counts?.connect_tunnel).toBe(1);

    await page.getByRole("link", { name: /Kill Switch Connection/i }).click();
    await page.getByRole("button", { name: "Run Again" }).click();

    await expect(page.getByText("Tunnel status connected")).toBeVisible();
    await expect(page.getByText("7s ago")).toBeVisible();
    await expect(page.getByText("RX 8.0 KiB / TX 16.0 KiB")).toBeVisible();

    await page.getByRole("link", { name: /Server Locations Provision/i }).click();

    const disconnectButton = nodeRow.getByRole("button", { name: "Disconnect", exact: true });
    await expect(disconnectButton).toBeVisible();
    await disconnectButton.click();

    await expect(page.getByText("Tunnel disconnected")).toBeVisible();
    await expect(nodeRow.getByRole("button", { name: "Connect", exact: true })).toBeVisible();

    const postDisconnectState = await page.evaluate(() => {
      const status = (window as unknown as { __TEST_DOCKER_TUNNEL_STATUS__?: string }).__TEST_DOCKER_TUNNEL_STATUS__;
      const counts = (window as unknown as {
        __TEST_DOCKER_TUNNEL_COUNTS__?: {
          connect_tunnel: number;
          disconnect_tunnel: number;
          get_health_panel_data: number;
        };
      }).__TEST_DOCKER_TUNNEL_COUNTS__;

      return { status, counts };
    });

    expect(postDisconnectState.status).toBe("disconnected");
    expect(postDisconnectState.counts?.disconnect_tunnel).toBe(1);

    await page.getByRole("link", { name: /Kill Switch Connection/i }).click();
    await page.getByRole("button", { name: "Run Again" }).click();

    await expect(page.getByText("Tunnel status disconnected")).toBeVisible();
    await expect(page.getByText("RX n/a / TX n/a")).toBeVisible();

    const commandCounts = await page.evaluate(() => {
      return (window as unknown as {
        __TEST_DOCKER_TUNNEL_COUNTS__?: {
          connect_tunnel: number;
          disconnect_tunnel: number;
          get_health_panel_data: number;
        };
      }).__TEST_DOCKER_TUNNEL_COUNTS__ ?? null;
    });

    expect(commandCounts).not.toBeNull();
    expect(commandCounts?.connect_tunnel).toBe(1);
    expect(commandCounts?.disconnect_tunnel).toBe(1);
    expect(commandCounts?.get_health_panel_data).toBeGreaterThanOrEqual(2);
  });
});
