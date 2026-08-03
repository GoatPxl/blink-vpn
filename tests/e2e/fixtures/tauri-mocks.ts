import type { Page } from "@playwright/test";

type Provider = {
  id: string;
  name: string;
  slug: string;
  status: "active" | "suspended";
  token_ref: string | null;
  created_at: string;
  updated_at: string;
};

export async function installSingleProviderTokenMock(
  page: Page,
  apiToken: string,
): Promise<void> {
  await page.addInitScript((tokenFromEnv) => {
    type TauriArgs = Record<string, unknown> | undefined;
    type ProviderRecord = {
      id: string;
      name: string;
      slug: string;
      status: "active" | "suspended";
      token_ref: string | null;
      created_at: string;
      updated_at: string;
    };

    const providerState: ProviderRecord = {
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
}

export async function installProviderStateMutationMock(page: Page): Promise<void> {
  await page.addInitScript(() => {
    type TauriArgs = Record<string, unknown> | undefined;
    type ProviderRecord = {
      id: string;
      name: string;
      slug: string;
      status: "active" | "suspended";
      token_ref: string | null;
      created_at: string;
      updated_at: string;
    };

    const providers: ProviderRecord[] = [
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

        if (command === "resume_provider") {
          const provider = providers.find((item) => item.id === String(args?.providerId ?? ""));
          if (!provider) {
            throw new Error("Provider not found.");
          }

          provider.status = "active";
          provider.updated_at = "2026-08-01T00:11:00Z";
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
}

export async function installProviderLifecycleMock(page: Page): Promise<void> {
  await page.addInitScript(() => {
    type TauriArgs = Record<string, unknown> | undefined;
    type ProviderRecord = {
      id: string;
      name: string;
      slug: string;
      status: "active" | "suspended";
      token_ref: string | null;
      created_at: string;
      updated_at: string;
    };
    type VpnNodeRecord = {
      id: string;
      provider_id: string;
      name: string;
      region: string;
      plan_key: string | null;
      size_slug: string | null;
      price_monthly_usd: number | null;
      price_hourly_usd: number | null;
      price_source_timestamp: string | null;
      lifecycle_status: string;
      external_ref: string | null;
      public_ipv4: string | null;
      server_public_key: string | null;
      error_reason: string | null;
      created_at: string;
      updated_at: string;
    };

    const providerLifecycleStorageKey = "goatstare.e2e.providerLifecycleState";

    function loadProviders(): ProviderRecord[] {
      const persisted = window.localStorage.getItem(providerLifecycleStorageKey);
      if (!persisted) {
        return [
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
      }

      try {
        const parsed = JSON.parse(persisted) as ProviderRecord[];
        if (Array.isArray(parsed) && parsed.length > 0) {
          return parsed;
        }
      } catch {
        // Fall through to the seeded defaults.
      }

      return [
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
    }

    function persistProviders(providers: ProviderRecord[]): void {
      window.localStorage.setItem(providerLifecycleStorageKey, JSON.stringify(providers));
    }

    const providers = loadProviders();

    const nodes: VpnNodeRecord[] = [
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

    const tunnels = [
      {
        id: "tunnel-1",
        node_id: "node-1",
        name: "wg-primary",
        status: "connected",
        interface_name: "wg-primary",
        assigned_ipv4: "10.44.0.2",
        last_handshake_age_seconds: 12,
        rx_bytes: 1024,
        tx_bytes: 2048,
        stats_updated_at: "2026-08-01T00:00:00Z",
        last_error: null,
        created_at: "2026-08-01T00:00:00Z",
        updated_at: "2026-08-01T00:00:00Z",
      },
    ];

    const pricingByRegion = {
      nyc1: [
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
      ],
    } as Record<string, unknown>;

    const internals = {
      invoke: async (command: string, args?: TauriArgs) => {
        if (command === "list_vpn_nodes") {
          return nodes;
        }

        if (command === "get_diagnostics_bundle") {
          return {
            generated_at: "2026-08-01T00:00:00Z",
            providers: providers.map((provider) => ({ ...provider })),
            nodes,
            tunnels,
            redacted_notes: [],
          };
        }

        if (command === "list_providers") {
          return providers.map((provider) => ({ ...provider }));
        }

        if (command === "list_provider_regions") {
          return ["nyc1"];
        }

        if (command === "list_provider_pricing") {
          const region = String(args?.region ?? "nyc1");
          return {
            provider_id: String(args?.providerId ?? "provider-1"),
            region,
            plans: (pricingByRegion[region] as Array<Record<string, unknown>> | undefined) ?? [],
          };
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
          persistProviders(providers);
          return;
        }

        if (command === "resume_provider") {
          const provider = providers.find((item) => item.id === String(args?.providerId ?? ""));
          if (!provider) {
            throw new Error("Provider not found.");
          }

          provider.status = "active";
          provider.updated_at = "2026-08-01T00:11:00Z";
          persistProviders(providers);
          return;
        }

        if (command === "delete_provider") {
          const index = providers.findIndex((item) => item.id === String(args?.providerId ?? ""));
          if (index === -1) {
            throw new Error("Provider not found.");
          }

          providers.splice(index, 1);
          persistProviders(providers);
          return;
        }

        if (command === "list_tunnels_for_node") {
          return tunnels.filter((tunnel) => tunnel.node_id === String(args?.nodeId ?? ""));
        }

        if (command === "get_health_panel_data") {
          return {
            node_id: String(args?.nodeId ?? "node-1"),
            tunnel_id: "tunnel-1",
            endpoint_ip: "203.0.113.10",
            handshake_age_seconds: 12,
            rx_bytes: 1024,
            tx_bytes: 2048,
            external_ipv4: "203.0.113.10",
            tunnel_status: "connected",
          };
        }

        throw new Error(`Unhandled mocked command: ${command}`);
      },
    };

    (window as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ = internals;
  });
}

export async function installSettingsLifecycleMock(
  page: Page,
  options?: {
    attachNodeToDigitalOcean?: boolean;
  },
): Promise<void> {
  await page.addInitScript((initOptions) => {
    type TauriArgs = Record<string, unknown> | undefined;
    type ProviderRecord = {
      id: string;
      name: string;
      slug: string;
      status: "active" | "suspended";
      token_ref: string | null;
      created_at: string;
      updated_at: string;
    };
    type VpnNodeRecord = {
      id: string;
      provider_id: string;
      name: string;
      region: string;
      plan_key: string | null;
      size_slug: string | null;
      price_monthly_usd: number | null;
      price_hourly_usd: number | null;
      price_source_timestamp: string | null;
      lifecycle_status: string;
      external_ref: string | null;
      public_ipv4: string | null;
      server_public_key: string | null;
      error_reason: string | null;
      created_at: string;
      updated_at: string;
    };

    const providers: ProviderRecord[] = [
      {
        id: "provider-1",
        name: "DigitalOcean",
        slug: "digitalocean",
        status: "active",
        token_ref: "do-token:provider-1:seeded",
        created_at: "2026-08-01T00:00:00Z",
        updated_at: "2026-08-01T00:00:00Z",
      },
      {
        id: "provider-docker",
        name: "Docker",
        slug: "docker",
        status: "active",
        token_ref: "do-token:provider-docker:seeded",
        created_at: "2026-08-01T00:01:00Z",
        updated_at: "2026-08-01T00:01:00Z",
      },
    ];

    const nodes: VpnNodeRecord[] = initOptions?.attachNodeToDigitalOcean
      ? [
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
        ]
      : [];

    const internals = {
      invoke: async (command: string, args?: TauriArgs) => {
        if (command === "list_vpn_nodes") {
          return nodes.map((node) => ({ ...node }));
        }

        if (command === "get_diagnostics_bundle") {
          return {
            generated_at: "2026-08-01T00:00:00Z",
            providers: providers.map((provider) => ({ ...provider, token_ref: null })),
            nodes: nodes.map((node) => ({ ...node })),
            tunnels: [],
            redacted_notes: ["token_ref removed from diagnostics bundle"],
          };
        }

        if (command === "list_providers") {
          return providers.map((provider) => ({ ...provider }));
        }

        if (command === "validate_provider_token") {
          const provider = providers.find((item) => item.id === String(args?.providerId ?? ""));
          if (!provider || !provider.token_ref) {
            throw new Error(`Provider '${String(args?.providerId ?? "")}' has no token_ref. Provider must be initialized with a token before adapter operations.`);
          }

          return `Provider token valid for account ${provider.id} (active)`;
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

        if (command === "resume_provider") {
          const provider = providers.find((item) => item.id === String(args?.providerId ?? ""));
          if (!provider) {
            throw new Error("Provider not found.");
          }

          provider.status = "active";
          provider.updated_at = "2026-08-01T00:11:00Z";
          return;
        }

        if (command === "delete_provider") {
          const provider = providers.find((item) => item.id === String(args?.providerId ?? ""));
          if (!provider) {
            throw new Error("Provider not found.");
          }

          const attachedNodes = nodes.filter((node) => node.provider_id === provider.id).length;
          if (attachedNodes > 0) {
            throw new Error(`Cannot delete provider key while ${attachedNodes} node${attachedNodes === 1 ? "" : "s"} are still attached. Destroy dependent nodes first.`);
          }

          provider.token_ref = null;
          provider.status = "active";
          provider.updated_at = "2026-08-01T00:12:00Z";
          return;
        }

        if (command === "list_tunnels_for_node") {
          return [];
        }

        if (command === "get_health_panel_data") {
          return {
            node_id: String(args?.nodeId ?? ""),
            tunnel_id: null,
            endpoint_ip: null,
            handshake_age_seconds: null,
            rx_bytes: null,
            tx_bytes: null,
            external_ipv4: null,
            tunnel_status: null,
          };
        }

        throw new Error(`Unhandled mocked command: ${command}`);
      },
    };

    (window as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ = internals;
  }, options);
}

export function buildProvider(
  overrides: Partial<Provider> & Pick<Provider, "id" | "name" | "slug">,
): Provider {
  return {
    status: "active",
    token_ref: null,
    created_at: "2026-08-01T00:00:00Z",
    updated_at: "2026-08-01T00:00:00Z",
    ...overrides,
  };
}

export async function installDockerSettingsMock(
  page: Page,
  options?: {
    dockerReadiness?: {
      installed: boolean;
      daemon_reachable: boolean;
      cli_version: string | null;
      daemon_version: string | null;
      message: string;
      action_guidance: string;
    };
  },
): Promise<void> {
  const dockerReadiness = options?.dockerReadiness ?? {
    installed: true,
    daemon_reachable: true,
    cli_version: "Docker version 27.0.0, build test",
    daemon_version: "27.0.0",
    message: "Docker CLI and daemon are available for development emulation.",
    action_guidance: "You can use Docker development mode workflows from Settings and Node planning surfaces.",
  };

  await page.addInitScript((input) => {
    type TauriArgs = Record<string, unknown> | undefined;
    type ProviderRecord = {
      id: string;
      name: string;
      slug: string;
      status: "active" | "suspended";
      token_ref: string | null;
      created_at: string;
      updated_at: string;
    };

    const providers: ProviderRecord[] = [
      {
        id: "provider-1",
        name: "DigitalOcean",
        slug: "digitalocean",
        status: "active",
        token_ref: "do-token:provider-1:seeded",
        created_at: "2026-08-01T00:00:00Z",
        updated_at: "2026-08-01T00:00:00Z",
      },
      {
        id: "provider-docker",
        name: "Docker",
        slug: "docker",
        status: "active",
        token_ref: null,
        created_at: "2026-08-01T00:01:00Z",
        updated_at: "2026-08-01T00:01:00Z",
      },
    ];

    let diagnosticsNotes = ["token_ref removed from diagnostics bundle"];

    const internals = {
      invoke: async (command: string, args?: TauriArgs) => {
        if (command === "list_vpn_nodes") {
          return [];
        }

        if (command === "get_diagnostics_bundle") {
          return {
            generated_at: "2026-08-01T00:00:00Z",
            providers: providers.map((provider) => ({ ...provider, token_ref: null })),
            nodes: [],
            tunnels: [],
            redacted_notes: diagnosticsNotes,
          };
        }

        if (command === "list_providers") {
          return providers.map((provider) => ({ ...provider }));
        }

        if (command === "validate_provider_token") {
          return "Provider token valid for account acct-1 (active)";
        }

        if (command === "check_docker_readiness") {
          return input;
        }

        if (command === "register_docker_provider_token") {
          const candidate = String(args?.apiToken ?? "").trim();
          if (!(candidate.startsWith("dkr_v1_") || candidate.startsWith("docker_"))) {
            throw new Error("Docker provider token must start with 'dkr_v1_' or 'docker_' for this phase.");
          }

          const provider = providers.find((item) => item.id === "provider-docker");
          if (!provider) {
            throw new Error("Docker provider was not found.");
          }

          provider.token_ref = "do-token:provider-docker:0123456789abcdef";
          provider.updated_at = "2026-08-01T00:03:00Z";
          diagnosticsNotes = ["token_ref removed from diagnostics bundle"];

          return `Provider registered with token_ref: ${provider.token_ref}`;
        }

        throw new Error(`Unhandled mocked command: ${command}`);
      },
    };

    (window as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ = internals;
  }, dockerReadiness);
}

export async function installDockerNodesCreateMock(page: Page): Promise<void> {
  await page.addInitScript(() => {
    type TauriArgs = Record<string, unknown> | undefined;
    type ProviderRecord = {
      id: string;
      name: string;
      slug: string;
      status: "active" | "suspended";
      token_ref: string | null;
      created_at: string;
      updated_at: string;
    };
    type VpnNodeRecord = {
      id: string;
      provider_id: string;
      name: string;
      region: string;
      plan_key: string | null;
      size_slug: string | null;
      price_monthly_usd: number | null;
      price_hourly_usd: number | null;
      price_source_timestamp: string | null;
      lifecycle_status: string;
      external_ref: string | null;
      public_ipv4: string | null;
      server_public_key: string | null;
      error_reason: string | null;
      created_at: string;
      updated_at: string;
    };

    const providers: ProviderRecord[] = [
      {
        id: "provider-1",
        name: "DigitalOcean",
        slug: "digitalocean",
        status: "active",
        token_ref: "do-token:provider-1:seeded",
        created_at: "2026-08-01T00:00:00Z",
        updated_at: "2026-08-01T00:00:00Z",
      },
      {
        id: "provider-docker",
        name: "Docker",
        slug: "docker",
        status: "active",
        token_ref: "do-token:provider-docker:seeded",
        created_at: "2026-08-01T00:01:00Z",
        updated_at: "2026-08-01T00:01:00Z",
      },
    ];

    const nodes: VpnNodeRecord[] = [];
    const tunnels: Array<Record<string, unknown>> = [];

    const pricingByProviderRegion: Record<string, Record<string, Array<Record<string, unknown>>>> = {
      "provider-1": {
        nyc1: [
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
        ],
      },
      "provider-docker": {
        "docker-local": [
          {
            plan_key: "docker-small",
            plan_label: "Docker Small",
            size_slug: "docker-small",
            region: "docker-local",
            memory_mb: 1024,
            vcpus: 1,
            disk_gb: 25,
            transfer_tb: 1,
            transfer_note: "Synthetic plan estimate for local Docker runtime: up to 1.00 TB transfer equivalent.",
            monthly_usd: 6,
            hourly_usd: 0.009,
            estimated: true,
            source_timestamp: "2026-08-01T00:00:00Z",
          },
          {
            plan_key: "docker-medium",
            plan_label: "Docker Medium",
            size_slug: "docker-medium",
            region: "docker-local",
            memory_mb: 4096,
            vcpus: 2,
            disk_gb: 80,
            transfer_tb: 2,
            transfer_note: "Synthetic plan estimate for local Docker runtime: up to 2.00 TB transfer equivalent.",
            monthly_usd: 24,
            hourly_usd: 0.036,
            estimated: true,
            source_timestamp: "2026-08-01T00:00:00Z",
          },
          {
            plan_key: "docker-large",
            plan_label: "Docker Large",
            size_slug: "docker-large",
            region: "docker-local",
            memory_mb: 8192,
            vcpus: 4,
            disk_gb: 160,
            transfer_tb: 4,
            transfer_note: "Synthetic plan estimate for local Docker runtime: up to 4.00 TB transfer equivalent.",
            monthly_usd: 48,
            hourly_usd: 0.072,
            estimated: true,
            source_timestamp: "2026-08-01T00:00:00Z",
          },
        ],
      },
    };

    const commandCounts = {
      list_provider_regions: 0,
      list_provider_pricing: 0,
      create_vpn_node_to_ready: 0,
    };

    const lastCreateRequest: { value: Record<string, unknown> | null } = {
      value: null,
    };

    const internals = {
      invoke: async (command: string, args?: TauriArgs) => {
        if (command === "list_vpn_nodes") {
          return nodes.map((node) => ({ ...node }));
        }

        if (command === "get_diagnostics_bundle") {
          return {
            generated_at: "2026-08-01T00:00:00Z",
            providers: providers.map((provider) => ({ ...provider, token_ref: null })),
            nodes: nodes.map((node) => ({ ...node })),
            tunnels: tunnels.map((tunnel) => ({ ...tunnel })),
            redacted_notes: ["token_ref removed from diagnostics bundle"],
          };
        }

        if (command === "list_providers") {
          return providers.map((provider) => ({ ...provider }));
        }

        if (command === "validate_provider_token") {
          const providerId = String(args?.providerId ?? "provider-1");
          return `Provider token valid for account ${providerId} (active)`;
        }

        if (command === "list_provider_regions") {
          commandCounts.list_provider_regions += 1;
          const providerId = String(args?.providerId ?? "provider-1");
          const regions = Object.keys(pricingByProviderRegion[providerId] ?? {});
          return regions;
        }

        if (command === "list_provider_pricing") {
          commandCounts.list_provider_pricing += 1;
          const providerId = String(args?.providerId ?? "provider-1");
          const region = String(args?.region ?? "");
          return {
            provider_id: providerId,
            region,
            plans: (pricingByProviderRegion[providerId] ?? {})[region] ?? [],
          };
        }

        if (command === "create_vpn_node_to_ready") {
          commandCounts.create_vpn_node_to_ready += 1;
          lastCreateRequest.value = {
            nodeId: args?.nodeId,
            providerId: args?.providerId,
            nodeName: args?.nodeName,
            region: args?.region,
            planKey: args?.planKey,
          };

          const providerId = String(args?.providerId ?? "provider-docker");
          const region = String(args?.region ?? "docker-local");
          const planKey = String(args?.planKey ?? "docker-small");
          const nodeId = String(args?.nodeId ?? "node-docker-1");
          const nodeName = String(args?.nodeName ?? "Docker Node");

          const plan = ((pricingByProviderRegion[providerId] ?? {})[region] ?? []).find(
            (candidate) => String(candidate.plan_key) === planKey,
          );

          nodes.push({
            id: nodeId,
            provider_id: providerId,
            name: nodeName,
            region,
            plan_key: plan ? String(plan.plan_key) : null,
            size_slug: plan ? String(plan.size_slug) : null,
            price_monthly_usd: plan ? Number(plan.monthly_usd) : null,
            price_hourly_usd: plan ? Number(plan.hourly_usd) : null,
            price_source_timestamp: plan ? String(plan.source_timestamp) : null,
            lifecycle_status: "ready",
            external_ref: "docker-123456",
            public_ipv4: "172.18.0.10",
            server_public_key: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
            error_reason: null,
            created_at: "2026-08-01T00:10:00Z",
            updated_at: "2026-08-01T00:10:00Z",
          });

          return `Create node complete: node_id=${nodeId}, droplet_id=123456, endpoint_ip=172.18.0.10, listen_port=51820, correlation_id=test-correlation, idempotent_reuse=false`;
        }

        if (command === "list_tunnels_for_node") {
          const nodeId = String(args?.nodeId ?? "");
          return tunnels.filter((tunnel) => String(tunnel.node_id) === nodeId).map((tunnel) => ({ ...tunnel }));
        }

        if (command === "get_health_panel_data") {
          return {
            node_id: String(args?.nodeId ?? ""),
            tunnel_id: null,
            endpoint_ip: "172.18.0.10",
            handshake_age_seconds: null,
            rx_bytes: null,
            tx_bytes: null,
            external_ipv4: "172.18.0.10",
            tunnel_status: "ready",
          };
        }

        throw new Error(`Unhandled mocked command: ${command}`);
      },
    };

    (window as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ = internals;
    (window as unknown as { __TEST_DOCKER_CREATE_COUNTS__?: typeof commandCounts }).__TEST_DOCKER_CREATE_COUNTS__ = commandCounts;
    (window as unknown as { __TEST_DOCKER_LAST_CREATE_REQUEST__?: Record<string, unknown> | null }).__TEST_DOCKER_LAST_CREATE_REQUEST__ = lastCreateRequest.value;

    const syncState = () => {
      (window as unknown as { __TEST_DOCKER_CREATE_COUNTS__?: typeof commandCounts }).__TEST_DOCKER_CREATE_COUNTS__ = {
        ...commandCounts,
      };
      (window as unknown as { __TEST_DOCKER_LAST_CREATE_REQUEST__?: Record<string, unknown> | null }).__TEST_DOCKER_LAST_CREATE_REQUEST__ = lastCreateRequest.value;
    };

    const originalInvoke = internals.invoke;
    internals.invoke = async (command: string, args?: TauriArgs) => {
      const result = await originalInvoke(command, args);
      syncState();
      return result;
    };
  });
}

export async function installDockerNodesCatalogBlockedProvisioningMock(page: Page): Promise<void> {
  await page.addInitScript(() => {
    type TauriArgs = Record<string, unknown> | undefined;
    type ProviderRecord = {
      id: string;
      name: string;
      slug: string;
      status: "active" | "suspended";
      token_ref: string | null;
      created_at: string;
      updated_at: string;
    };

    const providers: ProviderRecord[] = [
      {
        id: "provider-1",
        name: "DigitalOcean",
        slug: "digitalocean",
        status: "active",
        token_ref: "do-token:provider-1:seeded",
        created_at: "2026-08-01T00:00:00Z",
        updated_at: "2026-08-01T00:00:00Z",
      },
      {
        id: "provider-docker",
        name: "Docker",
        slug: "docker",
        status: "active",
        token_ref: "do-token:provider-docker:seeded",
        created_at: "2026-08-01T00:01:00Z",
        updated_at: "2026-08-01T00:01:00Z",
      },
    ];

    const pricingByProviderRegion: Record<string, Record<string, Array<Record<string, unknown>>>> = {
      "provider-docker": {
        "docker-local": [
          {
            plan_key: "docker-small",
            plan_label: "Docker Small",
            size_slug: "docker-small",
            region: "docker-local",
            memory_mb: 1024,
            vcpus: 1,
            disk_gb: 25,
            transfer_tb: 1,
            transfer_note: "Synthetic plan estimate for local Docker runtime: up to 1.00 TB transfer equivalent.",
            monthly_usd: 6,
            hourly_usd: 0.009,
            estimated: true,
            source_timestamp: "2026-08-01T00:00:00Z",
          },
          {
            plan_key: "docker-medium",
            plan_label: "Docker Medium",
            size_slug: "docker-medium",
            region: "docker-local",
            memory_mb: 4096,
            vcpus: 2,
            disk_gb: 80,
            transfer_tb: 2,
            transfer_note: "Synthetic plan estimate for local Docker runtime: up to 2.00 TB transfer equivalent.",
            monthly_usd: 24,
            hourly_usd: 0.036,
            estimated: true,
            source_timestamp: "2026-08-01T00:00:00Z",
          },
        ],
      },
    };

    const commandCounts = {
      list_provider_regions: 0,
      list_provider_pricing: 0,
      create_vpn_node_to_ready: 0,
    };

    const internals = {
      invoke: async (command: string, args?: TauriArgs) => {
        if (command === "list_vpn_nodes") {
          return [];
        }

        if (command === "get_diagnostics_bundle") {
          return {
            generated_at: "2026-08-01T00:00:00Z",
            providers: providers.map((provider) => ({ ...provider, token_ref: null })),
            nodes: [],
            tunnels: [],
            redacted_notes: ["token_ref removed from diagnostics bundle"],
          };
        }

        if (command === "list_providers") {
          return providers.map((provider) => ({ ...provider }));
        }

        if (command === "validate_provider_token") {
          const providerId = String(args?.providerId ?? "provider-1");
          if (providerId === "provider-docker") {
            throw new Error("Docker CLI is installed, but daemon is not reachable. Start Docker Desktop and retry readiness validation.");
          }
          return `Provider token valid for account ${providerId} (active)`;
        }

        if (command === "list_provider_regions") {
          commandCounts.list_provider_regions += 1;
          const providerId = String(args?.providerId ?? "provider-docker");
          return Object.keys(pricingByProviderRegion[providerId] ?? {});
        }

        if (command === "list_provider_pricing") {
          commandCounts.list_provider_pricing += 1;
          const providerId = String(args?.providerId ?? "provider-docker");
          const region = String(args?.region ?? "docker-local");
          return {
            provider_id: providerId,
            region,
            plans: (pricingByProviderRegion[providerId] ?? {})[region] ?? [],
          };
        }

        if (command === "create_vpn_node_to_ready") {
          commandCounts.create_vpn_node_to_ready += 1;
          throw new Error("Provisioning should stay disabled while Docker readiness is invalid.");
        }

        if (command === "list_tunnels_for_node") {
          return [];
        }

        if (command === "get_health_panel_data") {
          return {
            node_id: String(args?.nodeId ?? ""),
            tunnel_id: null,
            endpoint_ip: null,
            handshake_age_seconds: null,
            rx_bytes: null,
            tx_bytes: null,
            external_ipv4: null,
            tunnel_status: null,
          };
        }

        throw new Error(`Unhandled mocked command: ${command}`);
      },
    };

    (window as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ = internals;
    (window as unknown as { __TEST_DOCKER_CREATE_COUNTS__?: typeof commandCounts }).__TEST_DOCKER_CREATE_COUNTS__ = commandCounts;
  });
}

export async function installDockerTunnelLifecycleMock(page: Page): Promise<void> {
  await page.addInitScript(() => {
    type TauriArgs = Record<string, unknown> | undefined;
    type ProviderRecord = {
      id: string;
      name: string;
      slug: string;
      status: "active" | "suspended";
      token_ref: string | null;
      created_at: string;
      updated_at: string;
    };

    const providers: ProviderRecord[] = [
      {
        id: "provider-docker",
        name: "Docker",
        slug: "docker",
        status: "active",
        token_ref: "do-token:provider-docker:seeded",
        created_at: "2026-08-01T00:00:00Z",
        updated_at: "2026-08-01T00:00:00Z",
      },
    ];

    const node: {
      id: string;
      provider_id: string;
      name: string;
      region: string;
      plan_key: string;
      size_slug: string;
      price_monthly_usd: number;
      price_hourly_usd: number;
      price_source_timestamp: string;
      lifecycle_status: string;
      external_ref: string;
      public_ipv4: string;
      server_public_key: string;
      error_reason: string | null;
      created_at: string;
      updated_at: string;
    } = {
      id: "node-docker-tunnel-1",
      provider_id: "provider-docker",
      name: "Docker Tunnel Node",
      region: "docker-local",
      plan_key: "docker-small",
      size_slug: "docker-small",
      price_monthly_usd: 6,
      price_hourly_usd: 0.009,
      price_source_timestamp: "2026-08-01T00:00:00Z",
      lifecycle_status: "ready",
      external_ref: "docker-67890",
      public_ipv4: "172.18.0.12",
      server_public_key: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
      error_reason: null,
      created_at: "2026-08-01T00:00:00Z",
      updated_at: "2026-08-01T00:00:00Z",
    };

    const tunnel: {
      id: string;
      node_id: string;
      name: string;
      status: string;
      interface_name: string;
      assigned_ipv4: string;
      last_handshake_age_seconds: number | null;
      rx_bytes: number | null;
      tx_bytes: number | null;
      stats_updated_at: string;
      last_error: string | null;
      created_at: string;
      updated_at: string;
    } = {
      id: "tunnel-docker-1",
      node_id: "node-docker-tunnel-1",
      name: "wg-docker-primary",
      status: "disconnected",
      interface_name: "wg-docker-primary",
      assigned_ipv4: "10.55.0.2",
      last_handshake_age_seconds: null,
      rx_bytes: null,
      tx_bytes: null,
      stats_updated_at: "2026-08-01T00:00:00Z",
      last_error: null,
      created_at: "2026-08-01T00:00:00Z",
      updated_at: "2026-08-01T00:00:00Z",
    };

    const commandCounts = {
      connect_tunnel: 0,
      disconnect_tunnel: 0,
      get_health_panel_data: 0,
    };

    const makeHealth = () => {
      const connected = tunnel.status === "connected";
      return {
        node_id: node.id,
        tunnel_id: tunnel.id,
        endpoint_ip: node.public_ipv4,
        handshake_age_seconds: connected ? 7 : null,
        rx_bytes: connected ? 8192 : null,
        tx_bytes: connected ? 16384 : null,
        external_ipv4: node.public_ipv4,
        tunnel_status: connected ? "connected" : "disconnected",
      };
    };

    const internals = {
      invoke: async (command: string, args?: TauriArgs) => {
        if (command === "list_vpn_nodes") {
          return [{ ...node }];
        }

        if (command === "list_tunnels_for_node") {
          const nodeId = String(args?.nodeId ?? "");
          if (nodeId !== node.id) {
            return [];
          }
          return [{ ...tunnel }];
        }

        if (command === "get_diagnostics_bundle") {
          return {
            generated_at: "2026-08-01T00:00:00Z",
            providers: providers.map((provider) => ({ ...provider, token_ref: null })),
            nodes: [{ ...node }],
            tunnels: [{ ...tunnel }],
            redacted_notes: ["token_ref removed from diagnostics bundle"],
          };
        }

        if (command === "list_providers") {
          return providers.map((provider) => ({ ...provider }));
        }

        if (command === "validate_provider_token") {
          return "Provider token valid for account provider-docker (active)";
        }

        if (command === "list_provider_regions") {
          return ["docker-local"];
        }

        if (command === "list_provider_pricing") {
          return {
            provider_id: "provider-docker",
            region: String(args?.region ?? "docker-local"),
            plans: [
              {
                plan_key: "docker-small",
                plan_label: "Docker Small",
                size_slug: "docker-small",
                region: "docker-local",
                memory_mb: 1024,
                vcpus: 1,
                disk_gb: 25,
                transfer_tb: 1,
                transfer_note: "Synthetic plan estimate for local Docker runtime: up to 1.00 TB transfer equivalent.",
                monthly_usd: 6,
                hourly_usd: 0.009,
                estimated: true,
                source_timestamp: "2026-08-01T00:00:00Z",
              },
            ],
          };
        }

        if (command === "connect_tunnel") {
          commandCounts.connect_tunnel += 1;
          tunnel.status = "connected";
          tunnel.last_handshake_age_seconds = 7;
          tunnel.rx_bytes = 8192;
          tunnel.tx_bytes = 16384;
          tunnel.updated_at = "2026-08-01T00:05:00Z";
          node.lifecycle_status = "connected";
          node.updated_at = "2026-08-01T00:05:00Z";
          return;
        }

        if (command === "disconnect_tunnel") {
          commandCounts.disconnect_tunnel += 1;
          tunnel.status = "disconnected";
          tunnel.last_handshake_age_seconds = null;
          tunnel.rx_bytes = null;
          tunnel.tx_bytes = null;
          tunnel.updated_at = "2026-08-01T00:06:00Z";
          node.lifecycle_status = "ready";
          node.updated_at = "2026-08-01T00:06:00Z";
          return;
        }

        if (command === "get_health_panel_data") {
          commandCounts.get_health_panel_data += 1;
          return makeHealth();
        }

        throw new Error(`Unhandled mocked command: ${command}`);
      },
    };

    (window as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ = internals;

    const syncState = () => {
      (window as unknown as {
        __TEST_DOCKER_TUNNEL_COUNTS__?: {
          connect_tunnel: number;
          disconnect_tunnel: number;
          get_health_panel_data: number;
        };
      }).__TEST_DOCKER_TUNNEL_COUNTS__ = { ...commandCounts };
      (window as unknown as { __TEST_DOCKER_TUNNEL_STATUS__?: string }).__TEST_DOCKER_TUNNEL_STATUS__ = tunnel.status;
    };

    const originalInvoke = internals.invoke;
    internals.invoke = async (command: string, args?: TauriArgs) => {
      const result = await originalInvoke(command, args);
      syncState();
      return result;
    };

    syncState();
  });
}

export async function installDockerDestroyCleanupMock(page: Page): Promise<void> {
  await page.addInitScript(() => {
    type TauriArgs = Record<string, unknown> | undefined;
    type ProviderRecord = {
      id: string;
      name: string;
      slug: string;
      status: "active" | "suspended";
      token_ref: string | null;
      created_at: string;
      updated_at: string;
    };

    const providers: ProviderRecord[] = [
      {
        id: "provider-docker",
        name: "Docker",
        slug: "docker",
        status: "active",
        token_ref: "do-token:provider-docker:seeded",
        created_at: "2026-08-01T00:00:00Z",
        updated_at: "2026-08-01T00:00:00Z",
      },
    ];

    const destroyNode = {
      id: "node-docker-destroy-1",
      provider_id: "provider-docker",
      name: "Docker Destroy Node",
      region: "docker-local",
      plan_key: "docker-small",
      size_slug: "docker-small",
      price_monthly_usd: 6,
      price_hourly_usd: 0.009,
      price_source_timestamp: "2026-08-01T00:00:00Z",
      lifecycle_status: "ready",
      external_ref: "docker-destroy-1",
      public_ipv4: "172.18.0.21",
      server_public_key: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
      error_reason: null,
      created_at: "2026-08-01T00:00:00Z",
      updated_at: "2026-08-01T00:00:00Z",
    };

    const recoveryNode = {
      id: "node-docker-recovery-1",
      provider_id: "provider-docker",
      name: "Docker Recovery Node",
      region: "docker-local",
      plan_key: "docker-medium",
      size_slug: "docker-medium",
      price_monthly_usd: 24,
      price_hourly_usd: 0.036,
      price_source_timestamp: "2026-08-01T00:00:00Z",
      lifecycle_status: "error",
      external_ref: "docker-recovery-1",
      public_ipv4: "172.18.0.22",
      server_public_key: "BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB=",
      error_reason: "Bootstrap marker missing.",
      created_at: "2026-08-01T00:01:00Z",
      updated_at: "2026-08-01T00:01:00Z",
    };

    const nodes = [destroyNode, recoveryNode];
    const tunnels = [
      {
        id: "tunnel-docker-destroy-1",
        node_id: destroyNode.id,
        name: "wg-destroy",
        status: "disconnected",
        interface_name: "wg-destroy",
        assigned_ipv4: "10.55.0.11",
        last_handshake_age_seconds: null,
        rx_bytes: null,
        tx_bytes: null,
        stats_updated_at: "2026-08-01T00:00:00Z",
        last_error: null,
        created_at: "2026-08-01T00:00:00Z",
        updated_at: "2026-08-01T00:00:00Z",
      },
      {
        id: "tunnel-docker-recovery-1",
        node_id: recoveryNode.id,
        name: "wg-recovery",
        status: "error",
        interface_name: "wg-recovery",
        assigned_ipv4: "10.55.0.12",
        last_handshake_age_seconds: null,
        rx_bytes: null,
        tx_bytes: null,
        stats_updated_at: "2026-08-01T00:00:00Z",
        last_error: "stale interface",
        created_at: "2026-08-01T00:01:00Z",
        updated_at: "2026-08-01T00:01:00Z",
      },
    ];

    let destroyApplied = false;
    let recoveryCleanupApplied = false;

    const commandCounts = {
      destroy_vpn_node: 0,
      force_cleanup_vpn_node: 0,
      list_vpn_nodes: 0,
    };

    const internals = {
      invoke: async (command: string, args?: TauriArgs) => {
        if (command === "list_vpn_nodes") {
          commandCounts.list_vpn_nodes += 1;
          return nodes.map((node) => ({ ...node }));
        }

        if (command === "list_tunnels_for_node") {
          const nodeId = String(args?.nodeId ?? "");
          return tunnels.filter((tunnel) => tunnel.node_id === nodeId).map((tunnel) => ({ ...tunnel }));
        }

        if (command === "get_diagnostics_bundle") {
          return {
            generated_at: "2026-08-01T00:00:00Z",
            providers: providers.map((provider) => ({ ...provider, token_ref: null })),
            nodes: nodes.map((node) => ({ ...node })),
            tunnels: tunnels.map((tunnel) => ({ ...tunnel })),
            redacted_notes: ["token_ref removed from diagnostics bundle"],
          };
        }

        if (command === "list_providers") {
          return providers.map((provider) => ({ ...provider }));
        }

        if (command === "validate_provider_token") {
          return "Provider token valid for account provider-docker (active)";
        }

        if (command === "list_provider_regions") {
          return ["docker-local"];
        }

        if (command === "list_provider_pricing") {
          return {
            provider_id: "provider-docker",
            region: String(args?.region ?? "docker-local"),
            plans: [
              {
                plan_key: "docker-small",
                plan_label: "Docker Small",
                size_slug: "docker-small",
                region: "docker-local",
                memory_mb: 1024,
                vcpus: 1,
                disk_gb: 25,
                transfer_tb: 1,
                transfer_note: "Synthetic plan estimate for local Docker runtime: up to 1.00 TB transfer equivalent.",
                monthly_usd: 6,
                hourly_usd: 0.009,
                estimated: true,
                source_timestamp: "2026-08-01T00:00:00Z",
              },
              {
                plan_key: "docker-medium",
                plan_label: "Docker Medium",
                size_slug: "docker-medium",
                region: "docker-local",
                memory_mb: 4096,
                vcpus: 2,
                disk_gb: 80,
                transfer_tb: 2,
                transfer_note: "Synthetic plan estimate for local Docker runtime: up to 2.00 TB transfer equivalent.",
                monthly_usd: 24,
                hourly_usd: 0.036,
                estimated: true,
                source_timestamp: "2026-08-01T00:00:00Z",
              },
            ],
          };
        }

        if (command === "get_health_panel_data") {
          const nodeId = String(args?.nodeId ?? destroyNode.id);
          return {
            node_id: nodeId,
            tunnel_id: tunnels.find((tunnel) => tunnel.node_id === nodeId)?.id ?? null,
            endpoint_ip: nodes.find((node) => node.id === nodeId)?.public_ipv4 ?? null,
            handshake_age_seconds: null,
            rx_bytes: null,
            tx_bytes: null,
            external_ipv4: nodes.find((node) => node.id === nodeId)?.public_ipv4 ?? null,
            tunnel_status: nodes.find((node) => node.id === nodeId)?.lifecycle_status ?? "unknown",
          };
        }

        if (command === "destroy_vpn_node") {
          commandCounts.destroy_vpn_node += 1;
          const nodeId = String(args?.nodeId ?? "");
          if (nodeId === destroyNode.id) {
            if (!destroyApplied) {
              destroyApplied = true;
              destroyNode.lifecycle_status = "deleting";
              destroyNode.updated_at = "2026-08-01T00:04:00Z";
            }
            return;
          }

          return;
        }

        if (command === "force_cleanup_vpn_node") {
          commandCounts.force_cleanup_vpn_node += 1;
          const nodeId = String(args?.nodeId ?? "");
          if (nodeId === recoveryNode.id) {
            const removed = recoveryCleanupApplied ? 0 : 1;
            recoveryCleanupApplied = true;
            recoveryNode.lifecycle_status = "error";
            recoveryNode.updated_at = "2026-08-01T00:05:00Z";

            return {
              node_id: recoveryNode.id,
              correlation_id: "corr-docker-force-cleanup",
              local_tunnels_removed: removed,
              remote_droplet_delete_attempted: true,
              remote_droplet_delete_succeeded: true,
              node_record_removed: false,
            };
          }

          return {
            node_id: nodeId,
            correlation_id: "corr-docker-force-cleanup-noop",
            local_tunnels_removed: 0,
            remote_droplet_delete_attempted: false,
            remote_droplet_delete_succeeded: false,
            node_record_removed: false,
          };
        }

        throw new Error(`Unhandled mocked command: ${command}`);
      },
    };

    (window as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ = internals;

    const syncState = () => {
      (window as unknown as {
        __TEST_DOCKER_DESTROY_CLEANUP_COUNTS__?: {
          destroy_vpn_node: number;
          force_cleanup_vpn_node: number;
          list_vpn_nodes: number;
        };
      }).__TEST_DOCKER_DESTROY_CLEANUP_COUNTS__ = { ...commandCounts };
    };

    const originalInvoke = internals.invoke;
    internals.invoke = async (command: string, args?: TauriArgs) => {
      const result = await originalInvoke(command, args);
      syncState();
      return result;
    };

    syncState();
  });
}

export async function installDockerHostInstallAndConnectMock(
  page: Page,
  dockerApiToken: string,
): Promise<void> {
  await page.addInitScript((token) => {
    type TauriArgs = Record<string, unknown> | undefined;
    type ProviderRecord = {
      id: string;
      name: string;
      slug: string;
      status: "active" | "suspended";
      token_ref: string | null;
      created_at: string;
      updated_at: string;
    };

    const providers: ProviderRecord[] = [
      {
        id: "provider-docker",
        name: "Docker",
        slug: "docker",
        status: "active",
        token_ref: `docker-token:provider-docker:${token}`,
        created_at: "2026-08-01T00:00:00Z",
        updated_at: "2026-08-01T00:00:00Z",
      },
    ];

    const node = {
      id: "node-docker-host-1",
      provider_id: "provider-docker",
      name: "Docker Host Node",
      region: "docker-local",
      plan_key: "docker-small",
      size_slug: "docker-small",
      price_monthly_usd: 6,
      price_hourly_usd: 0.009,
      price_source_timestamp: "2026-08-01T00:00:00Z",
      lifecycle_status: "new",
      external_ref: null as string | null,
      public_ipv4: null as string | null,
      server_public_key: null as string | null,
      error_reason: null as string | null,
      created_at: "2026-08-01T00:00:00Z",
      updated_at: "2026-08-01T00:00:00Z",
    };

    const tunnels: Array<{
      id: string;
      node_id: string;
      name: string;
      status: string;
      interface_name: string;
      assigned_ipv4: string;
      last_handshake_age_seconds: number | null;
      rx_bytes: number | null;
      tx_bytes: number | null;
      stats_updated_at: string;
      last_error: string | null;
      created_at: string;
      updated_at: string;
    }> = [];

    const commandCounts = {
      create_vpn_node_to_ready: 0,
      create_and_activate_client_tunnel: 0,
      connect_tunnel: 0,
      get_health_panel_data: 0,
    };

    let lastCreateNodeRequest: Record<string, unknown> | null = null;
    let lastCreateTunnelRequest: Record<string, unknown> | null = null;

    const makeHealth = (nodeId: string) => {
      const tunnel = tunnels.find((item) => item.node_id === nodeId) ?? null;
      const connected = tunnel?.status === "connected";
      return {
        node_id: nodeId,
        tunnel_id: tunnel?.id ?? null,
        endpoint_ip: node.public_ipv4,
        handshake_age_seconds: connected ? 5 : null,
        rx_bytes: connected ? 12288 : null,
        tx_bytes: connected ? 24576 : null,
        external_ipv4: node.public_ipv4,
        tunnel_status: connected ? "connected" : node.lifecycle_status,
      };
    };

    const internals = {
      invoke: async (command: string, args?: TauriArgs) => {
        if (command === "list_vpn_nodes") {
          return [{ ...node }];
        }

        if (command === "get_diagnostics_bundle") {
          return {
            generated_at: "2026-08-01T00:00:00Z",
            providers: providers.map((provider) => ({ ...provider, token_ref: null })),
            nodes: [{ ...node }],
            tunnels: tunnels.map((tunnel) => ({ ...tunnel })),
            redacted_notes: ["token_ref removed from diagnostics bundle"],
          };
        }

        if (command === "list_providers") {
          return providers.map((provider) => ({ ...provider }));
        }

        if (command === "validate_provider_token") {
          return "Provider token valid for account provider-docker (active)";
        }

        if (command === "check_docker_readiness") {
          return {
            installed: true,
            daemon_reachable: true,
            cli_version: "Docker version 27.0.0, build test",
            daemon_version: "27.0.0",
            message: "Docker CLI and daemon are available for development emulation.",
            action_guidance: "You can use Docker development mode workflows.",
          };
        }

        if (command === "list_provider_regions") {
          return ["docker-local"];
        }

        if (command === "list_provider_pricing") {
          return {
            provider_id: "provider-docker",
            region: String(args?.region ?? "docker-local"),
            plans: [
              {
                plan_key: "docker-small",
                plan_label: "Docker Small",
                size_slug: "docker-small",
                region: "docker-local",
                memory_mb: 1024,
                vcpus: 1,
                disk_gb: 25,
                transfer_tb: 1,
                transfer_note: "Synthetic plan estimate for local Docker runtime: up to 1.00 TB transfer equivalent.",
                monthly_usd: 6,
                hourly_usd: 0.009,
                estimated: true,
                source_timestamp: "2026-08-01T00:00:00Z",
              },
            ],
          };
        }

        if (command === "create_vpn_node_to_ready") {
          commandCounts.create_vpn_node_to_ready += 1;
          lastCreateNodeRequest = {
            nodeId: args?.nodeId,
            providerId: args?.providerId,
            nodeName: args?.nodeName,
            region: args?.region,
            planKey: args?.planKey,
          };

          node.lifecycle_status = "ready";
          node.external_ref = "docker-wg-host-1";
          node.public_ipv4 = "127.0.0.1";
          node.server_public_key = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
          node.updated_at = "2026-08-01T00:10:00Z";

          return `Create node complete: node_id=${node.id}, droplet_id=1001, endpoint_ip=${node.public_ipv4}, listen_port=51820, correlation_id=test-correlation, idempotent_reuse=false`;
        }

        if (command === "list_tunnels_for_node") {
          const nodeId = String(args?.nodeId ?? "");
          return tunnels.filter((tunnel) => tunnel.node_id === nodeId).map((tunnel) => ({ ...tunnel }));
        }

        if (command === "create_and_activate_client_tunnel") {
          commandCounts.create_and_activate_client_tunnel += 1;
          lastCreateTunnelRequest = {
            tunnelId: args?.tunnelId,
            nodeId: args?.nodeId,
            tunnelName: args?.tunnelName,
            clientAddressCidr: args?.clientAddressCidr,
            dnsServer: args?.dnsServer,
            allowedIps: args?.allowedIps,
          };

          const tunnelId = String(args?.tunnelId ?? "tunnel-host-1");
          const tunnelName = String(args?.tunnelName ?? "wg-primary");
          const nodeId = String(args?.nodeId ?? node.id);

          const existingIndex = tunnels.findIndex((tunnel) => tunnel.id === tunnelId);
          const tunnelRecord = {
            id: tunnelId,
            node_id: nodeId,
            name: tunnelName,
            status: "connected",
            interface_name: tunnelName,
            assigned_ipv4: "10.66.0.2",
            last_handshake_age_seconds: 5,
            rx_bytes: 12288,
            tx_bytes: 24576,
            stats_updated_at: "2026-08-01T00:12:00Z",
            last_error: null,
            created_at: "2026-08-01T00:12:00Z",
            updated_at: "2026-08-01T00:12:00Z",
          };

          if (existingIndex >= 0) {
            tunnels[existingIndex] = tunnelRecord;
          } else {
            tunnels.push(tunnelRecord);
          }

          node.lifecycle_status = "connected";
          node.updated_at = "2026-08-01T00:12:00Z";

          return `Tunnel connected: tunnel_id=${tunnelId}, tunnel_name=${tunnelName}, client_public_key=BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB=`;
        }

        if (command === "connect_tunnel") {
          commandCounts.connect_tunnel += 1;
          const tunnelId = String(args?.tunnelId ?? "");
          const tunnel = tunnels.find((item) => item.id === tunnelId);
          if (tunnel) {
            tunnel.status = "connected";
            tunnel.last_handshake_age_seconds = 5;
            tunnel.rx_bytes = 12288;
            tunnel.tx_bytes = 24576;
            tunnel.updated_at = "2026-08-01T00:13:00Z";
          }
          node.lifecycle_status = "connected";
          node.updated_at = "2026-08-01T00:13:00Z";
          return;
        }

        if (command === "get_health_panel_data") {
          commandCounts.get_health_panel_data += 1;
          return makeHealth(String(args?.nodeId ?? node.id));
        }

        if (command === "disconnect_tunnel") {
          const tunnelId = String(args?.tunnelId ?? "");
          const tunnel = tunnels.find((item) => item.id === tunnelId);
          if (tunnel) {
            tunnel.status = "disconnected";
            tunnel.last_handshake_age_seconds = null;
            tunnel.rx_bytes = null;
            tunnel.tx_bytes = null;
          }
          node.lifecycle_status = "ready";
          node.updated_at = "2026-08-01T00:14:00Z";
          return;
        }

        throw new Error(`Unhandled mocked command: ${command}`);
      },
    };

    (window as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ = internals;

    const syncState = () => {
      (window as unknown as {
        __TEST_DOCKER_HOST_CONNECT_COUNTS__?: {
          create_vpn_node_to_ready: number;
          create_and_activate_client_tunnel: number;
          connect_tunnel: number;
          get_health_panel_data: number;
        };
      }).__TEST_DOCKER_HOST_CONNECT_COUNTS__ = { ...commandCounts };

      (window as unknown as { __TEST_DOCKER_HOST_CREATE_REQUEST__?: Record<string, unknown> | null }).__TEST_DOCKER_HOST_CREATE_REQUEST__ = lastCreateNodeRequest;
      (window as unknown as { __TEST_DOCKER_TUNNEL_CREATE_REQUEST__?: Record<string, unknown> | null }).__TEST_DOCKER_TUNNEL_CREATE_REQUEST__ = lastCreateTunnelRequest;
      (window as unknown as { __TEST_DOCKER_CURRENT_NODE_STATE__?: string }).__TEST_DOCKER_CURRENT_NODE_STATE__ = node.lifecycle_status;
    };

    const originalInvoke = internals.invoke;
    internals.invoke = async (command: string, args?: TauriArgs) => {
      const result = await originalInvoke(command, args);
      syncState();
      return result;
    };

    syncState();
  }, dockerApiToken);
}
