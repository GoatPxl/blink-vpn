import { expect, test } from "@playwright/test";

test.describe("Dashboard Nodes", () => {
  test("keeps existing nodes visible while disabling provisioning when provider access is invalid", async ({ page }) => {
    await page.addInitScript(() => {
      const nodes = [
        {
          id: "node-1",
          provider_id: "provider-1",
          name: "Primary Node",
          region: "nyc1",
          plan_key: "starter",
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

      const internals = {
        invoke: async (command: string) => {
          if (command === "list_vpn_nodes") return nodes;
          if (command === "get_diagnostics_bundle") {
            return { generated_at: "2026-08-01T00:00:00Z", providers: [], nodes, tunnels: [], redacted_notes: [] };
          }
          if (command === "list_providers") {
            return [{ id: "provider-1", name: "DigitalOcean", slug: "digitalocean", token_ref: "do-token:provider-1:seeded", created_at: "2026-08-01T00:00:00Z", updated_at: "2026-08-01T00:00:00Z" }];
          }
          if (command === "validate_provider_token") {
            throw new Error("DigitalOcean token was rejected. Verify token scope and save a new token in Settings.");
          }
          if (command === "list_tunnels_for_node") return [];
          if (command === "get_health_panel_data") {
            return { node_id: "node-1", tunnel_id: null, endpoint_ip: "203.0.113.10", handshake_age_seconds: null, rx_bytes: null, tx_bytes: null, external_ipv4: "203.0.113.10", tunnel_status: "ready" };
          }
          throw new Error(`Unhandled mocked command: ${command}`);
        },
      };

      (window as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ = internals;
    });

    await page.goto("/dashboard/nodes");

    await expect(page.getByRole("heading", { name: "Node Management" }).first()).toBeVisible();
    await expect(page.getByText("Primary Node", { exact: true })).toBeVisible();
    await expect(page.getByText("Provisioning is disabled until provider access is healthy.")).toBeVisible();
    await expect(page.getByRole("button", { name: "Create Node", exact: true })).toBeDisabled();
    await expect(page.getByRole("button", { name: "Open Settings", exact: true })).toBeVisible();
  });

  test("shows pricing preview, filters, and node details when provider access is ready", async ({ page }) => {
    await page.addInitScript(() => {
      const nodes = [
        {
          id: "node-1",
          provider_id: "provider-1",
          name: "Primary Node",
          region: "nyc1",
          plan_key: "starter",
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
        {
          id: "node-2",
          provider_id: "provider-1",
          name: "Recovery Node",
          region: "sfo3",
          plan_key: "balanced",
          size_slug: "s-2vcpu-2gb",
          price_monthly_usd: 12,
          price_hourly_usd: 0.01786,
          price_source_timestamp: "2026-08-02T00:00:00Z",
          lifecycle_status: "error",
          external_ref: "84",
          public_ipv4: "203.0.113.11",
          server_public_key: "server-key-2",
          error_reason: "Bootstrap marker missing.",
          created_at: "2026-08-02T00:00:00Z",
          updated_at: "2026-08-02T00:00:00Z",
        },
      ];

      const pricingByRegion = {
        nyc1: [
          {
            plan_key: "starter",
            plan_label: "Starter",
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
            plan_key: "balanced",
            plan_label: "Balanced",
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
        ],
        sfo3: [
          {
            plan_key: "balanced",
            plan_label: "Balanced",
            size_slug: "s-2vcpu-2gb",
            region: "sfo3",
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
        ],
      } as Record<string, unknown>;

      const tunnels = [{ id: "tunnel-1", node_id: "node-1", name: "wg-primary", status: "connected", interface_name: "wg-primary", assigned_ipv4: "10.44.0.2", last_handshake_age_seconds: 12, rx_bytes: 1024, tx_bytes: 2048, stats_updated_at: "2026-08-01T00:00:00Z", last_error: null, created_at: "2026-08-01T00:00:00Z", updated_at: "2026-08-01T00:00:00Z" }];

      const internals = {
        invoke: async (command: string, args?: Record<string, unknown>) => {
          if (command === "list_vpn_nodes") return nodes;
          if (command === "get_diagnostics_bundle") {
            return { generated_at: "2026-08-01T00:00:00Z", providers: [], nodes, tunnels, redacted_notes: [] };
          }
          if (command === "list_providers") {
            return [{ id: "provider-1", name: "DigitalOcean", slug: "digitalocean", token_ref: "do-token:provider-1:seeded", created_at: "2026-08-01T00:00:00Z", updated_at: "2026-08-01T00:00:00Z" }];
          }
          if (command === "validate_provider_token") return "Provider token valid for account acct-1 (active)";
          if (command === "list_provider_pricing") return { provider_id: "provider-1", region: String(args?.region ?? "nyc1"), plans: pricingByRegion[String(args?.region ?? "nyc1")] ?? [] };
          if (command === "list_tunnels_for_node") return tunnels.filter((tunnel) => tunnel.node_id === args?.nodeId);
          if (command === "get_health_panel_data") {
            return { node_id: String(args?.nodeId ?? "node-1"), tunnel_id: "tunnel-1", endpoint_ip: "203.0.113.10", handshake_age_seconds: 12, rx_bytes: 1024, tx_bytes: 2048, external_ipv4: "203.0.113.10", tunnel_status: "connected" };
          }
          throw new Error(`Unhandled mocked command: ${command}`);
        },
      };

      (window as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ = internals;
    });

    await page.goto("/dashboard/nodes");

    await expect(page.getByText("Provider access: Ready")).toBeVisible();
    await expect(page.getByRole("cell", { name: "1 vCPU · 1 GB · 25 GB" })).toBeVisible();
    await expect(page.getByText("Includes up to 1.00 TB transfer before provider overage policies apply.").first()).toBeVisible();

    await page.getByLabel("Status").selectOption("error");
    await expect(page.getByText("Recovery Node", { exact: true })).toBeVisible();
    await expect(page.getByText("Primary Node", { exact: true })).toBeHidden();

    await page.getByRole("button", { name: "View Details", exact: true }).click();
    await expect(page.getByText("Bootstrap marker missing. Next: retry validation, recreate the node, or use force cleanup if partial resources remain.")).toBeVisible();
    await expect(page.getByRole("button", { name: "Force Cleanup", exact: true })).toBeVisible();
  });
});