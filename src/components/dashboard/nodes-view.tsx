"use client";

import { useMemo, useState } from "react";

import { Button, Card, Field, MetricCard, PageChrome, type DashboardBusyAction, type ProviderAccessState, type ProviderPlanPricing, type ProviderRecord, type RecoveryState, type TunnelRecord, type VpnNodeRecord, type NodeSort, type PlanDisplayModel, type HealthPanelData } from "./shared";
import { formatCreatedTime, formatCurrency, formatMemory, formatRegionLabel, getNodePricingDisplay, statusBadge } from "./shared";

export function NodesView({
  providerAccessState,
  providerBadge,
  providerId,
  providerName,
  configuredProvider,
  providers,
  onSelectProvider,
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
  onNavigateToCreate,
  onInstallHost,
  onUninstallHost,
  onConnectNode,
  onConnectTunnel,
  onDisconnectTunnel,
  onForceDisconnectNodeService,
  onForceCleanupSelectedNode,
  nodeFeedback,
  nodeRegions,
  nodeStatuses,
  isDetailsModalOpen,
  onOpenNodeDetails,
  onCloseNodeDetails,
}: {
  providerAccessState: ProviderAccessState;
  providerBadge: { label: string; classes: string };
  providerId: string;
  providerName: string;
  configuredProvider: ProviderRecord | null;
  providers: ProviderRecord[];
  onSelectProvider: (providerId: string) => void;
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
  onNavigateToCreate: () => void;
  onInstallHost: (node: VpnNodeRecord) => Promise<void>;
  onUninstallHost: (node: VpnNodeRecord) => Promise<void>;
  onConnectNode: (node: VpnNodeRecord) => Promise<void>;
  onConnectTunnel: (tunnel: TunnelRecord) => Promise<void>;
  onDisconnectTunnel: (tunnel: TunnelRecord) => Promise<void>;
  onForceDisconnectNodeService: (node: VpnNodeRecord) => Promise<void>;
  onForceCleanupSelectedNode: () => Promise<void>;
  nodeFeedback: string;
  nodeRegions: string[];
  nodeStatuses: string[];
  isDetailsModalOpen: boolean;
  onOpenNodeDetails: (nodeId: string) => void;
  onCloseNodeDetails: () => void;
}) {
  const providerNameById = useMemo(
    () => new Map(providers.map((provider) => [provider.id, provider.name])),
    [providers],
  );
  const [optimisticConnectedByNodeId, setOptimisticConnectedByNodeId] = useState<Record<string, boolean>>({});

  return (
    <PageChrome>
      <div className="space-y-4">
        <div className="flex flex-col gap-4 rounded-[24px] border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.02)] p-5 shadow-[var(--shadow-elevated)] lg:flex-row lg:items-end lg:justify-between">
          <div>
            <h2 className="text-[28px] font-semibold tracking-tight text-[var(--on-surface)]">Node Management</h2>
            <p className="mt-1 max-w-3xl text-sm text-[var(--on-surface-variant)]">Provision provider-backed nodes, review lifecycle state, and inspect the pricing snapshot captured when each node was created.</p>
          </div>
          <div className="flex flex-wrap items-center gap-2">
            <Button tone="primary" onClick={onNavigateToCreate} disabled={busyAction !== null}><span className="text-lg leading-none">+</span> Create Node</Button>
          </div>
        </div>

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
                    const nodeLifecycle = node.lifecycle_status.toLowerCase();
                    const liveProviderStatus = (node.provider_status ?? "").toLowerCase();
                    const rowStatus = (node.provider_status ?? node.lifecycle_status).toLowerCase();
                    const diagnosticsTunnelForNode = (diagnostics?.tunnels ?? []).find((tunnel) => tunnel.node_id === node.id) ?? null;
                    const liveSelectedTunnelForNode = node.id === selectedNodeId ? selectedNodeTunnel : null;
                    const tunnelForNode = liveSelectedTunnelForNode ?? diagnosticsTunnelForNode;
                    const healthIndicatesConnected = node.id === selectedNodeId
                      && health?.tunnel_status?.toLowerCase() === "connected";
                    const tunnelConnected = tunnelForNode?.status.toLowerCase() === "connected" || healthIndicatesConnected;
                    const connectedByRuntime = !!tunnelForNode && (tunnelConnected || nodeLifecycle === "connected");
                    const optimisticConnected = optimisticConnectedByNodeId[node.id] ?? connectedByRuntime;
                    const nodeCanConnect = !!tunnelForNode && !optimisticConnected;
                    const runtimeConnectable =
                      liveProviderStatus === ""
                      || liveProviderStatus === "active"
                      || liveProviderStatus === "new";
                    const nodeHostReady = (nodeLifecycle === "ready" || nodeLifecycle === "connected") && runtimeConnectable;
                    const canInstallHost = nodeLifecycle === "new";
                    const hostInstallInProgress = nodeLifecycle === "provisioning" || nodeLifecycle === "bootstrapping";
                    const canUninstallHost = nodeLifecycle === "ready" || nodeLifecycle === "connected" || nodeLifecycle === "error";
                    const canConnectNode = nodeHostReady && !tunnelForNode && !optimisticConnected;
                    const canToggleTunnel = nodeHostReady || !!tunnelForNode || optimisticConnected;
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
                        <td className="px-4 py-4 align-top"><span className={`inline-flex rounded-full border px-2.5 py-1 text-[10px] uppercase tracking-[0.24em] ${statusBadge(rowStatus)}`}>{rowStatus}</span></td>
                        <td className="px-4 py-4 align-top text-[var(--on-surface-variant)]">{formatCreatedTime(node.created_at)}</td>
                        <td className="px-4 py-4 align-top">
                          <div className="flex flex-wrap justify-end gap-2">
                            {canInstallHost ? (
                              <Button tone="primary" onClick={() => void onInstallHost(node)} disabled={busyAction !== null} className="px-3 py-2 text-xs">
                                {busyAction === "create-node" ? "Installing Host" : "Install Host"}
                              </Button>
                            ) : (
                              <Button
                                tone={canUninstallHost ? "danger" : "secondary"}
                                onClick={() => (canUninstallHost ? void onUninstallHost(node) : undefined)}
                                disabled={busyAction !== null || hostInstallInProgress || !canUninstallHost}
                                className="px-3 py-2 text-xs"
                              >
                                {hostInstallInProgress
                                  ? "Installing Host"
                                  : busyAction === "destroy-node"
                                    ? "Uninstalling Host"
                                    : canUninstallHost
                                      ? "Uninstall Host"
                                      : "Host Pending"}
                              </Button>
                            )}
                            <Button
                              tone={optimisticConnected ? "danger" : (nodeCanConnect || canConnectNode ? "primary" : "secondary")}
                              onClick={() => {
                                const previous = optimisticConnected;
                                if (previous) {
                                  setOptimisticConnectedByNodeId((current) => ({ ...current, [node.id]: false }));
                                  void (async () => {
                                    try {
                                      if (tunnelForNode) {
                                        await onDisconnectTunnel(tunnelForNode);
                                      } else {
                                        await onForceDisconnectNodeService(node);
                                      }
                                    } catch {
                                      setOptimisticConnectedByNodeId((current) => ({ ...current, [node.id]: true }));
                                    }
                                  })();
                                  return;
                                }

                                setOptimisticConnectedByNodeId((current) => ({ ...current, [node.id]: true }));
                                void (async () => {
                                  try {
                                    if (tunnelForNode) {
                                      await onConnectTunnel(tunnelForNode);
                                      return;
                                    }

                                    if (canConnectNode) {
                                      await onConnectNode(node);
                                      return;
                                    }

                                    setOptimisticConnectedByNodeId((current) => ({ ...current, [node.id]: previous }));
                                  } catch {
                                    setOptimisticConnectedByNodeId((current) => ({ ...current, [node.id]: false }));
                                  }
                                })();
                              }}
                              disabled={busyAction !== null || !canToggleTunnel}
                              className="px-3 py-2 text-xs"
                            >
                              {optimisticConnected ? "Disconnect" : (nodeCanConnect || canConnectNode ? "Connect" : "Host Pending")}
                            </Button>
                            <Button tone="secondary" onClick={() => onOpenNodeDetails(node.id)} disabled={busyAction !== null} className="px-3 py-2 text-xs">View Details</Button>
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
      </div>

      {isDetailsModalOpen && selectedNode ? (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/75 p-4 backdrop-blur-[2px]" onClick={onCloseNodeDetails} aria-hidden="true">
          <div className="w-full max-w-5xl rounded-[28px] border border-[color:var(--outline-variant)] bg-[linear-gradient(180deg,rgba(24,28,36,0.99),rgba(13,16,23,0.99))] p-6 shadow-[0_30px_90px_rgba(0,0,0,0.45)]" onClick={(event) => event.stopPropagation()}>
            <div className="flex items-start justify-between gap-4">
              <div>
                <p className="text-xs uppercase tracking-[0.24em] text-[var(--on-surface-variant)]">Node Details</p>
                <h2 className="mt-2 text-2xl font-semibold tracking-tight text-[var(--on-surface)]">{selectedNode.name}</h2>
                <p className="mt-1 text-sm text-[var(--on-surface-variant)]">Provision metadata, pricing snapshot, health, and recovery actions.</p>
              </div>
              <Button tone="secondary" onClick={onCloseNodeDetails} disabled={busyAction !== null}>Close</Button>
            </div>

            <div className="mt-6 space-y-4">
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
          </div>
        </div>
      ) : null}
    </PageChrome>
  );
}
