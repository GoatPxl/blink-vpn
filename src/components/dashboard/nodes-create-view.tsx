"use client";

import { Button, Card, Field, PageChrome, type DashboardBusyAction, type PlanDisplayModel, type ProviderAccessState, type ProviderPlanPricing, type ProviderRecord } from "./shared";
import { formatCurrency, formatMemory, formatRegionLabel } from "./shared";

export function NodesCreateView({
  providerAccessState,
  providerBadge,
  providerStatusMessage,
  providerId,
  providerName,
  configuredProvider,
  providers,
  onSelectProvider,
  onCreateNode,
  onNavigateToNodes,
  onNavigateToSettings,
  busyAction,
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
  availableRegions,
  pricingCatalog,
  selectedCreatePlanDisplay,
  pricingBusy,
  pricingError,
}: {
  providerAccessState: ProviderAccessState;
  providerBadge: { label: string; classes: string };
  providerStatusMessage: string;
  providerId: string;
  providerName: string;
  configuredProvider: ProviderRecord | null;
  providers: ProviderRecord[];
  onSelectProvider: (providerId: string) => void;
  onCreateNode: () => Promise<void>;
  onNavigateToNodes: () => void;
  onNavigateToSettings: () => void;
  busyAction: DashboardBusyAction;
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
  availableRegions: string[];
  pricingCatalog: ProviderPlanPricing[];
  selectedCreatePlanDisplay: PlanDisplayModel | null;
  pricingBusy: boolean;
  pricingError: string | null;
}) {
  return (
    <PageChrome>
      <div className="space-y-4">
        <div className="flex flex-col gap-4 rounded-[24px] border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.02)] p-5 shadow-[var(--shadow-elevated)] lg:flex-row lg:items-end lg:justify-between">
          <div>
            <h2 className="text-[28px] font-semibold tracking-tight text-[var(--on-surface)]">Create Node</h2>
            <p className="mt-1 max-w-3xl text-sm text-[var(--on-surface-variant)]">Provision a new node with the currently selected provider token and live provider size catalog for the selected integration.</p>
          </div>
          <div className="flex flex-wrap items-center gap-2">
            <span className={`inline-flex rounded-full border px-3 py-1.5 text-xs ${providerBadge.classes}`}>Provider access: {providerBadge.label}</span>
            <Button tone="secondary" onClick={onNavigateToNodes}>Back to Nodes</Button>
          </div>
        </div>

        {(providerAccessState === "missing" || providerAccessState === "invalid") ? (
          <div className="flex flex-col gap-3 rounded-[20px] border border-[color:rgba(255,185,95,0.28)] bg-[color:rgba(255,185,95,0.08)] p-4 text-sm text-[var(--tertiary)] lg:flex-row lg:items-center lg:justify-between">
            <div>
              <div className="font-medium text-[var(--on-surface)]">Provisioning is disabled until provider access is healthy.</div>
              <div className="mt-1 text-[var(--on-surface-variant)]">{providerStatusMessage}</div>
            </div>
            <Button tone="secondary" onClick={onNavigateToSettings}>Open Settings</Button>
          </div>
        ) : null}

        <Card title="Create Node" description="Provision a new node with the currently selected provider token.">
          <div className="space-y-5">
            <div className="rounded-[18px] border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.02)] p-4 text-sm">
              <div className="text-[11px] uppercase tracking-[0.24em] text-[var(--on-surface-variant)]">Provider</div>
              <div className="mt-2 space-y-2">
                <select
                  value={providerId}
                  onChange={(event) => onSelectProvider(event.target.value)}
                  className="w-full rounded-xl border border-[color:var(--outline-variant)] bg-[color:rgba(8,10,16,0.9)] px-3 py-2.5 text-[var(--on-surface)] outline-none ring-0"
                >
                  {providers.map((provider) => (
                    <option key={provider.id} value={provider.id}>
                      {provider.name} ({provider.slug})
                    </option>
                  ))}
                </select>
                <div className="font-medium text-[var(--on-surface)]">{configuredProvider?.name ?? providerName}</div>
                <div className="text-[var(--on-surface-variant)]">Provider and size catalog are loaded dynamically from the selected integration.</div>
              </div>
              <div className="mt-3 flex items-center justify-end">
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
              <Button tone="secondary" onClick={onNavigateToNodes}>Back to Nodes</Button>
              <Button onClick={() => void onCreateNode()} disabled={busyAction !== null || nodesProvisioningDisabled || !nodeId.trim() || !nodeName.trim() || !region || !planKey || pricingCatalog.length === 0}>{busyAction === "create-node" ? "Provisioning..." : "Create Node"}</Button>
            </div>
          </div>
        </Card>
      </div>
    </PageChrome>
  );
}
