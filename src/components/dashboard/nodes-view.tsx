"use client";

import { useMemo } from "react";

import { AppIcon, Button, Card, Field, MetricCard, PageChrome, type AppView, type DashboardBusyAction, type ProviderAccessState, type ProviderPlanPricing, type ProviderRecord, type RecoveryState, type TunnelRecord, type VpnNodeRecord, type NodeSort, type PlanDisplayModel, type HealthPanelData } from "./shared";
import { formatCreatedTime, formatCurrency, formatMemory, formatRegionLabel, getNodePricingDisplay, statusBadge } from "./shared";

export function NodesView({
  providerAccessState,
  providerBadge,
  providerStatusMessage,
  providerName,
  configuredProvider,
  providers,
  nodes,
  diagnostics,
  pricingByRegion,
  pricingBusy,
  pricingError,
  nodeStatusFilter,
  setNodeStatusFilter,
  nodeRegionFilter,
  setNodeRegionFilter,
  nodeSort,
  setNodeSort,
  selectedNodeId,
  setSelectedNodeId,
  selectedNode,
  health,
  selectedNodePricing,
  selectedNodeActionMessage,
  selectedNodeTunnel,
  busyAction,
  onCreateNode,
  onConnectTunnel,
  onDisconnectTunnel,
  onDestroyNode,
  onForceCleanupSelectedNode,
  onNavigateToView,
  setRegion,
  region,
  setPlanKey,
  planKey,
  setNodeId,
  nodeId,
  setNodeName,
  nodeName,
  nodeFeedback,
  nodesProvisioningDisabled,
  nodeRegions,
  nodeStatuses,
  availableRegions,
  pricingCatalog,
  selectedCreatePlanDisplay,
}: {
  providerAccessState: ProviderAccessState;
  providerBadge: { label: string; classes: string };
  providerStatusMessage: string;
  providerName: string;
  configuredProvider: ProviderRecord | null;
  providers: ProviderRecord[];
  nodes: VpnNodeRecord[];
  diagnostics: { tunnels?: TunnelRecord[] } | null;
  pricingByRegion: Record<string, ProviderPlanPricing[]>;
  pricingBusy: boolean;
  pricingError: string | null;
  nodeStatusFilter: string;
  setNodeStatusFilter: (value: string) => void;
  nodeRegionFilter: string;
  setNodeRegionFilter: (value: string) => void;
  nodeSort: NodeSort;
  setNodeSort: (value: NodeSort) => void;
  selectedNodeId: string;
  setSelectedNodeId: (value: string) => void;
  selectedNode: VpnNodeRecord | null;
  health: HealthPanelData | null;
  selectedNodePricing: PlanDisplayModel | null;
  selectedNodeActionMessage: string;
  selectedNodeTunnel: TunnelRecord | null;
  busyAction: DashboardBusyAction;
  onCreateNode: () => Promise<void>;
  onConnectTunnel: (tunnel: TunnelRecord) => Promise<void>;
  onDisconnectTunnel: (tunnel: TunnelRecord) => Promise<void>;
  onDestroyNode: (node: VpnNodeRecord) => Promise<void>;
  onForceCleanupSelectedNode: () => Promise<void>;
  onNavigateToView: (view: AppView) => void;
  setRegion: (value: string) => void;
  region: string;
  setPlanKey: (value: string) => void;
  planKey: string;
  setNodeId: (value: string) => void;
  nodeId: string;
  setNodeName: (value: string) => void;
  nodeName: string;
  nodeFeedback: string;
  nodesProvisioningDisabled: boolean;
  nodeRegions: string[];
  nodeStatuses: string[];
  availableRegions: string[];
  pricingCatalog: ProviderPlanPricing[];
  selectedCreatePlanDisplay: PlanDisplayModel | null;
}) {
  const providerNameById = useMemo(
    () => new Map(providers.map((provider) => [provider.id, provider.name])),
    [providers],
  );

  return (
    <PageChrome>
      <div className="grid gap-4 xl:grid-cols-[minmax(0,1.45fr)_minmax(360px,0.95fr)]">
        <div className="space-y-4">
          <div className="flex flex-col gap-4 rounded-[24px] border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.02)] p-5 shadow-[var(--shadow-elevated)] lg:flex-row lg:items-end lg:justify-between">
            <div>
              <h2 className="text-[28px] font-semibold tracking-tight text-[var(--on-surface)]">Node Management</h2>
              <p className="mt-1 max-w-3xl text-sm text-[var(--on-surface-variant)]">Provision provider-backed nodes, review lifecycle state, and inspect the pricing snapshot captured when each node was created.</p>
            </div>
            <div className="flex flex-wrap items-center gap-2">
              <span className={`inline-flex rounded-full border px-3 py-1.5 text-xs ${providerBadge.classes}`}>Provider access: {providerBadge.label}</span>
              <Button tone="primary" onClick={() => void onCreateNode()} disabled={busyAction !== null || nodesProvisioningDisabled}><span className="text-lg leading-none">+</span> Create Node</Button>
            </div>
          </div>

          {(providerAccessState === "missing" || providerAccessState === "invalid") ? (
            <div className="flex flex-col gap-3 rounded-[20px] border border-[color:rgba(255,185,95,0.28)] bg-[color:rgba(255,185,95,0.08)] p-4 text-sm text-[var(--tertiary)] lg:flex-row lg:items-center lg:justify-between">
              <div>
                <div className="font-medium text-[var(--on-surface)]">Provisioning is disabled until provider access is healthy.</div>
                <div className="mt-1 text-[var(--on-surface-variant)]">{providerStatusMessage}</div>
              </div>
              <Button tone="secondary" onClick={() => onNavigateToView("settings")}>Open Settings</Button>
            </div>
          ) : null}

          <Card title="Fleet" description="Existing node records stay visible even if the provider key becomes invalid.">
            <div className="flex flex-col gap-3 border-b border-[color:var(--outline-variant)] pb-4 md:flex-row md:items-end">
              <Field label="Status">
                <select value={nodeStatusFilter} onChange={(event) => setNodeStatusFilter(event.target.value)} className="w-full rounded-xl border border-[color:var(--outline-variant)] bg-[color:rgba(8,10,16,0.9)] px-3 py-2.5 text-[var(--on-surface)] outline-none ring-0 md:min-w-[170px]">
                  <option value="all">All statuses</option>
                  {nodeStatuses.map((status) => <option key={status} value={status}>{status}</option>)}
                </select>
              </Field>
              <Field label="Region">
                <select value={nodeRegionFilter} onChange={(event) => setNodeRegionFilter(event.target.value)} className="w-full rounded-xl border border-[color:var(--outline-variant)] bg-[color:rgba(8,10,16,0.9)] px-3 py-2.5 text-[var(--on-surface)] outline-none ring-0 md:min-w-[190px]">
                  <option value="all">All regions</option>
                  {nodeRegions.map((regionValue) => <option key={regionValue} value={regionValue}>{formatRegionLabel(regionValue)}</option>)}
                </select>
              </Field>
              <Field label="Sort">
                <select value={nodeSort} onChange={(event) => setNodeSort(event.target.value as NodeSort)} className="w-full rounded-xl border border-[color:var(--outline-variant)] bg-[color:rgba(8,10,16,0.9)] px-3 py-2.5 text-[var(--on-surface)] outline-none ring-0 md:min-w-[170px]">
                  <option value="newest">Newest first</option>
                  <option value="oldest">Oldest first</option>
                </select>
              </Field>
            </div>

            {nodes.length === 0 ? (
              <div className="rounded-[18px] border border-dashed border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.02)] p-6 text-sm text-[var(--on-surface-variant)]">
                {providerAccessState === "ready"
                  ? "No nodes have been provisioned yet. Use Create Node to launch your first WireGuard server."
                  : "No nodes yet. Save and validate the selected provider token in Settings before creating your first node."}
              </div>
            ) : (
              <div className="overflow-x-auto">
                <table className="min-w-full text-left text-sm">
                  <thead className="text-[11px] uppercase tracking-[0.24em] text-[var(--on-surface-variant)]">
                    <tr>
                      <th className="px-4 py-3">Node</th>
                      <th className="px-4 py-3">Provider</th>
                      <th className="px-4 py-3">Region</th>
                      <th className="px-4 py-3">Plan</th>
                      <th className="px-4 py-3">vCPU / RAM / Disk</th>
                      <th className="px-4 py-3">Public IP</th>
                      <th className="px-4 py-3">Status</th>
                      <th className="px-4 py-3">Created</th>
                      <th className="px-4 py-3 text-right">Actions</th>
                    </tr>
                  </thead>
                  <tbody>
                    {nodes.map((node) => {
                      const tunnelForNode = (diagnostics?.tunnels ?? []).find((tunnel) => tunnel.node_id === node.id) ?? null;
                      const nodeCanConnect = !!tunnelForNode && tunnelForNode.status !== "connected";
                      const nodePricing = getNodePricingDisplay(node, pricingByRegion);
                      return (
                        <tr key={node.id} className={`border-t border-[color:var(--outline-variant)] ${selectedNodeId === node.id ? "bg-[color:rgba(171,199,255,0.08)]" : "bg-transparent"}`}>
                          <td className="px-4 py-4 align-top">
                            <div className="font-medium text-[var(--on-surface)]">{node.name}</div>
                            <div className="mt-1 font-mono text-[12px] text-[var(--on-surface-variant)]">{node.id}</div>
                          </td>
                          <td className="px-4 py-4 align-top text-[var(--on-surface)]">{providerNameById.get(node.provider_id) ?? node.provider_id}</td>
                          <td className="px-4 py-4 align-top text-[var(--on-surface)]">{formatRegionLabel(node.region)}</td>
                          <td className="px-4 py-4 align-top">
                            <div className="text-[var(--on-surface)]">{nodePricing?.planLabel ?? node.plan_key ?? "n/a"}</div>
                            <div className="mt-1 font-mono text-[12px] text-[var(--on-surface-variant)]">{node.size_slug ?? nodePricing?.sizeSlug ?? "n/a"}</div>
                            <div className="mt-1 text-xs text-[var(--on-surface-variant)]">{formatCurrency(node.price_hourly_usd ?? nodePricing?.hourlyUsd, 5)}/hr · {formatCurrency(node.price_monthly_usd ?? nodePricing?.monthlyUsd)}/mo</div>
                          </td>
                          <td className="px-4 py-4 align-top text-[var(--on-surface)]">{nodePricing ? `${nodePricing.vcpus ?? "?"} vCPU · ${formatMemory(nodePricing.memoryMb)} · ${nodePricing.diskGb ?? "?"} GB` : "n/a"}</td>
                          <td className="px-4 py-4 align-top font-mono text-[12px] text-[var(--on-surface)]">{node.public_ipv4 ?? "n/a"}</td>
                          <td className="px-4 py-4 align-top"><span className={`inline-flex rounded-full border px-2.5 py-1 text-[10px] uppercase tracking-[0.24em] ${statusBadge(node.lifecycle_status)}`}>{node.lifecycle_status}</span></td>
                          <td className="px-4 py-4 align-top text-[var(--on-surface-variant)]">{formatCreatedTime(node.created_at)}</td>
                          <td className="px-4 py-4 align-top">
                            <div className="flex flex-wrap justify-end gap-2">
                              <Button tone={nodeCanConnect ? "primary" : tunnelForNode ? "danger" : "secondary"} onClick={() => (tunnelForNode ? (nodeCanConnect ? void onConnectTunnel(tunnelForNode) : void onDisconnectTunnel(tunnelForNode)) : undefined)} disabled={!tunnelForNode || busyAction !== null} className="px-3 py-2 text-xs">{nodeCanConnect ? "Connect" : tunnelForNode ? "Disconnect" : "No Tunnel"}</Button>
                              <Button tone="secondary" onClick={() => setSelectedNodeId(node.id)} disabled={busyAction !== null} className="px-3 py-2 text-xs">View Details</Button>
                              <Button tone="danger" onClick={() => void onDestroyNode(node)} disabled={busyAction !== null} className="px-3 py-2 text-xs">Destroy</Button>
                            </div>
                          </td>
                        </tr>
                      );
                    })}
                  </tbody>
                </table>
              </div>
            )}
          </Card>

          <Card title="Node Details" description="Selected node metadata, pricing snapshot, and latest health metrics.">
            {selectedNode ? (
              <div className="space-y-4">
                <div className="grid gap-3 md:grid-cols-2">
                  <MetricCard title="Endpoint" value={selectedNode.public_ipv4 ?? "pending"} detail={`External ref ${selectedNode.external_ref ?? "n/a"}`} tone="primary" />
                  <MetricCard title="Health" value={health?.tunnel_status ?? selectedNode.lifecycle_status} detail={`Handshake ${health ? "live" : "pending"}`} tone="secondary" />
                </div>
                <div className="grid gap-4 lg:grid-cols-2">
                  <div className="rounded-[18px] border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.02)] p-4">
                    <div className="text-[11px] uppercase tracking-[0.24em] text-[var(--on-surface-variant)]">Provision Metadata</div>
                    <dl className="mt-3 space-y-2 text-sm">
                      <div className="flex items-center justify-between gap-3"><dt className="text-[var(--on-surface-variant)]">Provider</dt><dd className="text-[var(--on-surface)]">{providerNameById.get(selectedNode.provider_id) ?? selectedNode.provider_id}</dd></div>
                      <div className="flex items-center justify-between gap-3"><dt className="text-[var(--on-surface-variant)]">Region</dt><dd className="text-[var(--on-surface)]">{formatRegionLabel(selectedNode.region)}</dd></div>
                      <div className="flex items-center justify-between gap-3"><dt className="text-[var(--on-surface-variant)]">Created</dt><dd className="text-[var(--on-surface)]">{formatCreatedTime(selectedNode.created_at)}</dd></div>
                      <div className="flex items-center justify-between gap-3"><dt className="text-[var(--on-surface-variant)]">Server key</dt><dd className="max-w-[220px] truncate font-mono text-[var(--on-surface)]">{selectedNode.server_public_key ?? "pending"}</dd></div>
                      <div className="flex items-center justify-between gap-3"><dt className="text-[var(--on-surface-variant)]">Latest tunnel</dt><dd className="text-[var(--on-surface)]">{selectedNodeTunnel?.name ?? "None created yet"}</dd></div>
                    </dl>
                  </div>
                  <div className="rounded-[18px] border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.02)] p-4">
                    <div className="text-[11px] uppercase tracking-[0.24em] text-[var(--on-surface-variant)]">Pricing Snapshot</div>
                    <dl className="mt-3 space-y-2 text-sm">
                      <div className="flex items-center justify-between gap-3"><dt className="text-[var(--on-surface-variant)]">Plan</dt><dd className="text-[var(--on-surface)]">{selectedNodePricing?.planLabel ?? selectedNode.plan_key ?? "n/a"}</dd></div>
                      <div className="flex items-center justify-between gap-3"><dt className="text-[var(--on-surface-variant)]">Size</dt><dd className="font-mono text-[var(--on-surface)]">{selectedNode.size_slug ?? selectedNodePricing?.sizeSlug ?? "n/a"}</dd></div>
                      <div className="flex items-center justify-between gap-3"><dt className="text-[var(--on-surface-variant)]">Hourly</dt><dd className="text-[var(--on-surface)]">{formatCurrency(selectedNode.price_hourly_usd ?? selectedNodePricing?.hourlyUsd, 5)}</dd></div>
                      <div className="flex items-center justify-between gap-3"><dt className="text-[var(--on-surface-variant)]">Monthly</dt><dd className="text-[var(--on-surface)]">{formatCurrency(selectedNode.price_monthly_usd ?? selectedNodePricing?.monthlyUsd)}</dd></div>
                      <div className="flex items-start justify-between gap-3"><dt className="text-[var(--on-surface-variant)]">Transfer</dt><dd className="max-w-[220px] text-right text-[var(--on-surface)]">{selectedNodePricing?.transferNote ?? "Estimate unavailable for this node."}</dd></div>
                      <div className="flex items-center justify-between gap-3"><dt className="text-[var(--on-surface-variant)]">Source</dt><dd className="text-[var(--on-surface)]">{selectedNodePricing?.estimated ? "Estimated" : "Provider live catalog"}</dd></div>
                    </dl>
                  </div>
                </div>
                <div className="rounded-[18px] border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.02)] p-4 text-sm text-[var(--on-surface-variant)]">
                  <div className="font-medium text-[var(--on-surface)]">Next step</div>
                  <div className="mt-1">{selectedNodeActionMessage}</div>
                  {selectedNode.lifecycle_status === "error" ? <div className="mt-3"><Button tone="danger" onClick={() => void onForceCleanupSelectedNode()} disabled={busyAction !== null}>Force Cleanup</Button></div> : null}
                </div>
              </div>
            ) : (
              <div className="rounded-[18px] border border-dashed border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.02)] p-5 text-sm text-[var(--on-surface-variant)]">Select a node row to inspect provision metadata, endpoint health, and the pricing snapshot captured at creation time.</div>
            )}
          </Card>
        </div>

        <div className="space-y-4">
          <Card title="Create Node" description="Provision a new node with the currently selected provider token.">
            <div className="space-y-5">
              <div className="rounded-[18px] border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.02)] p-4 text-sm">
                <div className="text-[11px] uppercase tracking-[0.24em] text-[var(--on-surface-variant)]">Provider</div>
                <div className="mt-2 flex items-center justify-between gap-3">
                  <div>
                    <div className="font-medium text-[var(--on-surface)]">{configuredProvider?.name ?? providerName}</div>
                    <div className="mt-1 text-[var(--on-surface-variant)]">Provider and size catalog are loaded dynamically from the selected integration.</div>
                  </div>
                  <span className={`inline-flex rounded-full border px-2.5 py-1 text-xs ${providerBadge.classes}`}>{providerBadge.label}</span>
                </div>
              </div>

              <div className="grid gap-4 md:grid-cols-2">
                <Field label="Node ID" hint="Used for local orchestration records."><input value={nodeId} onChange={(event) => setNodeId(event.target.value)} className="w-full rounded-xl border border-[color:var(--outline-variant)] bg-[color:rgba(8,10,16,0.9)] px-3 py-2.5 text-[var(--on-surface)] outline-none ring-0" /></Field>
                <Field label="Node Name" hint="Shown in the node list and diagnostics."><input value={nodeName} onChange={(event) => setNodeName(event.target.value)} className="w-full rounded-xl border border-[color:var(--outline-variant)] bg-[color:rgba(8,10,16,0.9)] px-3 py-2.5 text-[var(--on-surface)] outline-none ring-0" /></Field>
              </div>

              <Field label="Region" hint="Pricing can vary by region and provider updates.">
                <select value={region} onChange={(event) => setRegion(event.target.value)} className="w-full rounded-xl border border-[color:var(--outline-variant)] bg-[color:rgba(8,10,16,0.9)] px-3 py-2.5 text-[var(--on-surface)] outline-none ring-0">
                  {availableRegions.length === 0 ? <option value="">No regions available</option> : null}
                  {availableRegions.map((option) => <option key={option} value={option}>{formatRegionLabel(option)}</option>)}
                </select>
              </Field>

              <div className="space-y-3">
                <div className="text-[11px] uppercase tracking-[0.24em] text-[var(--on-surface-variant)]">Size / Plan</div>
                <div className="grid gap-3">
                  {pricingCatalog.map((plan) => {
                    const isSelected = plan.plan_key === planKey;
                    return (
                      <button key={`${plan.region}-${plan.plan_key}`} type="button" onClick={() => setPlanKey(plan.plan_key)} className={`rounded-[18px] border px-4 py-4 text-left transition-colors ${isSelected ? "border-[color:rgba(171,199,255,0.35)] bg-[color:rgba(171,199,255,0.12)]" : "border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.02)] hover:bg-[color:rgba(255,255,255,0.04)]"}`}>
                        <div className="flex items-start justify-between gap-4">
                          <div>
                            <div className="font-medium text-[var(--on-surface)]">{plan.plan_label}</div>
                            <div className="mt-1 font-mono text-[12px] text-[var(--on-surface-variant)]">{plan.size_slug}</div>
                            <div className="mt-2 text-sm text-[var(--on-surface-variant)]">{plan.vcpus} vCPU · {formatMemory(plan.memory_mb)} · {plan.disk_gb} GB disk</div>
                          </div>
                          <div className="text-right">
                            <div className="font-medium text-[var(--on-surface)]">{formatCurrency(plan.monthly_usd)}/mo</div>
                            <div className="mt-1 text-xs text-[var(--on-surface-variant)]">{formatCurrency(plan.hourly_usd, 5)}/hr</div>
                            {plan.estimated ? <div className="mt-2 text-[10px] uppercase tracking-[0.24em] text-[var(--tertiary)]">Estimated</div> : null}
                          </div>
                        </div>
                      </button>
                    );
                  })}
                  {pricingCatalog.length === 0 ? (
                    <div className="rounded-[18px] border border-dashed border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.02)] p-4 text-sm text-[var(--on-surface-variant)]">
                      No live plans are available for this region yet. Refresh pricing or choose another region.
                    </div>
                  ) : null}
                </div>
              </div>

              <div className="rounded-[18px] border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.02)] p-4 text-sm">
                <div className="text-[11px] uppercase tracking-[0.24em] text-[var(--on-surface-variant)]">Estimated Cost Preview</div>
                <div className="mt-3 grid gap-2 sm:grid-cols-2">
                  <div className="text-[var(--on-surface-variant)]">Plan name <span className="ml-2 text-[var(--on-surface)]">{selectedCreatePlanDisplay?.planLabel ?? "n/a"}</span></div>
                  <div className="text-[var(--on-surface-variant)]">Size <span className="ml-2 font-mono text-[var(--on-surface)]">{selectedCreatePlanDisplay?.sizeSlug ?? "n/a"}</span></div>
                  <div className="text-[var(--on-surface-variant)]">Hourly <span className="ml-2 text-[var(--on-surface)]">{formatCurrency(selectedCreatePlanDisplay?.hourlyUsd, 5)}</span></div>
                  <div className="text-[var(--on-surface-variant)]">Monthly <span className="ml-2 text-[var(--on-surface)]">{formatCurrency(selectedCreatePlanDisplay?.monthlyUsd)}</span></div>
                  <div className="sm:col-span-2 text-[var(--on-surface-variant)]">Bandwidth <span className="ml-2 text-[var(--on-surface)]">{selectedCreatePlanDisplay?.transferNote ?? "Best available transfer estimate is unavailable."}</span></div>
                </div>
                <div className="mt-3 rounded-xl border border-[color:rgba(255,185,95,0.25)] bg-[color:rgba(255,185,95,0.08)] px-3 py-2 text-xs text-[var(--tertiary)]">Billing preview assumes the provider catalog available at provisioning time. Transfer overages, taxes, and future provider price changes can alter the actual invoice.</div>
              </div>

              {pricingError ? <div className="rounded-xl border border-[color:rgba(255,107,107,0.25)] bg-[color:rgba(255,107,107,0.08)] px-3 py-2 text-sm text-[#ffd5d5]">{pricingError}</div> : null}
              {pricingBusy ? <div className="text-sm text-[var(--on-surface-variant)]">Refreshing pricing catalog...</div> : null}
              <div className="text-sm text-[var(--on-surface-variant)]">{nodeFeedback}</div>

              <div className="flex flex-wrap items-center justify-between gap-3">
                <Button tone="secondary" onClick={() => onNavigateToView("settings")}>Settings</Button>
                <Button onClick={() => void onCreateNode()} disabled={busyAction !== null || nodesProvisioningDisabled || !nodeId.trim() || !nodeName.trim() || !region || !planKey || pricingCatalog.length === 0}>{busyAction === "create-node" ? "Provisioning..." : "Create Node"}</Button>
              </div>
            </div>
          </Card>
        </div>
      </div>
    </PageChrome>
  );
}
