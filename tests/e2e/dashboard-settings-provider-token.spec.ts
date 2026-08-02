import { expect, test } from "@playwright/test";

test.describe("Dashboard settings provider token", () => {
  test("opens provider modal from card, saves token, and shows updated connected provider details", async ({ page }) => {
    const apiToken = process.env.DO_API_TOKEN ?? "dop_v1_test_token_1234567890abcdef";

    await page.addInitScript((tokenFromEnv) => {
      type TauriArgs = Record<string, unknown> | undefined;
      type Provider = {
        id: string;
        name: string;
        slug: string;
        status: "active" | "suspended";
        token_ref: string | null;
        created_at: string;
        updated_at: string;
      };

      const providerState: Provider = {
        id: "provider-1",
        name: "DigitalOcean",
        slug: "digitalocean",
        status: "active",
        token_ref: null,
        created_at: "2026-08-01T00:00:00Z",
        updated_at: "2026-08-01T00:00:00Z",
      };

      const internals = {
        invoke: async (command: string, args?: TauriArgs) => {
          if (command === "list_vpn_nodes") {
            return [];
          }

          if (command === "get_diagnostics_bundle") {
            return {
              generated_at: "2026-08-01T00:00:00Z",
              providers: [
                {
                  ...providerState,
                  token_ref: null,
                },
              ],
              nodes: [],
              tunnels: [],
              redacted_notes: ["token_ref removed from diagnostics bundle"],
            };
          }

          if (command === "list_providers") {
            return [{ ...providerState }];
          }

          if (command === "register_digitalocean_provider_token") {
            const candidate = String(args?.apiToken ?? "");
            if (!candidate || !candidate.startsWith("dop_v1_")) {
              throw new Error("DigitalOcean token was rejected. Verify token format and scope.");
            }

            providerState.token_ref = "do-token:provider-1:REDACTED_LEAKED_DO_TOKEN";
            providerState.updated_at = "2026-08-01T00:05:00Z";

            return `Provider registered with token_ref: ${providerState.token_ref}`;
          }

          if (command === "validate_provider_token") {
            if (!providerState.token_ref) {
              throw new Error("Provider 'provider-1' has no token_ref. Provider must be initialized with a token before adapter operations.");
            }

            return "Provider token valid for account acct-1 (active)";
          }

          throw new Error(`Unhandled mocked command: ${command}`);
        },
      };

      (window as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ = internals;
      (window as unknown as { __TEST_API_TOKEN__?: string }).__TEST_API_TOKEN__ = tokenFromEnv;
    }, apiToken);

    await page.goto("/dashboard/settings");

    await expect(page.getByRole("heading", { name: "Provider API Keys" })).toBeVisible();
    await expect(page.getByText("Installed API keys by provider", { exact: true })).toBeVisible();
    await expect(page.getByRole("row", { name: /DigitalOcean/i }).getByText("Missing", { exact: true })).toBeVisible();

    await page.getByRole("button", { name: "Open Key Entry Modal" }).click();
    await expect(page.getByRole("heading", { name: "Provider Credential" })).toBeVisible();
    await page.getByPlaceholder("dop_v1_...").fill(apiToken);
    await page.getByRole("button", { name: "Save Token", exact: true }).click();

    await expect(page.getByRole("row", { name: /DigitalOcean/i })).toContainText("Installed");
    await expect(page.getByRole("row", { name: /DigitalOcean/i }).getByText("do-token...f6bf", { exact: true })).toBeVisible();
    await expect(page.getByText(/Provider registered with token_ref:/).first()).toBeVisible();
    await expect(page.getByPlaceholder("dop_v1_...")).toHaveValue("");
  });

  test("updates the provider list when suspending or deleting a provider", async ({ page }) => {
    await page.addInitScript(() => {
      type TauriArgs = Record<string, unknown> | undefined;
      type Provider = {
        id: string;
        name: string;
        slug: string;
        status: "active" | "suspended";
        token_ref: string | null;
        created_at: string;
        updated_at: string;
      };

      const providers: Provider[] = [
        {
          id: "provider-1",
          name: "DigitalOcean",
          slug: "digitalocean",
          status: "active",
          token_ref: "do-token:provider-1:REDACTED_LEAKED_DO_TOKEN",
          created_at: "2026-08-01T00:00:00Z",
          updated_at: "2026-08-01T00:00:00Z",
        },
        {
          id: "provider-2",
          name: "Docker",
          slug: "docker",
          status: "active",
          token_ref: "dev-token:provider-2:local",
          created_at: "2026-08-01T00:01:00Z",
          updated_at: "2026-08-01T00:01:00Z",
        },
      ];

      const internals = {
        invoke: async (command: string, args?: TauriArgs) => {
          if (command === "list_vpn_nodes") {
            return [];
          }

          if (command === "get_diagnostics_bundle") {
            return {
              generated_at: "2026-08-01T00:00:00Z",
              providers: providers.map((provider) => ({ ...provider })),
              nodes: [],
              tunnels: [],
              redacted_notes: [],
            };
          }

          if (command === "list_providers") {
            return providers.map((provider) => ({ ...provider }));
          }

          if (command === "validate_provider_token") {
            return "Provider token valid for account acct-1 (active)";
          }

          if (command === "suspend_provider") {
            const provider = providers.find((item) => item.id === String(args?.providerId ?? ""));
            if (!provider) {
              throw new Error("Provider not found.");
            }

            provider.status = "suspended";
            provider.updated_at = "2026-08-01T00:10:00Z";
            return;
          }

          if (command === "delete_provider") {
            const index = providers.findIndex((item) => item.id === String(args?.providerId ?? ""));
            if (index === -1) {
              throw new Error("Provider not found.");
            }

            providers.splice(index, 1);
            return;
          }

          throw new Error(`Unhandled mocked command: ${command}`);
        },
      };

      (window as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ = internals;
    });

    await page.goto("/dashboard/settings");

    const digitalOceanRow = page.getByRole("row", { name: /DigitalOcean/i });
    await expect(digitalOceanRow.getByText("Connected", { exact: true })).toBeVisible();

    page.once("dialog", async (dialog) => {
      await dialog.accept();
    });
    await digitalOceanRow.getByRole("button", { name: "Suspend" }).click();

    await expect(digitalOceanRow).toContainText("Suspended");
    await expect(digitalOceanRow.getByRole("button", { name: "Suspended" })).toBeVisible();

    page.once("dialog", async (dialog) => {
      await dialog.accept();
    });
    await digitalOceanRow.getByRole("button", { name: "Delete" }).click();

    await expect(page.getByRole("row", { name: /DigitalOcean/i })).toHaveCount(0);
    await expect(page.getByRole("row", { name: /Docker/i })).toBeVisible();
  });
});