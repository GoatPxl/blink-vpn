import type { Page } from "@playwright/test";
import { expect } from "@playwright/test";

export const APP_URL = "http://localhost:3000";
export const DOCKER_TOKEN = "dkr_v1_local_dev_mock_token_2026";
export const DOCKER_PROVIDER_ID = "provider-docker";

/**
 * Toast titles the UI shows for a failed operation.
 *
 * These exist so tests can fail *fast and loudly* on them. An earlier version of this suite
 * treated them as acceptable outcomes — every wait accepted "succeeded OR any of these" — so the
 * specs passed whether the app worked or not. Nothing here may ever wait for one of these as a
 * success condition.
 */
const ERROR_TITLES = [
  "Operation failed",
  "Provider issue",
  "Local state issue",
  "WireGuard issue",
  "Lifecycle issue",
  "Credential storage issue",
  "Missing prerequisite",
  "Invalid request",
];

/**
 * Read a failure from the toast region only.
 *
 * Scoped deliberately. The dashboard also renders a persistent recovery banner, so matching error
 * text anywhere on the page reports a failure left over from an earlier action — or from startup
 * reconcile — as though the action under test had just failed.
 */
export async function readToastError(page: Page): Promise<string | null> {
  const region = page.getByTestId("toast-region");

  for (const title of ERROR_TITLES) {
    const toast = region.getByText(title, { exact: false }).first();
    if (await toast.isVisible().catch(() => false)) {
      const text = await region.innerText().catch(() => title);
      return text.replace(/\s+/g, " ").trim().slice(0, 400);
    }
  }

  return null;
}

/** Wait for the toast region to clear, so a stale toast cannot be read as this action's result. */
export async function settleToasts(page: Page, timeoutMs = 8_000): Promise<void> {
  const region = page.getByTestId("toast-region");
  const deadline = Date.now() + timeoutMs;

  while (Date.now() < deadline) {
    const text = await region.innerText().catch(() => "");
    if (text.trim().length === 0) return;
    await page.waitForTimeout(250);
  }
}

/**
 * Wait for one specific success toast, failing immediately if an error toast appears instead.
 *
 * The early exit matters: without it a genuine failure just burns the full timeout and then
 * reports "not visible", hiding the backend's actual explanation.
 */
export async function expectSuccess(
  page: Page,
  successText: string,
  timeoutMs = 60_000,
): Promise<void> {
  const region = page.getByTestId("toast-region");
  const deadline = Date.now() + timeoutMs;

  while (Date.now() < deadline) {
    if (await region.getByText(successText, { exact: false }).first().isVisible().catch(() => false)) {
      return;
    }

    const failure = await readToastError(page);
    if (failure) {
      throw new Error(`Expected "${successText}" but the app reported a failure: ${failure}`);
    }

    await page.waitForTimeout(250);
  }

  throw new Error(`Timed out after ${timeoutMs}ms waiting for "${successText}".`);
}

/** Call a Tauri command directly, for asserting on backend state the UI only summarizes. */
export async function invokeCommand<T>(
  page: Page,
  command: string,
  args: Record<string, unknown> = {},
): Promise<T> {
  return page.evaluate<T>(
    `(async () => await window.__TAURI_INTERNALS__.invoke(${JSON.stringify(command)}, ${JSON.stringify(args)}))()`,
  );
}

/**
 * Force-clean every existing node so a run starts from a known state.
 *
 * Left-over nodes from earlier sessions reference containers that no longer exist, and startup
 * reconcile surfaces them as errors that have nothing to do with the test.
 */
export async function resetNodes(page: Page): Promise<void> {
  // Land on a dashboard route first: the app redirects an unconfigured install to onboarding, and
  // commands issued from a transient page race the navigation.
  await page.goto(`${APP_URL}/dashboard`);
  await page.waitForLoadState("domcontentloaded");

  const diagnostics = await readDiagnostics(page);

  for (const node of diagnostics.nodes ?? []) {
    await invokeCommand(page, "force_cleanup_vpn_node", { nodeId: node.id }).catch(() => undefined);
  }

  await page.reload();
  await page.waitForLoadState("domcontentloaded");
  await settleToasts(page);
}

export type DiagnosticsBundle = {
  nodes?: Array<{
    id: string;
    lifecycle_status: string;
    public_ipv4: string | null;
    server_public_key: string | null;
    endpoint_port: number | null;
  }>;
  tunnels?: Array<{
    id: string;
    node_id: string;
    status: string;
    last_handshake_age_seconds: number | null;
    rx_bytes: number | null;
    tx_bytes: number | null;
  }>;
  init_runs?: Array<{
    node_id: string;
    provider_slug: string;
    stage: string;
    outcome: string;
  }>;
};

export async function readDiagnostics(page: Page): Promise<DiagnosticsBundle> {
  return invokeCommand<DiagnosticsBundle>(page, "get_diagnostics_bundle");
}

/** Save the dev Docker credential. Provisioning is blocked until provider access is healthy. */
export async function saveDockerCredential(page: Page): Promise<void> {
  await page.goto(`${APP_URL}/dashboard/settings`);
  await expect(page.getByRole("heading", { name: "Provider API Keys" })).toBeVisible({
    timeout: 30_000,
  });

  await page.getByRole("button", { name: "Open Key Entry Modal" }).click();
  await expect(page.getByRole("heading", { name: "Provider Credential" })).toBeVisible({
    timeout: 10_000,
  });

  await page.locator("select").first().selectOption(DOCKER_PROVIDER_ID);
  await page.getByLabel("Docker Key").fill(DOCKER_TOKEN);

  await settleToasts(page);
  await page.getByRole("button", { name: "Save Token" }).click();
  await expectSuccess(page, "Docker faux token saved");

  await settleToasts(page);
  await page.getByRole("button", { name: "Validate Key" }).click();
  await expectSuccess(page, "Docker readiness checked");
}

/** Provision a node through the UI and wait for it to reach ready. */
export async function createDockerNode(
  page: Page,
  nodeId: string,
  nodeName: string,
): Promise<void> {
  await page.goto(`${APP_URL}/dashboard/nodes/create`);
  await expect(page.getByRole("heading", { name: "Create Node" }).first()).toBeVisible({
    timeout: 30_000,
  });

  await page.locator("select").first().selectOption(DOCKER_PROVIDER_ID);
  await page.getByLabel("Node ID").fill(nodeId);
  await page.getByLabel("Node Name").fill(nodeName);

  // Pricing loads asynchronously; a plan must be selected before create is allowed.
  const planCards = page.locator("button").filter({ hasText: "/mo" });
  await expect(planCards.first()).toBeVisible({ timeout: 60_000 });
  await planCards.first().click();

  await settleToasts(page);
  await page.getByRole("button", { name: "Create Node", exact: true }).click();
  // Provisioning pulls an image and runs apt inside the container on a cold cache.
  await expectSuccess(page, "Node created", 240_000);
}
