"use client";

import { NodesView } from "@/components/dashboard/nodes-view";
import { useDashboard } from "@/components/dashboard/dashboard-provider";
import {
  DEFAULT_NODE_REGION,
  DEFAULT_PLAN_KEY,
  getNodePricingDisplay,
  planDisplayFromPricing,
  providerAccessBadge,
  type NodeSort,
  VIEW_ROUTE,
} from "@/components/dashboard/shared";
import { listProviderPricing, type ProviderPlanPricing } from "@/lib/tauri";
import { useRouter } from "next/navigation";
import { useEffect, useMemo, useState } from "react";

export default function DashboardNodesPage() {
  const router = useRouter();
  const {
    busyAction,
    configuredProvider,
    createNode,
    diagnostics,
    destroyNode,
    forceCleanupSelectedNode,
    health,
    nodes,
    providerAccessState,
    providerId,
    providerName,
    providerStatusMessage,
    selectedNode,
    selectedNodeId,
    setSelectedNodeId,
    connectTunnel,
    disconnectTunnel,
  } = useDashboard();

  const [nodeId, setNodeId] = useState("node-1");
  const [nodeName, setNodeName] = useState("Primary Node");
  const [region, setRegion] = useState(DEFAULT_NODE_REGION);
  const [planKey, setPlanKey] = useState(DEFAULT_PLAN_KEY);
  const [nodeStatusFilter, setNodeStatusFilter] = useState<string>("all");
  const [nodeRegionFilter, setNodeRegionFilter] = useState<string>("all");
  const [nodeSort, setNodeSort] = useState<NodeSort>("newest");
  const [pricingByRegion, setPricingByRegion] = useState<Record<string, ProviderPlanPricing[]>>({});
  const [pricingBusy, setPricingBusy] = useState(false);
  const [pricingError, setPricingError] = useState<string | null>(null);

  const providerBadge = providerAccessBadge(providerAccessState);
  const nodesProvisioningDisabled = providerAccessState !== "ready";

  useEffect(() => {
    async function loadPricingCatalog() {
      if (!providerId || providerAccessState !== "ready") {
        setPricingByRegion({});
        return;
      }

      const regions = Array.from(new Set([region, ...nodes.map((node) => node.region)].filter(Boolean)));
      if (regions.length === 0) {
        setPricingByRegion({});
        setPricingError(null);
        return;
      }

      setPricingBusy(true);
      try {
        const pricingEntries = await Promise.all(
          regions.map(async (regionValue) => {
            const response = await listProviderPricing(providerId, regionValue);
            return [regionValue, response.plans] as const;
          }),
        );
        setPricingByRegion(Object.fromEntries(pricingEntries));
        setPricingError(null);
      } catch (error) {
        const message = error instanceof Error ? error.message : String(error);
        setPricingError(message);
      } finally {
        setPricingBusy(false);
      }
    }

    void loadPricingCatalog();
  }, [nodes, providerAccessState, providerId, region]);

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
  const selectedRegionPricing = pricingByRegion[region] ?? [];
  const selectedCreatePlan = selectedRegionPricing.find((plan) => plan.plan_key === planKey) ?? null;
  const selectedCreatePlanDisplay = selectedCreatePlan ? planDisplayFromPricing(selectedCreatePlan) : null;
  const selectedNodePricing = selectedNode ? getNodePricingDisplay(selectedNode, pricingByRegion) : null;
  const selectedNodeTunnel = (diagnostics?.tunnels ?? []).find((tunnel) => tunnel.node_id === selectedNodeId) ?? null;
  const selectedNodeActionMessage = selectedNode
    ? selectedNode.lifecycle_status === "error"
      ? `${selectedNode.error_reason ?? "Provisioning failed."} Next: retry validation, recreate the node, or use force cleanup if partial resources remain.`
      : selectedNode.lifecycle_status === "provisioning"
        ? "Provisioning is in progress. Avoid submitting another create request until this record moves to Ready or Error."
        : selectedNode.lifecycle_status === "bootstrapping"
          ? "Bootstrap is still running. Keep this page open or refresh to follow the progress updates."
          : "Use View Details to inspect endpoint health, pricing snapshot, and recovery actions for this node."
    : "Select a node to inspect health, pricing, and recovery options.";

  async function handleCreateNode() {
    await createNode({ nodeId, nodeName, region, planKey });
  }

  function handleNavigateToView(view: keyof typeof VIEW_ROUTE) {
    router.push(VIEW_ROUTE[view]);
  }

  return (
    <NodesView
      providerAccessState={providerAccessState}
      providerBadge={providerBadge}
      providerStatusMessage={providerStatusMessage}
      providerName={providerName}
      configuredProvider={configuredProvider}
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
      onCreateNode={handleCreateNode}
      onConnectTunnel={connectTunnel}
      onDisconnectTunnel={disconnectTunnel}
      onDestroyNode={destroyNode}
      onForceCleanupSelectedNode={forceCleanupSelectedNode}
      onNavigateToView={handleNavigateToView}
      setRegion={setRegion}
      region={region}
      setPlanKey={setPlanKey}
      planKey={planKey}
      setNodeId={setNodeId}
      nodeId={nodeId}
      setNodeName={setNodeName}
      nodeName={nodeName}
      nodeFeedback={pricingError ?? "No node action yet."}
      providerId={providerId}
      nodesProvisioningDisabled={nodesProvisioningDisabled}
      nodeRegions={nodeRegions}
      nodeStatuses={nodeStatuses}
      pricingCatalog={selectedRegionPricing}
      setPricingByRegion={setPricingByRegion}
      selectedCreatePlanDisplay={selectedCreatePlanDisplay}
      selectedCreatePlan={selectedCreatePlan}
      providerAccessStateValue={providerAccessState}
    />
  );
}
