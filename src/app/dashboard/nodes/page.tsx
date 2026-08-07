"use client";

import { NodesView } from "@/components/dashboard/nodes-view";
import { useDashboard } from "@/components/dashboard/dashboard-provider";
import {
  getNodePricingDisplay,
  providerAccessBadge,
  type NodeSort,
} from "@/components/dashboard/shared";
import { loadProviderPricingCatalog } from "@/lib/provider-pricing-cache";
import { type ProviderPlanPricing } from "@/lib/tauri";
import { useRouter } from "next/navigation";
import { useEffect, useMemo, useState } from "react";

export default function DashboardNodesPage() {
  const router = useRouter();
  const {
    busyAction,
    configuredProvider,
    diagnostics,
    forceCleanupSelectedNode,
    health,
    nodes,
    providerAccessState,
    providerId,
    providerName,
    providers,
    selectProviderId,
    selectedNode,
    selectedNodeId,
    setSelectedNodeId,
    installHostForNode,
    connectNode,
    uninstallHostForNode,
    connectTunnel,
    disconnectTunnel,
    forceDisconnectNodeService,
    tunnels,
  } = useDashboard();
  const [nodeStatusFilter, setNodeStatusFilter] = useState<string>("all");
  const [nodeRegionFilter, setNodeRegionFilter] = useState<string>("all");
  const [nodeSort, setNodeSort] = useState<NodeSort>("newest");
  const [pricingByRegion, setPricingByRegion] = useState<Record<string, ProviderPlanPricing[]>>({});
  const [pricingBusy, setPricingBusy] = useState(false);
  const [pricingError, setPricingError] = useState<string | null>(null);
  const [isDetailsModalOpen, setIsDetailsModalOpen] = useState(false);

  const providerBadge = providerAccessBadge(providerAccessState);

  useEffect(() => {
    let active = true;

    async function loadPricingCatalog() {
      if (!providerId || providerAccessState !== "ready") {
        if (active) {
          setPricingByRegion({});
        }
        return;
      }

      if (active) {
        setPricingBusy(true);
      }
      try {
        const catalog = await loadProviderPricingCatalog(providerId);
        if (!active) {
          return;
        }

        if (catalog.regions.length === 0) {
          setPricingByRegion({});
          setPricingError("Provider returned no available regions for pricing lookup.");
          return;
        }

        const nextPricingByRegion = catalog.pricingByRegion;

        setPricingByRegion(nextPricingByRegion);
        setPricingError(null);
      } catch (error) {
        if (!active) {
          return;
        }
        setPricingByRegion({});
        const message = error instanceof Error ? error.message : String(error);
        setPricingError(message);
      } finally {
        if (active) {
          setPricingBusy(false);
        }
      }
    }

    void loadPricingCatalog();
    return () => {
      active = false;
    };
  }, [providerAccessState, providerId]);

  const filteredNodes = useMemo(() => {
    const filtered = nodes.filter((node) => {
      const statusMatches = nodeStatusFilter === "all" || node.lifecycle_status === nodeStatusFilter;
      const regionMatches = nodeRegionFilter === "all" || node.region === nodeRegionFilter;
      return statusMatches && regionMatches;
    });

    filtered.sort((left, right) => {
      const leftValue = Date.parse(left.created_at || left.updated_at || "0");
      const rightValue = Date.parse(right.created_at || right.updated_at || "0");
      return nodeSort === "newest" ? rightValue - leftValue : leftValue - rightValue;
    });

    return filtered;
  }, [nodeRegionFilter, nodeSort, nodeStatusFilter, nodes]);

  const nodeRegions = useMemo(() => Array.from(new Set(nodes.map((node) => node.region).filter(Boolean))).sort(), [nodes]);
  const nodeStatuses = useMemo(() => Array.from(new Set(nodes.map((node) => node.lifecycle_status))).sort(), [nodes]);
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
      ? `${selectedNode.error_reason ?? "Provisioning failed."} Next: retry validation, recreate the node, or use force cleanup if partial resources remain.`
      : selectedNode.lifecycle_status === "provisioning"
        ? "Provisioning is in progress. Avoid submitting another create request until this record moves to Ready or Error."
        : selectedNode.lifecycle_status === "bootstrapping"
          ? "Bootstrap is still running. Keep this page open or refresh to follow the progress updates."
          : "Use View Details to inspect endpoint health, pricing snapshot, and recovery actions for this node."
    : "Select a node to inspect health, pricing, and recovery options.";

  function handleNavigateToCreate() {
    router.push("/dashboard/nodes/create");
  }

  function handleOpenNodeDetails(nodeId: string) {
    setSelectedNodeId(nodeId);
    setIsDetailsModalOpen(true);
  }

  function handleCloseNodeDetails() {
    setIsDetailsModalOpen(false);
  }

  return (
    <NodesView
      providerAccessState={providerAccessState}
      providerBadge={providerBadge}
      providerId={providerId}
      providerName={providerName}
      configuredProvider={configuredProvider}
      providers={providers}
      onSelectProvider={selectProviderId}
      nodes={filteredNodes}
      diagnostics={diagnostics}
      pricingByRegion={pricingByRegion}
      pricingBusy={pricingBusy}
      pricingError={pricingError}
      nodeStatusFilter={nodeStatusFilter}
      setNodeStatusFilter={setNodeStatusFilter}
      nodeRegionFilter={nodeRegionFilter}
      setNodeRegionFilter={setNodeRegionFilter}
      nodeSort={nodeSort}
      setNodeSort={setNodeSort}
      selectedNodeId={selectedNodeId}
      setSelectedNodeId={setSelectedNodeId}
      selectedNode={selectedNode}
      health={health}
      selectedNodePricing={selectedNodePricing}
      selectedNodeActionMessage={selectedNodeActionMessage}
      selectedNodeTunnel={selectedNodeTunnel}
      busyAction={busyAction}
      onCreateNode={() => Promise.resolve()}
      onNavigateToCreate={handleNavigateToCreate}
      onInstallHost={installHostForNode}
      onUninstallHost={uninstallHostForNode}
      onConnectNode={connectNode}
      onConnectTunnel={connectTunnel}
      onDisconnectTunnel={disconnectTunnel}
      onForceDisconnectNodeService={forceDisconnectNodeService}
      onForceCleanupSelectedNode={forceCleanupSelectedNode}
      nodeFeedback={pricingError ?? "No node action yet."}
      nodeRegions={nodeRegions}
      nodeStatuses={nodeStatuses}
      isDetailsModalOpen={isDetailsModalOpen}
      onOpenNodeDetails={handleOpenNodeDetails}
      onCloseNodeDetails={handleCloseNodeDetails}
    />
  );
}
