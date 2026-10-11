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
  activity?: Array<{
    node_id: string | null;
    provider_slug: string | null;
    category: string;
    stage: string;
    outcome: string;
    correlation_id: string | null;
    detail: string;
  }>;
};

export async function readDiagnostics(page: Page): Promise<DiagnosticsBundle> {
  return invokeCommand<DiagnosticsBundle>(page, "get_diagnostics_bundle");
}

export type AppPreferences = {
  close_to_tray: boolean;
  mobile_devices: string[];
  auto_connect_after_provision: boolean;
  first_connect_done: boolean;
  explain_uac: boolean;
  favourites: string[];
  game_high_score: number;
};

/** Merge a change into the stored preferences through the real backend, and return the result. */
export async function setPreferences(page: Page, patch: Partial<AppPreferences>): Promise<AppPreferences> {
  const current = await invokeCommand<AppPreferences>(page, "get_app_preferences");
  return invokeCommand<AppPreferences>(page, "set_app_preferences", { preferences: { ...current, ...patch } });
}

/** Save the dev Docker credential. Provisioning is blocked until provider access is healthy. */
export async function saveDockerCredential(page: Page): Promise<void> {
  await page.goto(`${APP_URL}/dashboard/settings`);
  await expect(page.getByRole("heading", { name: "Providers" })).toBeVisible({
    timeout: 30_000,
  });

  await page.getByRole("button", { name: "Add API key" }).click();

  // One step now: the modal saves the key and validates it against the provider, so there is no
  // separate Validate click and no window where a key is stored but unchecked.
  await page.locator("select").first().selectOption(DOCKER_PROVIDER_ID);
  await page.getByLabel("API key").fill(DOCKER_TOKEN);

  await settleToasts(page);
  await page.getByRole("button", { name: "Save & validate" }).click();
  await expectSuccess(page, "Key saved and verified");

  // The row must report the verified key — the status column previously could not express this.
  const dockerRow = page.getByRole("row").filter({ hasText: "Docker" });
  await expect(dockerRow.getByText("Verified", { exact: true })).toBeVisible({ timeout: 30_000 });
}

/**
 * Build a Docker node through the UI — Map, its region row, the build sheet — and wait for ready.
 *
 * Returns the node's id, which the app assigns. Auto-connect is off unless a test asks for it: a
 * connect needs elevation, and most specs run without it — an automatic connect there would stop
 * on a UAC prompt nobody can answer.
 */
export async function createDockerNode(
  page: Page,
  nodeName: string,
  { autoConnect = false }: { autoConnect?: boolean } = {},
): Promise<string> {
  await page.goto(`${APP_URL}/dashboard`);
  await setPreferences(page, { auto_connect_after_provision: autoConnect });
  const before = new Set(((await readDiagnostics(page)).nodes ?? []).map((node) => node.id));

  await page.goto(`${APP_URL}/dashboard/map`);
  // The dev provider has no geography, so it is listed under the map rather than pinned on it.
  const row = page.locator(`[data-region-key^="${DOCKER_PROVIDER_ID}:"]`).first();
  await expect(row).toBeVisible({ timeout: 60_000 });
  const regionKey = await row.getAttribute("data-region-key");
  const place = regionKey?.slice(DOCKER_PROVIDER_ID.length + 1) ?? "";
  await row.getByRole("button", { name: "Build here", exact: true }).click();

  const sheet = page.getByRole("dialog");
  // Pricing loads asynchronously; a size must be chosen before the build is allowed.
  await expect(sheet.getByRole("radio").first()).toBeVisible({ timeout: 60_000 });
  await sheet.getByText("Advanced", { exact: true }).click();
  await sheet.getByLabel("Server name").fill(nodeName);

  await settleToasts(page);
  await sheet.getByRole("button", { name: `Build ${place}`, exact: true }).click();

  // The build screen must name this server while it runs.
  const progress = page.getByTestId("operation-progress");
  await expect(progress).toHaveAttribute("data-kind", "provision", { timeout: 10_000 });
  await expect(progress).toContainText(nodeName);

  // Provisioning pulls an image and runs apt inside the container on a cold cache.
  await expectSuccess(page, autoConnect ? `${place} is ready — connecting now` : `${place} is ready`, 240_000);
  await expect(progress).toHaveCount(0);

  const created = ((await readDiagnostics(page)).nodes ?? []).filter((node) => !before.has(node.id));
  expect(created, "exactly one new node should exist after the build").toHaveLength(1);
  return created[0].id;
}
