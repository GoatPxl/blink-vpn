"use client";

import { NodesCreateView } from "@/components/dashboard/nodes-create-view";
import { useDashboard } from "@/components/dashboard/dashboard-provider";
import {
  planDisplayFromPricing,
  providerAccessBadge,
  type ProviderPlanPricing,
} from "@/components/dashboard/shared";
import { loadProviderPricingCatalog } from "@/lib/provider-pricing-cache";
import { useRouter } from "next/navigation";
import { useEffect, useState } from "react";

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

  const [nodeId, setNodeId] = useState("node-1");
  const [nodeName, setNodeName] = useState("Primary Node");
  const [region, setRegion] = useState("");
  const [planKey, setPlanKey] = useState("");
  const [availableRegions, setAvailableRegions] = useState<string[]>([]);
  const [pricingByRegion, setPricingByRegion] = useState<Record<string, ProviderPlanPricing[]>>({});
  const [pricingBusy, setPricingBusy] = useState(false);
  const [pricingError, setPricingError] = useState<string | null>(null);

  const providerBadge = providerAccessBadge(providerAccessState);

  useEffect(() => {
    let active = true;

    async function loadPricingCatalog() {
      const canLoadDockerPlanningCatalog = configuredProvider?.slug === "docker" && !!configuredProvider.token_ref;
      const canLoadPricingCatalog = !!providerId && (providerAccessState === "ready" || canLoadDockerPlanningCatalog);

      if (!canLoadPricingCatalog) {
        if (active) {
          setAvailableRegions([]);
          setPricingByRegion({});
          setRegion("");
          setPlanKey("");
          setPricingError(
            providerAccessState === "missing" || providerAccessState === "invalid"
              ? providerStatusMessage
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
        const nextRegion = catalog.regions.includes(region) ? region : catalog.regions[0];
        const nextRegionPlans = nextPricingByRegion[nextRegion] ?? [];
        const hasCurrentPlan = nextRegionPlans.some((plan) => plan.plan_key === planKey);
        const nextPlanKey = hasCurrentPlan ? planKey : (nextRegionPlans[0]?.plan_key ?? "");

        setAvailableRegions(catalog.regions);
        setPricingByRegion(nextPricingByRegion);
        setRegion(nextRegion);
        setPlanKey(nextPlanKey);
        setPricingError(providerAccessState === "ready" ? null : providerStatusMessage);
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
  }, [configuredProvider, providerAccessState, providerId, providerStatusMessage, region, planKey]);

  const selectedRegionPricing = pricingByRegion[region] ?? [];
  const selectedCreatePlan = selectedRegionPricing.find((plan) => plan.plan_key === planKey) ?? null;
  const selectedCreatePlanDisplay = selectedCreatePlan ? planDisplayFromPricing(selectedCreatePlan) : null;

  async function handleCreateNode() {
    if (!region || !planKey) {
      return;
    }
    await createNode({ nodeId, nodeName, region, planKey });
  }

  function handleNavigateToNodes() {
    router.push("/dashboard/nodes");
  }

  function handleNavigateToSettings() {
    router.push("/dashboard/settings");
  }

  return (
    <NodesCreateView
      providerAccessState={providerAccessState}
      providerBadge={providerBadge}
      providerStatusMessage={providerStatusMessage}
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
      nodeFeedback={pricingError ?? (providerAccessState !== "ready" ? providerStatusMessage : "No node action yet.")}
      nodesProvisioningDisabled={providerAccessState !== "ready"}
      availableRegions={availableRegions}
      pricingCatalog={selectedRegionPricing}
      selectedCreatePlanDisplay={selectedCreatePlanDisplay}
      pricingBusy={pricingBusy}
      pricingError={pricingError}
    />
  );
}
