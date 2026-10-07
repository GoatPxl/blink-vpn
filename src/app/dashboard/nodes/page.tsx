"use client";

import { NodesView } from "@/components/dashboard/nodes-view";
import { SavedLocationsView } from "@/components/dashboard/saved-locations-view";
import { useDashboard } from "@/components/dashboard/dashboard-provider";
import { getNodePricingDisplay, NODE_CREATE_ROUTE, PageChrome } from "@/components/dashboard/shared";
import {
  NODE_STATUS_ORDER,
  buildRegionOptions,
  describeLocation,
  deriveNodeStatus,
  duplicatedRegions,
  selectNodeRows,
  type NodeRow,
  type NodeSortOrder,
  type NodeStatus,
} from "@/lib/node-display";
import { loadProviderPricingCatalog } from "@/lib/provider-pricing-cache";
import { type ProviderPlanPricing } from "@/lib/tauri";
import { useRouter } from "next/navigation";
import { useEffect, useMemo, useState } from "react";

export default function DashboardNodesPage() {
  const router = useRouter();
  const {
    busyAction,
    connectAttempt,
    currentNodeId,
    diagnostics,
    forceCleanupSelectedNode,
    health,
    nodes,
    providerAccessState,
    providerId,
    providers,
    selectedNode,
    selectedNodeId,
    setSelectedNodeId,
    setCurrentNode,
    installHostForNode,
    connectNode,
    destroyNode,
    connectTunnel,
    disconnectTunnel,
    forceDisconnectNodeService,
    tunnelConnected,
    tunnels,
    nodePeers,
    claimDevice,
    loadDeviceConfig,
    exportDeviceConfig,
    copyText,
    savedLocations,
    turnOnLocation,
    turnOffLocation,
    forgetLocation,
  } = useDashboard();
  const [statusFilter, setStatusFilter] = useState<NodeStatus | "all">("all");
  const [regionFilter, setRegionFilter] = useState<string>("all");
  const [sort, setSort] = useState<NodeSortOrder>("location");
  const [pricingByRegion, setPricingByRegion] = useState<Record<string, ProviderPlanPricing[]>>({});
  const [isDetailsModalOpen, setIsDetailsModalOpen] = useState(false);

  // The live catalog only enriches: every node also carries the price snapshot taken when it was
  // created, so a failed lookup costs specs detail in the drawer, not the price in the table.
  // Nothing surfaces the failure, which is why it is not tracked as state.
  useEffect(() => {
    let active = true;

    async function loadPricingCatalog() {
      if (!providerId || providerAccessState !== "ready") {
        if (active) {
          setPricingByRegion({});
        }
        return;
      }

      try {
        const catalog = await loadProviderPricingCatalog(providerId);
        if (active) {
          setPricingByRegion(catalog.pricingByRegion);
        }
      } catch {
        if (active) {
          setPricingByRegion({});
        }
      }
    }

    void loadPricingCatalog();
    return () => {
      active = false;
    };
  }, [providerAccessState, providerId]);

  // One derived status per node, feeding both the badge and the filter — they used to read
  // different fields, so filtering "ready" returned rows that all rendered "ACTIVE".
  const allRows = useMemo<NodeRow[]>(() => nodes.map((node) => ({
    node,
    status: deriveNodeStatus({
      node,
      isCurrent: node.id === currentNodeId,
      // Only the node holding the live tunnel counts as connected.
      isLiveConnected: tunnelConnected && node.lifecycle_status.toLowerCase() === "connected",
      // Sourced from the one in-flight attempt rather than per-row local state. A row used to flip
      // to "Disconnect" the instant it was clicked, which claimed a working tunnel during the very
      // window where the user has no internet.
      isConnecting: connectAttempt?.nodeId === node.id,
    }),
    location: describeLocation(node.region_geo, node.region),
  })), [connectAttempt, currentNodeId, nodes, tunnelConnected]);

  const rows = useMemo(
    () => selectNodeRows({ rows: allRows, statusFilter, regionFilter, sort }),
    [allRows, regionFilter, sort, statusFilter],
  );

  const statusOptions = useMemo(() => {
    const present = new Set(allRows.map((row) => row.status));
    return NODE_STATUS_ORDER.filter((status) => present.has(status));
  }, [allRows]);

  const regionOptions = useMemo(
    () => buildRegionOptions(nodes.map((node) => ({ region: node.region_geo, slug: node.region }))),
    [nodes],
  );

  const duplicatedRegionSlugs = useMemo(() => duplicatedRegions(nodes), [nodes]);
  const selectedNodePricing = selectedNode ? getNodePricingDisplay(selectedNode, pricingByRegion) : null;
  const selectedNodeTunnel = useMemo(() => {
    const liveTunnel = selectedNodeId
      ? (tunnels.find((tunnel) => tunnel.node_id === selectedNodeId) ?? null)
      : null;

    if (liveTunnel) {
      return liveTunnel;
    }

    return (diagnostics?.tunnels ?? []).find((tunnel) => tunnel.node_id === selectedNodeId) ?? null;
  }, [diagnostics?.tunnels, selectedNodeId, tunnels]);
  const selectedNodeActionMessage = selectedNode
    ? selectedNode.lifecycle_status === "error"
      // A node in Error cannot transition back to Ready, and creating again with the same id is
      // refused while the row exists — so cleanup is not optional advice, it is the only route.
      ? `${selectedNode.error_reason ?? "Provisioning failed."} Force cleanup removes this record and any partial provider resources; the same node ID cannot be reused until it does.`
      : selectedNode.lifecycle_status === "provisioning"
        ? "Provisioning is in progress. Avoid submitting another create request until this record moves to Ready or Error."
        : selectedNode.lifecycle_status === "bootstrapping"
          ? "The server is booting and starting WireGuard. This record moves to Ready once it is up."
          : "This server is provisioned and ready to connect."
    : "Select a node to inspect health, pricing, and recovery options.";

  function handleNavigateToCreate() {
    router.push(NODE_CREATE_ROUTE);
  }

  function handleOpenNodeDetails(nodeId: string) {
    setSelectedNodeId(nodeId);
    setIsDetailsModalOpen(true);
  }

  function handleCloseNodeDetails() {
    setIsDetailsModalOpen(false);
  }

  return (
    <PageChrome>
      <SavedLocationsView
        locations={savedLocations}
        busyAction={busyAction}
        onTurnOn={turnOnLocation}
        onTurnOff={turnOffLocation}
        onForget={forgetLocation}
      />
      <NodesView
        providerAccessState={providerAccessState}
        providers={providers}
        nodes={nodes}
        rows={rows}
        diagnostics={diagnostics}
        pricingByRegion={pricingByRegion}
        statusFilter={statusFilter}
        setStatusFilter={setStatusFilter}
        regionFilter={regionFilter}
        setRegionFilter={setRegionFilter}
        sort={sort}
        setSort={setSort}
        statusOptions={statusOptions}
        regionOptions={regionOptions}
        duplicatedRegionSlugs={duplicatedRegionSlugs}
        currentNodeId={currentNodeId}
        selectedNodeId={selectedNodeId}
        selectedNode={selectedNode}
        health={health}
        selectedNodePricing={selectedNodePricing}
        selectedNodeActionMessage={selectedNodeActionMessage}
        selectedNodeTunnel={selectedNodeTunnel}
        busyAction={busyAction}
        onNavigateToCreate={handleNavigateToCreate}
        onInstallHost={installHostForNode}
        onDestroyNode={destroyNode}
        onSetCurrentNode={setCurrentNode}
        onConnectNode={connectNode}
        onConnectTunnel={connectTunnel}
        onDisconnectTunnel={disconnectTunnel}
        onForceDisconnectNodeService={forceDisconnectNodeService}
        nodePeers={nodePeers}
        onClaimDevice={claimDevice}
        onLoadDeviceConfig={loadDeviceConfig}
        onExportDeviceConfig={exportDeviceConfig}
        onCopyText={copyText}
        onForceCleanupSelectedNode={forceCleanupSelectedNode}
        isDetailsModalOpen={isDetailsModalOpen}
        onOpenNodeDetails={handleOpenNodeDetails}
        onCloseNodeDetails={handleCloseNodeDetails}
      />
    </PageChrome>
  );
}
