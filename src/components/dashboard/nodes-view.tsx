"use client";

import { useMemo } from "react";

import { Button, Panel, Field, Modal, PageChrome, type DashboardBusyAction, type ProviderAccessState, type ProviderPlanPricing, type ProviderRecord, type TunnelRecord, type VpnNodeRecord, type PlanDisplayModel, type HealthPanelData } from "./shared";
import { formatCreatedTime, formatCurrency, formatMemory, getNodePricingDisplay } from "./shared";
import { DeviceSlots } from "./device-slots";
import type { NodePeerRecord, PeerConfigPayload } from "@/lib/tauri";
import {
  NODE_STATUS_LABEL,
  isConnectable,
  type NodeRow,
  type NodeSortOrder,
  type NodeStatus,
} from "@/lib/node-display";

/** One treatment per state, so a powered-on server no longer looks identical to a connected one. */
const NODE_STATUS_CLASSES: Record<NodeStatus, string> = {
  connected: "border-[color:rgba(78,222,163,0.35)] bg-[color:rgba(78,222,163,0.12)] text-[var(--secondary)]",
  // Amber, and deliberately not the green used for `connected`: during this state the user has no
  // internet, so it must not be mistaken at a glance for a working tunnel.
  connecting: "border-[color:rgba(255,185,95,0.35)] bg-[color:rgba(255,185,95,0.12)] text-[var(--tertiary)]",
  current: "border-[color:rgba(171,199,255,0.35)] bg-[color:rgba(171,199,255,0.12)] text-[var(--primary)]",
  ready: "border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.04)] text-[var(--on-surface-variant)]",
  provisioning: "border-[color:rgba(171,199,255,0.25)] bg-[color:rgba(171,199,255,0.07)] text-[var(--on-surface-variant)]",
  unavailable: "border-[color:rgba(255,185,95,0.3)] bg-[color:rgba(255,185,95,0.1)] text-[var(--tertiary)]",
  error: "border-[color:rgba(255,155,155,0.35)] bg-[color:rgba(255,155,155,0.1)] text-[#ff9b9b]",
};

function StatusPill({ status }: { status: NodeStatus }) {
  return (
    <span className={`inline-flex rounded-full border px-2.5 py-1 text-[10px] uppercase tracking-[0.2em] ${NODE_STATUS_CLASSES[status]}`}>
      {NODE_STATUS_LABEL[status]}
    </span>
  );
}

export function NodesView({
  providerAccessState,
  providers,
  nodes,
  rows,
  diagnostics,
  pricingByRegion,
  statusFilter,
  setStatusFilter,
  regionFilter,
  setRegionFilter,
  sort,
  setSort,
  statusOptions,
  regionOptions,
  duplicatedRegionSlugs,
  currentNodeId,
  selectedNodeId,
  selectedNode,
  health,
  selectedNodePricing,
  selectedNodeActionMessage,
  selectedNodeTunnel,
  busyAction,
  onNavigateToCreate,
  onInstallHost,
  onDestroyNode,
  onSetCurrentNode,
  onConnectNode,
  onConnectTunnel,
  onDisconnectTunnel,
  onForceDisconnectNodeService,
  nodePeers,
  onClaimDevice,
  onLoadDeviceConfig,
  onExportDeviceConfig,
  onCopyText,
  onForceCleanupSelectedNode,
  isDetailsModalOpen,
  onOpenNodeDetails,
  onCloseNodeDetails,
}: {
  providerAccessState: ProviderAccessState;
  providers: ProviderRecord[];
  nodes: VpnNodeRecord[];
  rows: NodeRow[];
  diagnostics: { tunnels?: TunnelRecord[] } | null;
  pricingByRegion: Record<string, ProviderPlanPricing[]>;
  statusFilter: NodeStatus | "all";
  setStatusFilter: (value: NodeStatus | "all") => void;
  regionFilter: string;
  setRegionFilter: (value: string) => void;
  sort: NodeSortOrder;
  setSort: (value: NodeSortOrder) => void;
  statusOptions: NodeStatus[];
  regionOptions: Array<{ slug: string; label: string }>;
  duplicatedRegionSlugs: Set<string>;
  currentNodeId: string | null;
  selectedNodeId: string;
  selectedNode: VpnNodeRecord | null;
  health: HealthPanelData | null;
  selectedNodePricing: PlanDisplayModel | null;
  selectedNodeActionMessage: string;
  selectedNodeTunnel: TunnelRecord | null;
  busyAction: DashboardBusyAction;
  onNavigateToCreate: () => void;
  onInstallHost: (node: VpnNodeRecord) => Promise<void>;
  onDestroyNode: (node: VpnNodeRecord) => Promise<void>;
  onSetCurrentNode: (nodeId: string) => Promise<void>;
  onConnectNode: (node: VpnNodeRecord) => Promise<void>;
  onConnectTunnel: (tunnel: TunnelRecord) => Promise<void>;
  onDisconnectTunnel: (tunnel: TunnelRecord) => Promise<void>;
  onForceDisconnectNodeService: (node: VpnNodeRecord) => Promise<void>;
  nodePeers: NodePeerRecord[];
  onClaimDevice: (nodeId: string, peerId: string, label: string) => Promise<boolean>;
  onLoadDeviceConfig: (nodeId: string, peerId: string) => Promise<PeerConfigPayload | null>;
  onExportDeviceConfig: (nodeId: string, peerId: string) => Promise<void>;
  onCopyText: (value: string, label: string) => Promise<void>;
  onForceCleanupSelectedNode: () => Promise<void>;
  isDetailsModalOpen: boolean;
  onOpenNodeDetails: (nodeId: string) => void;
  onCloseNodeDetails: () => void;
}) {
  const providerNameById = useMemo(
    () => new Map(providers.map((provider) => [provider.id, provider.name])),
    [providers],
  );

  return (
    <PageChrome>
      <div className="space-y-4">
        {/* The page header above already names this screen; a second identical title was the
            "Node Management" repeated twice in the original layout. */}
        <Panel
          title="Fleet"
          description="Existing node records stay visible even if the provider key becomes invalid."
          action={<Button tone="ghost" onClick={onNavigateToCreate} disabled={busyAction !== null}>Pick a city</Button>}
        >
          <div className="flex flex-col gap-3 border-b border-[color:var(--outline-variant)] pb-4 md:flex-row md:items-end">
            <Field label="Status">
              <select value={statusFilter} onChange={(event) => setStatusFilter(event.target.value as NodeStatus | "all")} className="w-full rounded-xl border border-[color:var(--outline-variant)] bg-[color:rgba(8,10,16,0.9)] px-3 py-2.5 text-[var(--on-surface)] outline-none ring-0 md:min-w-[170px]">
                <option value="all">All statuses</option>
                {statusOptions.map((status) => <option key={status} value={status}>{NODE_STATUS_LABEL[status]}</option>)}
              </select>
            </Field>
            <Field label="Location">
              <select value={regionFilter} onChange={(event) => setRegionFilter(event.target.value)} className="w-full rounded-xl border border-[color:var(--outline-variant)] bg-[color:rgba(8,10,16,0.9)] px-3 py-2.5 text-[var(--on-surface)] outline-none ring-0 md:min-w-[200px]">
                <option value="all">All locations</option>
                {regionOptions.map((region) => <option key={region.slug} value={region.slug}>{region.label}</option>)}
              </select>
            </Field>
            <Field label="Sort">
              <select value={sort} onChange={(event) => setSort(event.target.value as NodeSortOrder)} className="w-full rounded-xl border border-[color:var(--outline-variant)] bg-[color:rgba(8,10,16,0.9)] px-3 py-2.5 text-[var(--on-surface)] outline-none ring-0 md:min-w-[170px]">
                <option value="location">Location A–Z</option>
                <option value="newest">Newest first</option>
                <option value="oldest">Oldest first</option>
              </select>
            </Field>
          </div>

          {nodes.length === 0 ? (
            <div className="rounded-[18px] border border-dashed border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.02)] p-6 text-sm text-[var(--on-surface-variant)]">
              {providerAccessState === "ready"
                ? "No servers yet. Pick a city on the map to build your first one."
                : "No nodes yet. Save and validate the selected provider token in Setup before creating your first node."}
            </div>
          ) : rows.length === 0 ? (
            <div className="rounded-[18px] border border-dashed border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.02)] p-6 text-sm text-[var(--on-surface-variant)]">
              No servers match these filters.
            </div>
          ) : (
            <div className="overflow-x-auto">
              <table className="min-w-full text-left text-sm">
                <thead className="text-[11px] uppercase tracking-[0.24em] text-[var(--on-surface-variant)]">
                  <tr>
                    <th className="px-4 py-3">Location</th>
                    <th className="px-4 py-3">Status</th>
                    <th className="px-4 py-3">Price</th>
                    <th className="px-4 py-3 text-right">Actions</th>
                  </tr>
                </thead>
                <tbody>
                  {rows.map(({ node, status, location }) => {
                    const diagnosticsTunnelForNode = (diagnostics?.tunnels ?? []).find((tunnel) => tunnel.node_id === node.id) ?? null;
                    const liveSelectedTunnelForNode = node.id === selectedNodeId ? selectedNodeTunnel : null;
                    const tunnelForNode = liveSelectedTunnelForNode ?? diagnosticsTunnelForNode;
                    // Derived from the one shared status, not a per-row optimistic shadow. The row
                    // used to flip to "Disconnect" the moment it was clicked, which asserted a
                    // working tunnel during the exact window where the user has no internet.
                    const isConnectedRow = status === "connected";
                    const isConnectingRow = status === "connecting";
                    const canInstallHost = node.lifecycle_status.toLowerCase() === "new";
                    const canToggleTunnel = isConnectable(status) || isConnectedRow || !!tunnelForNode;
                    const nodePricing = getNodePricingDisplay(node, pricingByRegion);
                    const monthly = node.price_monthly_usd ?? nodePricing?.monthlyUsd;

                    return (
                      <tr
                        key={node.id}
                        // Stable hook for the E2E suite: the row no longer always renders the node
                        // name, so text matching is not a reliable way to find a specific server.
                        data-node-id={node.id}
                        onClick={() => onOpenNodeDetails(node.id)}
                        className={`cursor-pointer border-t border-[color:var(--outline-variant)] transition-colors hover:bg-[color:rgba(255,255,255,0.03)] ${selectedNodeId === node.id && isDetailsModalOpen ? "bg-[color:rgba(171,199,255,0.08)]" : "bg-transparent"}`}
                      >
                        <td className="px-4 py-4 align-middle">
                          <div className="flex items-center gap-3">
                            {location.countryCode ? <span className="shrink-0 border-2 border-line-3 px-1.5 font-mono text-[11.5px] text-stone">{location.countryCode}</span> : null}
                            <div>
                              <div className={`font-medium ${location.unresolved ? "font-mono text-[13px] text-[var(--on-surface-variant)]" : "text-[var(--on-surface)]"}`}>{location.primary}</div>
                              {location.secondary ? <div className="mt-0.5 text-xs text-[var(--on-surface-variant)]">{location.secondary}</div> : null}
                              {/* Location leads the row, so same-city servers would otherwise be indistinguishable. */}
                              {duplicatedRegionSlugs.has(node.region) ? <div className="mt-0.5 text-xs text-[var(--on-surface-variant)]">{node.name}</div> : null}
                            </div>
                          </div>
                        </td>
                        <td className="px-4 py-4 align-middle"><StatusPill status={status} /></td>
                        <td className="px-4 py-4 align-middle text-[var(--on-surface)]">
                          {monthly === null || monthly === undefined ? "n/a" : <>{formatCurrency(monthly)}<span className="text-[var(--on-surface-variant)]">/mo</span></>}
                        </td>
                        <td className="px-4 py-4 align-middle" onClick={(event) => event.stopPropagation()}>
                          <div className="flex items-center justify-end gap-2">
                            {status === "error" ? (
                              // A node in Error cannot be connected or re-created under the same
                              // id until it is cleaned up, so cleanup is the only useful action —
                              // offering a dead Connect button here just strands the user.
                              <Button tone="ghost" onClick={() => onOpenNodeDetails(node.id)} disabled={busyAction !== null} className="px-3 py-2 text-xs">
                                Resolve
                              </Button>
                            ) : canInstallHost ? (
                              <Button tone="primary" onClick={() => void onInstallHost(node)} disabled={busyAction !== null} className="px-3 py-2 text-xs">
                                {busyAction === "create-node" ? "Installing" : "Install Host"}
                              </Button>
                            ) : (
                              <Button
                                tone={isConnectedRow ? "destructive" : "primary"}
                                onClick={() => {
                                  if (isConnectedRow) {
                                    void (tunnelForNode
                                      ? onDisconnectTunnel(tunnelForNode)
                                      : onForceDisconnectNodeService(node));
                                    return;
                                  }

                                  void (tunnelForNode ? onConnectTunnel(tunnelForNode) : onConnectNode(node));
                                }}
                                disabled={busyAction !== null || isConnectingRow || !canToggleTunnel}
                                className="min-w-[104px] px-3 py-2 text-xs"
                              >
                                {isConnectingRow ? "Connecting" : isConnectedRow ? "Disconnect" : "Connect"}
                              </Button>
                            )}
                            <button
                              type="button"
                              onClick={() => onOpenNodeDetails(node.id)}
                              aria-label={`Details for ${node.name}`}
                              title="Details"
                              className="rounded-lg px-2 py-2 text-[var(--on-surface-variant)] transition-colors hover:bg-[color:rgba(255,255,255,0.06)] hover:text-[var(--on-surface)]"
                            >
                              <svg viewBox="0 0 24 24" className="h-4 w-4" aria-hidden="true"><path d="M6 12h.01M12 12h.01M18 12h.01" className="fill-none stroke-current stroke-[2.5]" strokeLinecap="round" /></svg>
                            </button>
                          </div>
                        </td>
                      </tr>
                    );
                  })}
                </tbody>
              </table>
            </div>
          )}
        </Panel>
      </div>

      {isDetailsModalOpen && selectedNode ? (
        <NodeDetailsDrawer
          node={selectedNode}
          providerName={providerNameById.get(selectedNode.provider_id) ?? selectedNode.provider_id}
          isCurrent={currentNodeId === selectedNode.id}
          health={health}
          pricing={selectedNodePricing}
          tunnel={selectedNodeTunnel}
          actionMessage={selectedNodeActionMessage}
          busyAction={busyAction}
          onClose={onCloseNodeDetails}
          onSetCurrentNode={onSetCurrentNode}
          onDestroyNode={onDestroyNode}
          nodePeers={nodePeers}
          onClaimDevice={onClaimDevice}
          onLoadDeviceConfig={onLoadDeviceConfig}
          onExportDeviceConfig={onExportDeviceConfig}
          onCopyText={onCopyText}
          onForceCleanup={onForceCleanupSelectedNode}
        />
      ) : null}
    </PageChrome>
  );
}

function DetailRow({ label, value, mono = false }: { label: string; value: string; mono?: boolean }) {
  return (
    <div className="flex items-start justify-between gap-3">
      <dt className="text-[var(--on-surface-variant)]">{label}</dt>
      <dd className={`max-w-[220px] truncate text-right text-[var(--on-surface)] ${mono ? "font-mono text-[12px]" : ""}`} title={value}>{value}</dd>
    </div>
  );
}

/**
 * Everything the table no longer shows, plus the actions that should never be one stray click away.
 *
 * A slide-over rather than a centred modal so the fleet stays visible behind it — the panel is a
 * detail of the row you came from, not a separate place.
 */
function NodeDetailsDrawer({
  node,
  providerName,
  isCurrent,
  health,
  pricing,
  tunnel,
  actionMessage,
  busyAction,
  onClose,
  onSetCurrentNode,
  onDestroyNode,
  nodePeers,
  onClaimDevice,
  onLoadDeviceConfig,
  onExportDeviceConfig,
  onCopyText,
  onForceCleanup,
}: {
  node: VpnNodeRecord;
  providerName: string;
  isCurrent: boolean;
  health: HealthPanelData | null;
  pricing: PlanDisplayModel | null;
  tunnel: TunnelRecord | null;
  actionMessage: string;
  busyAction: DashboardBusyAction;
  onClose: () => void;
  onSetCurrentNode: (nodeId: string) => Promise<void>;
  onDestroyNode: (node: VpnNodeRecord) => Promise<void>;
  nodePeers: NodePeerRecord[];
  onClaimDevice: (nodeId: string, peerId: string, label: string) => Promise<boolean>;
  onLoadDeviceConfig: (nodeId: string, peerId: string) => Promise<PeerConfigPayload | null>;
  onExportDeviceConfig: (nodeId: string, peerId: string) => Promise<void>;
  onCopyText: (value: string, label: string) => Promise<void>;
  onForceCleanup: () => Promise<void>;
}) {
  const specs = pricing
    ? `${pricing.vcpus ?? "?"} vCPU · ${formatMemory(pricing.memoryMb)} · ${pricing.diskGb ?? "?"} GB`
    : "n/a";

  return (
    <Modal onClose={onClose} labelledBy="node-details-title" placement="end">
      <>
        <div className="flex items-start justify-between gap-4">
          <div>
            <p className="text-xs uppercase tracking-[0.24em] text-[var(--on-surface-variant)]">Server</p>
            <h2 id="node-details-title" className="mt-2 text-2xl font-semibold tracking-tight text-[var(--on-surface)]">{node.name}</h2>
          </div>
          <Button tone="ghost" onClick={onClose} className="px-3 py-2 text-xs">Close</Button>
        </div>

        {!isCurrent ? (
          <div className="mt-5">
            <Button tone="ghost" onClick={() => void onSetCurrentNode(node.id)} disabled={busyAction !== null} className="w-full justify-center">
              Make this my current server
            </Button>
          </div>
        ) : (
          <p className="mt-5 rounded-xl border border-[color:rgba(171,199,255,0.25)] bg-[color:rgba(171,199,255,0.08)] px-3 py-2 text-xs text-[var(--primary)]">
            This is your current server.
          </p>
        )}

        <div className="mt-6 space-y-4">
          <section className="rounded-[18px] border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.02)] p-4">
            <div className="text-[11px] uppercase tracking-[0.24em] text-[var(--on-surface-variant)]">Connection</div>
            <dl className="mt-3 space-y-2 text-sm">
              <DetailRow label="Status" value={health?.tunnel_status ?? node.lifecycle_status} />
              <DetailRow label="Public IP" value={node.public_ipv4 ?? "pending"} mono />
              <DetailRow label="Port" value={node.endpoint_port ? String(node.endpoint_port) : "n/a"} mono />
              <DetailRow label="Tunnel" value={tunnel?.name ?? "None created yet"} />
            </dl>
          </section>

          <section className="rounded-[18px] border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.02)] p-4">
            <div className="text-[11px] uppercase tracking-[0.24em] text-[var(--on-surface-variant)]">Server</div>
            <dl className="mt-3 space-y-2 text-sm">
              <DetailRow label="Provider" value={providerName} />
              <DetailRow label="Region" value={node.region} mono />
              <DetailRow label="Plan" value={pricing?.planLabel ?? node.plan_key ?? "n/a"} />
              <DetailRow label="Size" value={node.size_slug ?? pricing?.sizeSlug ?? "n/a"} mono />
              <DetailRow label="Specs" value={specs} />
              {/* Whatever the owning provider calls this node: a DigitalOcean droplet id, an EC2
                  `region/instance-id`, a GCE `zone/name`, a Docker container id. This read
                  "Droplet ID" until every provider but DigitalOcean made that wrong. */}
              <DetailRow label="Provider ID" value={node.external_ref ?? "n/a"} mono />
              <DetailRow label="Node ID" value={node.id} mono />
              <DetailRow label="Created" value={formatCreatedTime(node.created_at)} />
              <DetailRow label="Server key" value={node.server_public_key ?? "pending"} mono />
            </dl>
          </section>

          <section className="rounded-[18px] border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.02)] p-4">
            <div className="text-[11px] uppercase tracking-[0.24em] text-[var(--on-surface-variant)]">Cost</div>
            <dl className="mt-3 space-y-2 text-sm">
              <DetailRow label="Monthly" value={formatCurrency(node.price_monthly_usd ?? pricing?.monthlyUsd)} />
              <DetailRow label="Hourly" value={formatCurrency(node.price_hourly_usd ?? pricing?.hourlyUsd, 5)} />
              <DetailRow label="Transfer" value={pricing?.transferNote ?? "Estimate unavailable for this node."} />
              <DetailRow label="Source" value={pricing?.estimated ? "Estimated" : "Provider live catalog"} />
            </dl>
          </section>

          <section className="rounded-[18px] border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.02)] p-4 text-sm text-[var(--on-surface-variant)]">
            <div className="font-medium text-[var(--on-surface)]">Next step</div>
            <div className="mt-1">{actionMessage}</div>
          </section>

          <DeviceSlots
            nodeId={node.id}
            peers={nodePeers}
            busyAction={busyAction}
            onClaimDevice={onClaimDevice}
            onLoadDeviceConfig={onLoadDeviceConfig}
            onExportDeviceConfig={onExportDeviceConfig}
            onCopyText={onCopyText}
          />

          {/* Separated because both actions delete a paid server. "Uninstall Host" used to sit in
              every row, coloured red, one click from destroying a server the user is billed for. */}
          <section className="rounded-[18px] border border-[color:rgba(255,155,155,0.25)] bg-[color:rgba(255,155,155,0.05)] p-4">
            <div className="text-[11px] uppercase tracking-[0.24em] text-[#ff9b9b]">Danger zone</div>
            <p className="mt-2 text-xs text-[var(--on-surface-variant)]">
              Turning off deletes the server at {providerName} and stops its billing. Its location stays
              saved, so you can turn it back on from Saved locations; phones will need to scan their new
              codes.
            </p>
            <div className="mt-3 flex flex-wrap gap-2">
              {/* For a failed node this is the recovery path, not a destructive extra: the record
                  blocks its own node ID until removed. Shown first so it reads as the way out. */}
              {node.lifecycle_status === "error" ? (
                <Button tone="primary" onClick={() => void onForceCleanup()} disabled={busyAction !== null} className="px-3 py-2 text-xs">
                  {busyAction === "force-cleanup" ? "Cleaning up" : "Force cleanup"}
                </Button>
              ) : null}
              <Button tone="destructive" onClick={() => void onDestroyNode(node)} disabled={busyAction !== null} className="px-3 py-2 text-xs">
                {busyAction === "destroy-node" ? "Turning off" : "Turn off"}
              </Button>
            </div>
          </section>
        </div>
      </>
    </Modal>
  );
}
