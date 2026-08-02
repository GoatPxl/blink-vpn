import { expect, test } from "@playwright/test";

test.describe("Dashboard settings provider token", () => {
  test("saves a token and shows the stored token ref on the current provider card", async ({ page }) => {
    const apiToken = process.env.DO_API_TOKEN ?? "dop_v1_test_token_1234567890abcdef";

    await page.addInitScript((tokenFromEnv) => {
      type TauriArgs = Record<string, unknown> | undefined;
      type Provider = {
        id: string;
        name: string;
        slug: string;
        token_ref: string | null;
        created_at: string;
        updated_at: string;
      };

      const providerState: Provider = {
        id: "provider-1",
        name: "DigitalOcean",
        slug: "digitalocean",
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

    await expect(page.getByRole("heading", { name: "Current Provider" })).toBeVisible();
    await expect(page.getByText("Missing", { exact: true })).toBeVisible();
    await expect(page.getByText("Not configured", { exact: true })).toBeVisible();

    await page.getByPlaceholder("dop_v1_...").fill(apiToken);
    await page.getByRole("button", { name: "Save Token", exact: true }).click();

    await expect(page.getByText("Configured", { exact: true })).toBeVisible();
    await expect(page.getByText("do-token...f6bf", { exact: true })).toBeVisible();
    await expect(page.getByText(/Provider registered with token_ref:/).first()).toBeVisible();
    await expect(page.getByPlaceholder("dop_v1_...")).toHaveValue("");
  });
});