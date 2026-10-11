"use client";

import { NodesCreateView } from "@/components/dashboard/nodes-create-view";
import { useDashboard } from "@/components/dashboard/dashboard-provider";
import {
  planDisplayFromPricing,
  providerAccessBadge,
  VIEW_ROUTE,
  type ProviderPlanPricing,
} from "@/components/dashboard/shared";
import { loadProviderPricingCatalog } from "@/lib/provider-pricing-cache";
import type { ProviderRegion } from "@/lib/tauri";
import { useRouter } from "next/navigation";
import { useEffect, useRef, useState } from "react";

export default function DashboardNodesCreatePage() {
  const router = useRouter();
  const {
    busyAction,
    configuredProvider,
    createNode,
    providerAccessState,
    providerId,
    providerName,
    providerStatusMessage,
    providers,
    selectProviderId,
  } = useDashboard();

  // A provision runs for minutes and the user may wander off meanwhile; only a user still on this
  // page is sent back to the list when it finishes.
  const mountedRef = useRef(true);
  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  const [nodeId, setNodeId] = useState("node-1");
  const [nodeName, setNodeName] = useState("Primary Node");
  const [region, setRegion] = useState("");
  const [planKey, setPlanKey] = useState("");
  const [availableRegions, setAvailableRegions] = useState<ProviderRegion[]>([]);
  const [pricingByRegion, setPricingByRegion] = useState<Record<string, ProviderPlanPricing[]>>({});
  const [pricingBusy, setPricingBusy] = useState(false);
  const [pricingError, setPricingError] = useState<string | null>(null);
  const isProviderSuspended = configuredProvider?.status === "suspended";
  const suspendedMessage = configuredProvider
    ? `Provider '${configuredProvider.id}' is suspended. Resume the provider from Setup before running this operation.`
    : "Selected provider is suspended. Resume the provider from Setup before running this operation.";
  const effectiveProviderStatusMessage = isProviderSuspended
    ? suspendedMessage
    : providerStatusMessage;
  const nodesProvisioningDisabled = providerAccessState !== "ready" || isProviderSuspended;

  const providerBadge = providerAccessBadge(providerAccessState);

  useEffect(() => {
    let active = true;

    async function loadPricingCatalog() {
      const canLoadDockerPlanningCatalog = configuredProvider?.slug === "docker" && !!configuredProvider.token_ref;
      const canLoadPricingCatalog = !!providerId
        && !isProviderSuspended
        && (providerAccessState === "ready" || canLoadDockerPlanningCatalog);

      if (!canLoadPricingCatalog) {
        if (active) {
          setAvailableRegions([]);
          setPricingByRegion({});
          setRegion("");
          setPlanKey("");
          setPricingError(
            providerAccessState === "missing" || providerAccessState === "invalid" || isProviderSuspended
              ? effectiveProviderStatusMessage
              : null,
          );
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
          setAvailableRegions([]);
          setPricingByRegion({});
          setRegion("");
          setPlanKey("");
          setPricingError("Provider returned no available regions for pricing lookup.");
          return;
        }

        const nextPricingByRegion = catalog.pricingByRegion;
        // State stays keyed by slug; the region records only supply display names.
        const nextRegion = catalog.regions.some((entry) => entry.slug === region)
          ? region
          : catalog.regions[0].slug;
        const nextRegionPlans = nextPricingByRegion[nextRegion] ?? [];
        const hasCurrentPlan = nextRegionPlans.some((plan) => plan.plan_key === planKey);
        const nextPlanKey = hasCurrentPlan ? planKey : (nextRegionPlans[0]?.plan_key ?? "");

        setAvailableRegions(catalog.regions);
        setPricingByRegion(nextPricingByRegion);
        setRegion(nextRegion);
        setPlanKey(nextPlanKey);
        setPricingError(providerAccessState === "ready" && !isProviderSuspended ? null : effectiveProviderStatusMessage);
      } catch (error) {
        if (!active) {
          return;
        }
        setAvailableRegions([]);
        setPricingByRegion({});
        setRegion("");
        setPlanKey("");
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
  }, [configuredProvider, providerAccessState, providerId, effectiveProviderStatusMessage, isProviderSuspended, region, planKey]);

  const selectedRegionPricing = pricingByRegion[region] ?? [];
  const selectedCreatePlan = selectedRegionPricing.find((plan) => plan.plan_key === planKey) ?? null;
  const selectedCreatePlanDisplay = selectedCreatePlan ? planDisplayFromPricing(selectedCreatePlan) : null;

  async function handleCreateNode() {
    if (!region || !planKey) {
      return;
    }
    const created = await createNode({ nodeId, nodeName, region, planKey });
    if (created && mountedRef.current) {
      router.push(VIEW_ROUTE.map);
    }
  }

  function handleNavigateToNodes() {
    router.push(VIEW_ROUTE.map);
  }

  function handleNavigateToSettings() {
    router.push("/dashboard/settings");
  }

  return (
    <NodesCreateView
      providerAccessState={providerAccessState}
      providerBadge={providerBadge}
      providerStatusMessage={effectiveProviderStatusMessage}
      providerId={providerId}
      providerName={providerName}
      configuredProvider={configuredProvider}
      providers={providers}
      onSelectProvider={selectProviderId}
      onCreateNode={handleCreateNode}
      onNavigateToNodes={handleNavigateToNodes}
      onNavigateToSettings={handleNavigateToSettings}
      busyAction={busyAction}
      region={region}
      setRegion={setRegion}
      planKey={planKey}
      nodeId={nodeId}
      setPlanKey={setPlanKey}
      setNodeId={setNodeId}
      setNodeName={setNodeName}
      nodeName={nodeName}
      nodeFeedback={pricingError ?? (nodesProvisioningDisabled ? effectiveProviderStatusMessage : "No node action yet.")}
      nodesProvisioningDisabled={nodesProvisioningDisabled}
      availableRegions={availableRegions}
      pricingCatalog={selectedRegionPricing}
      selectedCreatePlanDisplay={selectedCreatePlanDisplay}
      pricingBusy={pricingBusy}
      pricingError={pricingError}
    />
  );
}
