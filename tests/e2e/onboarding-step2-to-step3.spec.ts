import { expect, test } from "@playwright/test";

test.describe("Onboarding Step 2", () => {
  test.beforeEach(async ({ page }) => {
    const apiToken = process.env.DO_API_TOKEN ?? "dop_v1_test_token";

    await page.addInitScript((tokenFromEnv) => {
      type TauriArgs = Record<string, unknown> | undefined;
      type Plan = {
        plan_key: string;
        plan_label: string;
        size_slug: string;
        region: string;
        memory_mb: number;
        vcpus: number;
        disk_gb: number;
        transfer_tb: number;
        transfer_note: string;
        monthly_usd: number;
        hourly_usd: number;
        estimated: boolean;
        source_timestamp: string;
      };

      const state = {
        tokenSaved: false,
        apiToken: tokenFromEnv,
        tokenRef: null as string | null,
        nodes: [] as Array<Record<string, unknown>>,
      };

      const commandCounts = {
        list_provider_regions: 0,
        list_provider_pricing: 0,
      };

      const successPricing: Plan[] = [
        {
          plan_key: "s-1vcpu-1gb",
          plan_label: "Basic",
          size_slug: "s-1vcpu-1gb",
          region: "nyc1",
          memory_mb: 1024,
          vcpus: 1,
          disk_gb: 25,
          transfer_tb: 1,
          transfer_note: "Includes up to 1.00 TB transfer before provider overage policies apply.",
          monthly_usd: 6,
          hourly_usd: 0.00893,
          estimated: false,
          source_timestamp: "2026-08-01T00:00:00Z",
        },
        {
          plan_key: "s-2vcpu-2gb",
          plan_label: "Basic",
          size_slug: "s-2vcpu-2gb",
          region: "nyc1",
          memory_mb: 2048,
          vcpus: 2,
          disk_gb: 60,
          transfer_tb: 3,
          transfer_note: "Includes up to 3.00 TB transfer before provider overage policies apply.",
          monthly_usd: 12,
          hourly_usd: 0.01786,
          estimated: false,
          source_timestamp: "2026-08-01T00:00:00Z",
        },
        {
          plan_key: "s-4vcpu-8gb",
          plan_label: "General Purpose",
          size_slug: "s-4vcpu-8gb",
          region: "nyc1",
          memory_mb: 8192,
          vcpus: 4,
          disk_gb: 160,
          transfer_tb: 5,
          transfer_note: "Includes up to 5.00 TB transfer before provider overage policies apply.",
          monthly_usd: 24,
          hourly_usd: 0.03571,
          estimated: false,
          source_timestamp: "2026-08-01T00:00:00Z",
        },
      ];

      const getSeededTokenRef = () => (window as unknown as { __TEST_PROVIDER_TOKEN_REF?: string | null }).__TEST_PROVIDER_TOKEN_REF ?? null;

      const internals = {
        invoke: async (command: string, args?: TauriArgs) => {
          if (command === "check_wireguard_readiness") {
            return {
              installed: true,
              executable_found: true,
              service_available: true,
              version: "0.5.3",
              message: "WireGuard is ready.",
              action_guidance: "Continue onboarding.",
            };
          }

          if (command === "create_vpn_node_to_ready") {
            const nodeId = String(args?.nodeId ?? "node-1");
            const region = String(args?.region ?? "nyc1");
            const planKey = String(args?.planKey ?? "s-1vcpu-1gb");
            const node = {
              id: nodeId,
              provider_id: "provider-1",
              name: String(args?.nodeName ?? "Primary Node"),
              region,
              plan_key: planKey,
              size_slug: planKey,
              price_monthly_usd: 6,
              price_hourly_usd: 0.00893,
              price_source_timestamp: "2026-08-01T00:00:00Z",
              lifecycle_status: "ready",
              external_ref: "42",
              public_ipv4: "203.0.113.10",
              server_public_key: "server-key-1",
              error_reason: null,
              created_at: "2026-08-01T00:00:00Z",
              updated_at: "2026-08-01T00:00:00Z",
            };
            state.nodes = [node];
            return "Node created and ready.";
          }

          if (command === "register_digitalocean_provider_token") {
            const candidate = String(args?.apiToken ?? "");
            if (!candidate || !candidate.startsWith("dop_v1_")) {
              throw new Error("DigitalOcean token was rejected. Verify token format and scope.");
            }
            state.tokenSaved = true;
            state.apiToken = candidate;
            state.tokenRef = "do-token:provider-1:test-token-ref";
            return "Provider registered with secure token reference.";
          }

          if (command === "list_providers") {
            const seededTokenRef = getSeededTokenRef();
            return [
              {
                id: "provider-1",
                name: "DigitalOcean",
                slug: "digitalocean",
                token_ref: seededTokenRef ?? state.tokenRef,
                created_at: "2026-08-01T00:00:00Z",
                updated_at: "2026-08-01T00:00:00Z",
              },
            ];
          }

          if (command === "validate_provider_token") {
            const seededTokenRef = getSeededTokenRef();
            if ((!state.tokenSaved && !state.tokenRef && !seededTokenRef) || !state.apiToken) {
              throw new Error("Provider 'provider-1' has no token_ref. Provider must be initialized with a token before adapter operations.");
            }
            return "Provider token is valid for DigitalOcean API.";
          }

          if (command === "list_provider_pricing") {
            commandCounts.list_provider_pricing += 1;
            const forcePricingStale = Boolean((window as unknown as { __TEST_FORCE_STALE?: boolean }).__TEST_FORCE_STALE);
            if (forcePricingStale) {
              state.tokenSaved = false;
              state.tokenRef = null;
              throw new Error("Provider token_ref 'do-token:provider-1:stale' has no stored secret. Stored token reference is stale: secure storage entry is missing.");
            }

            if (!state.tokenSaved) {
              throw new Error("Provider 'provider-1' has no token_ref. Provider must be initialized with a token before adapter operations.");
            }

            const region = String(args?.region ?? "nyc1");
            return {
              provider_id: "provider-1",
              region,
              plans: successPricing.map((plan) => ({ ...plan, region })),
            };
          }

          if (command === "list_provider_regions") {
            commandCounts.list_provider_regions += 1;
            return ["nyc1", "sfo3", "fra1"];
          }

          if (command === "list_vpn_nodes") {
            return state.nodes.length > 0
              ? state.nodes
              : [
              {
                id: "node-1",
                provider_id: "provider-1",
                name: "Primary Node",
                region: "nyc1",
                plan_key: "s-1vcpu-1gb",
                size_slug: "s-1vcpu-1gb",
                price_monthly_usd: 6,
                price_hourly_usd: 0.00893,
                price_source_timestamp: "2026-08-01T00:00:00Z",
                lifecycle_status: "ready",
                external_ref: "42",
                public_ipv4: "203.0.113.10",
                server_public_key: "server-key-1",
                error_reason: null,
                created_at: "2026-08-01T00:00:00Z",
                updated_at: "2026-08-01T00:00:00Z",
              },
              ];
          }

          if (command === "get_diagnostics_bundle") {
            return { generated_at: "2026-08-01T00:00:00Z", providers: [], nodes: [], tunnels: [], redacted_notes: [] };
          }

          if (command === "list_providers") {
            return [{ id: "provider-1", name: "DigitalOcean", slug: "digitalocean", token_ref: "do-token:provider-1:test-token-ref", created_at: "2026-08-01T00:00:00Z", updated_at: "2026-08-01T00:00:00Z" }];
          }

          if (command === "validate_provider_token") {
            return "Provider token is valid for DigitalOcean API.";
          }

          if (command === "list_tunnels_for_node") {
            return [];
          }

          if (command === "get_health_panel_data") {
            return { node_id: "node-1", tunnel_id: null, endpoint_ip: "203.0.113.10", handshake_age_seconds: null, rx_bytes: null, tx_bytes: null, external_ipv4: "203.0.113.10", tunnel_status: "ready" };
          }

          (window as unknown as { __TEST_COMMAND_COUNTS__?: typeof commandCounts }).__TEST_COMMAND_COUNTS__ = commandCounts;

          throw new Error(`Unhandled mocked command: ${command}`);
        },
      };

      (window as unknown as { __TEST_COMMAND_COUNTS__?: typeof commandCounts }).__TEST_COMMAND_COUNTS__ = commandCounts;

      (window as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ = internals;
    }, apiToken);

    await page.goto("/onboarding");

    await expect(page.getByRole("heading", { name: "WireGuard Readiness" })).toBeVisible();
    await page.getByRole("button", { name: "Next", exact: true }).click();
    await expect(page.getByRole("heading", { name: "Provider Token" })).toBeVisible();
  });

  test("requires save before validation", async ({ page }) => {
    const validateButton = page.getByRole("button", { name: "Validate Token", exact: true });
    await expect(validateButton).toBeDisabled();
  });

  test("does not allow advancing to step 3 before token registration", async ({ page }) => {
    await expect(page.getByRole("button", { name: "Next", exact: true })).toBeDisabled();
  });

  test("save and validate token in one action advances to step 3 with live pricing", async ({ page }) => {
    const tokenInput = page.getByPlaceholder("dop_v1_...");
    await tokenInput.fill(process.env.DO_API_TOKEN ?? "dop_v1_test_token");

    await page.getByRole("button", { name: "Save + Validate Token", exact: true }).click();
    await expect(page.getByText("Provider registered with secure token reference.")).toBeVisible();
    await expect(page.getByText("Provider token is valid for DigitalOcean API.")).toBeVisible();

    await page.getByRole("button", { name: "Next", exact: true }).click();

    await expect(page.getByRole("heading", { name: "Server Configuration" })).toBeVisible();
    await expect(page.getByText("Live pricing")).toBeVisible();
    await expect(page.getByText("$6.00/mo")).toBeVisible();

    const pricingCounts = await page.evaluate(() => (window as unknown as { __TEST_COMMAND_COUNTS__?: { list_provider_regions: number; list_provider_pricing: number } }).__TEST_COMMAND_COUNTS__ ?? null);
    expect(pricingCounts).toEqual({ list_provider_regions: 1, list_provider_pricing: 3 });

    await page.getByRole("button", { name: "Next", exact: true }).click();
    await page.getByRole("button", { name: "Start Provisioning", exact: true }).click();
    await expect(page.getByRole("button", { name: "Next", exact: true })).toBeEnabled();
    await page.getByRole("button", { name: "Next", exact: true }).click();
    await expect(page.getByRole("button", { name: "Go to Dashboard", exact: true })).toBeVisible();
    await page.getByRole("button", { name: "Go to Dashboard", exact: true }).click();
    await expect(page.getByRole("heading", { name: "Network Stats" })).toBeVisible();

    await page.getByRole("link", { name: "Server Locations" }).click();
    await expect(page.getByRole("heading", { name: "Node Management" }).first()).toBeVisible();

    const postNavCounts = await page.evaluate(() => (window as unknown as { __TEST_COMMAND_COUNTS__?: { list_provider_regions: number; list_provider_pricing: number } }).__TEST_COMMAND_COUNTS__ ?? null);
    expect(postNavCounts).toEqual({ list_provider_regions: 1, list_provider_pricing: 3 });
  });

  test("returns to step 2 with guidance when token becomes stale during step 3 pricing load", async ({ page }) => {
    const tokenInput = page.getByPlaceholder("dop_v1_...");
    await tokenInput.fill(process.env.DO_API_TOKEN ?? "dop_v1_test_token");

    await page.getByRole("button", { name: "Save + Validate Token", exact: true }).click();
    await expect(page.getByText("Provider token is valid for DigitalOcean API.")).toBeVisible();

    await page.evaluate(() => {
      (window as unknown as { __TEST_FORCE_STALE?: boolean }).__TEST_FORCE_STALE = true;
    });

    await page.getByRole("button", { name: "Next", exact: true }).click();

    await expect(page.getByRole("heading", { name: "Provider Token" })).toBeVisible();
    await expect(page.getByText("Provider token is no longer valid in secure storage. Save Token, then Validate Token to continue.")).toBeVisible();
  });

  test("recognizes a token saved from settings and unlocks step 3 without re-entry", async ({ page }) => {
    await page.addInitScript(() => {
      (window as unknown as { __TEST_PROVIDER_TOKEN_REF?: string | null }).__TEST_PROVIDER_TOKEN_REF = "do-token:provider-1:seeded-from-settings";
    });

    await page.goto("/onboarding");
    await expect(page.getByText("Provider token is valid for DigitalOcean API.")).toBeVisible();
    await expect(page.getByRole("button", { name: "Next", exact: true })).toBeEnabled();
  });
});
