import { expect, test } from "@playwright/test";
import { installDockerNodesCreateMock } from "./fixtures/tauri-mocks";

test.describe("Dashboard Docker nodes create", () => {
  test.skip(
    process.env.NEXT_PUBLIC_DEV_DOCKER !== "1",
    "Docker nodes tests require NEXT_PUBLIC_DEV_DOCKER=1 for frontend Docker surfaces.",
  );

  test("switches to Docker provider, loads docker-local plans, and creates a ready node", async ({ page }) => {
    await installDockerNodesCreateMock(page);

    await page.goto("/dashboard/nodes/create");

    const providerSelect = page.locator("select").first();
    const regionSelect = page.locator("label:has-text('Region') select");

    await providerSelect.selectOption("provider-docker");
    await expect(page.getByText("Docker", { exact: true })).toBeVisible();

    await expect(regionSelect).toHaveValue("docker-local");
    await expect(regionSelect.locator("option")).toHaveCount(1);
    await expect(regionSelect.locator("option").first()).toHaveText("docker-local");

    const dockerSmallPlan = page.getByRole("button", { name: /Docker Small/i });
    const dockerMediumPlan = page.getByRole("button", { name: /Docker Medium/i });
    const dockerLargePlan = page.getByRole("button", { name: /Docker Large/i });

    await expect(dockerSmallPlan).toBeVisible();
    await expect(dockerMediumPlan).toBeVisible();
    await expect(dockerLargePlan).toBeVisible();

    await dockerMediumPlan.click();

    await page.locator("label:has-text('Node ID') input").fill("node-docker-1");
    await page.locator("label:has-text('Node Name') input").fill("Docker E2E Node");
    await page.getByRole("button", { name: "Create Node", exact: true }).click();

    const lastCreateRequest = await page.evaluate(() => {
      return (window as unknown as { __TEST_DOCKER_LAST_CREATE_REQUEST__?: Record<string, unknown> | null })
        .__TEST_DOCKER_LAST_CREATE_REQUEST__ ?? null;
    });

    expect(lastCreateRequest).toEqual({
      nodeId: "node-docker-1",
      providerId: "provider-docker",
      nodeName: "Docker E2E Node",
      region: "docker-local",
      planKey: "docker-medium",
    });

    await page.getByRole("button", { name: "Back to Nodes" }).first().click();

    await expect(page.getByRole("heading", { name: "Node Management" })).toBeVisible();
    const nodeRow = page.getByRole("row", { name: /Docker E2E Node/i });
    await expect(nodeRow).toBeVisible();
    await expect(nodeRow).toContainText("ready");
    await expect(nodeRow).toContainText("docker-medium");
    await expect(nodeRow).toContainText("172.18.0.10");

    const commandCounts = await page.evaluate(() => {
      return (window as unknown as {
        __TEST_DOCKER_CREATE_COUNTS__?: {
          list_provider_regions: number;
          list_provider_pricing: number;
          create_vpn_node_to_ready: number;
        };
      }).__TEST_DOCKER_CREATE_COUNTS__ ?? null;
    });

    expect(commandCounts).not.toBeNull();
    expect(commandCounts?.create_vpn_node_to_ready).toBe(1);
    expect(commandCounts?.list_provider_regions).toBeGreaterThanOrEqual(1);
    expect(commandCounts?.list_provider_pricing).toBeGreaterThanOrEqual(1);
  });
});
