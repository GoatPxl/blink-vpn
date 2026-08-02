"use client";

import { useEffect, useMemo, useState } from "react";
import { useRouter } from "next/navigation";
import {
  createVpnNode,
  installWireGuardWithPrerequisites,
  listProviders,
  listProviderPricing,
  listProviderRegions,
  listVpnNodes,
  registerProviderToken,
  validateProviderToken,
  type ProviderPlanPricing,
  type WireGuardInstallResult,
  type VpnNodeRecord,
  type WireGuardReadiness,
} from "@/lib/tauri";
import {
  DEFAULT_ONBOARDING_STATE,
  loadOnboardingState,
  saveOnboardingState,
  type OnboardingState,
} from "@/lib/onboarding-state";
import { runWireGuardReadinessCheck } from "@/lib/wireguard-readiness";

function stepTitle(step: number): string {
  if (step === 1) return "WireGuard Readiness";
  if (step === 2) return "DigitalOcean Token";
  if (step === 3) return "Server Configuration";
  if (step === 4) return "Provision Server";
  return "Ready";
}

export function OnboardingWizard() {
  const router = useRouter();
  const [state, setState] = useState<OnboardingState>(DEFAULT_ONBOARDING_STATE);
  const [step, setStep] = useState(1);
  const [isHydrated, setIsHydrated] = useState(false);

  const [wireGuardReadiness, setWireGuardReadiness] = useState<WireGuardReadiness | null>(null);
  const [wireGuardBusy, setWireGuardBusy] = useState(false);
  const [wireGuardInstallBusy, setWireGuardInstallBusy] = useState(false);
  const [wireGuardInstallMessage, setWireGuardInstallMessage] = useState("");

  const [apiToken, setApiToken] = useState("");
  const [tokenMessage, setTokenMessage] = useState("No token action yet.");
  const [tokenBusy, setTokenBusy] = useState(false);
  const [storedProviderTokenRef, setStoredProviderTokenRef] = useState<string | null>(null);

  const [pricingBusy, setPricingBusy] = useState(false);
  const [pricingError, setPricingError] = useState<string | null>(null);
  const [availableRegions, setAvailableRegions] = useState<string[]>([]);
  const [livePricing, setLivePricing] = useState<ProviderPlanPricing[]>([]);

  const [provisionBusy, setProvisionBusy] = useState(false);
  const [provisionPhase, setProvisionPhase] = useState<"idle" | "provisioning" | "bootstrapping" | "ready" | "error">("idle");
  const [provisionMessage, setProvisionMessage] = useState("Not started.");
  const [provisionNode, setProvisionNode] = useState<VpnNodeRecord | null>(null);

  const [finalCheckBusy, setFinalCheckBusy] = useState(false);

  useEffect(() => {
    const loaded = loadOnboardingState();
    // eslint-disable-next-line react-hooks/set-state-in-effect
    setState(loaded);
    setStep(Math.min(Math.max(loaded.currentStep, 1), 5));
    setIsHydrated(true);
  }, []);

  useEffect(() => {
    if (!isHydrated) return;
    saveOnboardingState({ ...state, currentStep: step, status: step >= 5 ? state.status : "in_progress" });
  }, [isHydrated, state, step]);

  useEffect(() => {
    if (!isHydrated) return;
    if (step === 1 && wireGuardReadiness === null) {
      void checkWireGuard();
    }
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [isHydrated, step]);

  useEffect(() => {
    if (!isHydrated) return;
    if (step < 2) return;

    const timeoutId = window.setTimeout(() => {
      void syncStoredProviderTokenState(true);
    }, 0);

    return () => window.clearTimeout(timeoutId);
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [isHydrated, state.providerId, step]);

  useEffect(() => {
    if (!isHydrated) return;
    if (step !== 3) return;
    if (!state.tokenSaved && !state.tokenValidated) return;
    const timeoutId = window.setTimeout(() => {
      void loadLivePricing();
    }, 0);
    return () => window.clearTimeout(timeoutId);
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [isHydrated, step, state.region, state.providerId, state.tokenSaved, state.tokenValidated]);

  const selectedPlanPricing = useMemo(() => {
    return livePricing.find((entry) => entry.plan_key === state.plan) ?? null;
  }, [livePricing, state.plan]);

  const hasUnsavedTokenInput = apiToken.trim().length > 0;
  const hasStoredProviderToken = !!storedProviderTokenRef;

  async function syncStoredProviderTokenState(validateStoredToken = false) {
    try {
      const providers = await listProviders();
      const currentProvider = providers.find((provider) => provider.id === state.providerId) ?? null;
      const tokenRef = currentProvider?.token_ref ?? null;

      setStoredProviderTokenRef(tokenRef);

      if (!tokenRef) {
        return;
      }

      setState((current) => ({
        ...current,
        tokenSaved: true,
      }));

      setTokenMessage((current) => (
        current === "No token action yet."
          ? "Stored provider token found. Click Validate Token to continue, or enter a new token to replace it."
          : current
      ));

      if (!validateStoredToken || hasUnsavedTokenInput) {
        return;
      }

      const validateResult = await validateProviderToken(state.providerId);
      setState((current) => ({
        ...current,
        tokenSaved: true,
        tokenValidated: true,
      }));
      setTokenMessage(`Stored provider token found. ${validateResult}`);
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      const needsResave = /no stored secret|missing token_ref|has no token_ref|initialized with a token|stale/i.test(message);

      if (needsResave) {
        setStoredProviderTokenRef(null);
        setState((current) => ({
          ...current,
          tokenSaved: false,
          tokenValidated: false,
        }));
        setTokenMessage((current) => (
          current === "No token action yet." || current.startsWith("Stored provider token")
            ? "Stored provider token reference is stale or missing. Click Save Token to retry secure registration."
            : current
        ));
        return;
      }

      if (validateStoredToken) {
        setState((current) => ({
          ...current,
          tokenSaved: true,
          tokenValidated: false,
        }));
        setTokenMessage(message);
      }
    }
  }

  async function loadLivePricing() {
    setPricingBusy(true);
    setPricingError(null);
    try {
      const regions = await listProviderRegions(state.providerId);
      setAvailableRegions(regions);
      if (regions.length === 0) {
        setLivePricing([]);
        setState((current) => ({ ...current, region: "", plan: "" }));
        setPricingError("No available regions were returned by the provider API.");
        return;
      }

      const selectedRegion = regions.includes(state.region) ? state.region : regions[0];
      const response = await listProviderPricing(state.providerId, selectedRegion);
      setLivePricing(response.plans);
      const hasSelectedPlan = response.plans.some((entry) => entry.plan_key === state.plan);
      const nextPlan = hasSelectedPlan ? state.plan : (response.plans[0]?.plan_key ?? "");
      setState((current) => ({
        ...current,
        region: selectedRegion,
        plan: nextPlan,
      }));
      if (response.plans.length === 0) {
        setPricingError("No supported plan pricing is currently available for the selected region.");
      }
    } catch (error) {
      setAvailableRegions([]);
      setLivePricing([]);
      const message = error instanceof Error ? error.message : String(error);
      const isTokenRecoveryNeeded = /no stored secret|missing token_ref|has no token_ref|initialized with a token|stale/i.test(message);
      if (isTokenRecoveryNeeded) {
        setStoredProviderTokenRef(null);
        setState((current) => ({
          ...current,
          tokenSaved: false,
          tokenValidated: false,
        }));
        setTokenMessage("Provider token is no longer valid in secure storage. Save Token, then Validate Token to continue.");
        setStep(2);
      }
      setPricingError(
        isTokenRecoveryNeeded
          ? `Provider token is not currently registered. Return to Step 2, save your DigitalOcean token again, then validate it.`
          : message,
      );
    } finally {
      setPricingBusy(false);
    }
  }

  async function checkWireGuard() {
    setWireGuardBusy(true);
    try {
      const readiness = await runWireGuardReadinessCheck();
      setWireGuardReadiness(readiness);
      setState((current) => ({ ...current, lastWireGuardPassed: readiness.installed && readiness.executable_found && readiness.service_available }));
    } finally {
      setWireGuardBusy(false);
    }
  }

  async function onInstallWireGuardFromApp() {
    setWireGuardInstallMessage("");
    setWireGuardInstallBusy(true);
    try {
      const result: WireGuardInstallResult = await installWireGuardWithPrerequisites();
      setWireGuardInstallMessage(`${result.message} ${result.next_action}`);
      await checkWireGuard();
    } catch (error) {
      setWireGuardInstallMessage(error instanceof Error ? error.message : String(error));
    } finally {
      setWireGuardInstallBusy(false);
    }
  }

  async function onSaveToken() {
    if (!apiToken.trim()) {
      setTokenMessage("Enter a DigitalOcean token to continue.");
      return;
    }

    setTokenBusy(true);
    try {
      const saveResult = await registerProviderToken(state.providerId, state.providerName, apiToken.trim());
      setState((current) => ({ ...current, tokenSaved: true, tokenValidated: false }));

      // Validate immediately after save so Step 2 has a single clear outcome.
      const validateResult = await validateProviderToken(state.providerId);
      await syncStoredProviderTokenState();
      setTokenMessage(`${saveResult} ${validateResult}`);
      setState((current) => ({ ...current, tokenSaved: true, tokenValidated: true }));
      setApiToken("");
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      const needsResave = /no stored secret|missing token_ref|has no token_ref|initialized with a token|stale/i.test(message);
      if (needsResave) {
        setStoredProviderTokenRef(null);
        setState((current) => ({
          ...current,
          tokenSaved: false,
          tokenValidated: false,
        }));
        setTokenMessage("Stored provider token reference is stale or missing. Click Save Token to retry secure registration.");
      } else {
        setState((current) => ({
          ...current,
          tokenValidated: false,
        }));
        setTokenMessage(message);
      }
    } finally {
      setTokenBusy(false);
    }
  }

  async function onValidateToken() {
    if (hasUnsavedTokenInput) {
      setTokenMessage("Click Save + Validate Token to replace the stored token.");
      return;
    }

    if (!state.tokenSaved && !hasStoredProviderToken) {
      setTokenMessage("Save your DigitalOcean token first, then validate it.");
      return;
    }

    setTokenBusy(true);
    try {
      const result = await validateProviderToken(state.providerId);
      await syncStoredProviderTokenState();
      setTokenMessage(result);
      setState((current) => ({ ...current, tokenValidated: true, tokenSaved: true }));
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      const needsResave = /no stored secret|missing token_ref|has no token_ref|initialized with a token|stale/i.test(message);
      if (needsResave) {
        setStoredProviderTokenRef(null);
        setState((current) => ({
          ...current,
          tokenSaved: false,
          tokenValidated: false,
        }));
        setTokenMessage("Stored provider token reference is stale or missing. Click Save Token, then Validate Token.");
      } else {
        setTokenMessage(message);
      }
    } finally {
      setTokenBusy(false);
    }
  }

  async function startProvision() {
    setProvisionBusy(true);
    setProvisionPhase("provisioning");
    setProvisionMessage("Provisioning droplet resources...");

    let pollStopped = false;
    const intervalId = window.setInterval(async () => {
      if (pollStopped) return;
      try {
        const nodes = await listVpnNodes();
        const created = nodes.find((node) => node.id === state.nodeId);
        if (!created) return;

        setProvisionNode(created);

        const status = created.lifecycle_status.toLowerCase();
        if (status === "provisioning") {
          setProvisionPhase("provisioning");
          setProvisionMessage("Provisioning in progress...");
        } else if (status === "bootstrapping") {
          setProvisionPhase("bootstrapping");
          setProvisionMessage("Bootstrapping WireGuard server configuration...");
        } else if (status === "ready" || status === "connected") {
          setProvisionPhase("ready");
          setProvisionMessage("Server is ready.");
        } else if (status === "error") {
          setProvisionPhase("error");
          setProvisionMessage(created.error_reason ?? "Provisioning failed.");
        }
      } catch {
        // Keep polling resiliently; error details are handled by create flow below.
      }
    }, 1800);

    try {
      const result = await createVpnNode({
        nodeId: state.nodeId,
        providerId: state.providerId,
        nodeName: state.nodeName,
        region: state.region,
        planKey: state.plan,
      });

      setProvisionMessage(result);

      const nodes = await listVpnNodes();
      const created = nodes.find((node) => node.id === state.nodeId) ?? null;
      setProvisionNode(created);
      const isReady = created ? (created.lifecycle_status === "ready" || created.lifecycle_status === "connected") : false;
      setProvisionPhase(isReady ? "ready" : "error");

      setState((current) => ({
        ...current,
        nodeReady: isReady,
        lastNodePublicIp: created?.public_ipv4 ?? null,
      }));
    } catch (error) {
      setProvisionPhase("error");
      setProvisionMessage(error instanceof Error ? error.message : String(error));
      setState((current) => ({ ...current, nodeReady: false }));
    } finally {
      pollStopped = true;
      window.clearInterval(intervalId);
      setProvisionBusy(false);
    }
  }

  async function finalWireGuardCheck() {
    setFinalCheckBusy(true);
    try {
      const readiness = await runWireGuardReadinessCheck();
      setWireGuardReadiness(readiness);
      const passed = readiness.installed && readiness.executable_found && readiness.service_available;
      setState((current) => ({ ...current, lastWireGuardPassed: passed }));
      return passed;
    } finally {
      setFinalCheckBusy(false);
    }
  }

  const canGoNext = useMemo(() => {
    if (step === 1) {
      return !!wireGuardReadiness && wireGuardReadiness.installed && wireGuardReadiness.executable_found && wireGuardReadiness.service_available;
    }
    if (step === 2) {
      return state.tokenValidated;
    }
    if (step === 3) {
      return !!state.region && !!state.plan && !!selectedPlanPricing;
    }
    if (step === 4) {
      return state.nodeReady && provisionPhase === "ready";
    }
    return true;
  }, [step, wireGuardReadiness, state, provisionPhase, selectedPlanPricing]);

  function goBack() {
    setStep((current) => Math.max(1, current - 1));
  }

  function goNext() {
    if (!canGoNext) return;
    setStep((current) => Math.min(5, current + 1));
  }

  function onCancel() {
    saveOnboardingState({ ...state, status: "in_progress", currentStep: step });
    router.push("/");
  }

  async function onFinish() {
    const passed = await finalWireGuardCheck();
    if (!passed) return;

    const completedState: OnboardingState = {
      ...state,
      status: "completed",
      currentStep: 5,
      lastWireGuardPassed: true,
    };
    saveOnboardingState(completedState);
    router.push("/");
  }

  return (
    <main className="mx-auto min-h-screen w-full max-w-5xl px-4 py-6 lg:px-6">
      <section className="rounded-[24px] border border-[color:var(--outline-variant)] bg-[linear-gradient(180deg,rgba(28,32,39,0.96),rgba(16,19,27,0.96))] p-6 shadow-[var(--shadow-elevated)] lg:p-8">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <div>
            <div className="text-xs uppercase tracking-[0.24em] text-[var(--on-surface-variant)]">Step {step} of 5</div>
            <h1 className="mt-2 text-2xl font-semibold tracking-tight text-[var(--on-surface)]">{stepTitle(step)}</h1>
          </div>
          <button
            type="button"
            onClick={onCancel}
            className="inline-flex items-center justify-center rounded-xl border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.03)] px-4 py-2 text-sm text-[var(--on-surface)] hover:bg-[color:rgba(255,255,255,0.08)]"
          >
            Cancel
          </button>
        </div>

        <div className="mt-6 rounded-[18px] border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.02)] p-5">
          {step === 1 ? (
            <div className="space-y-4">
              <p className="text-sm text-[var(--on-surface-variant)]">Check local WireGuard requirements used by this app before continuing.</p>
              {wireGuardReadiness ? (
                <div className={`rounded-xl border px-4 py-3 text-sm ${wireGuardReadiness.installed && wireGuardReadiness.executable_found && wireGuardReadiness.service_available ? "border-[color:rgba(78,222,163,0.25)] bg-[color:rgba(78,222,163,0.08)] text-[var(--secondary)]" : "border-[color:rgba(255,185,95,0.25)] bg-[color:rgba(255,185,95,0.08)] text-[var(--tertiary)]"}`}>
                  <div>{wireGuardReadiness.message}</div>
                  <div className="mt-1 text-xs opacity-90">{wireGuardReadiness.action_guidance}</div>
                </div>
              ) : null}
              <div className="grid gap-2 text-sm text-[var(--on-surface-variant)] md:grid-cols-2">
                <div>Installed: <span className="text-[var(--on-surface)]">{wireGuardReadiness?.installed ? "Yes" : "No"}</span></div>
                <div>Executables found: <span className="text-[var(--on-surface)]">{wireGuardReadiness?.executable_found ? "Yes" : "No"}</span></div>
                <div>Service available: <span className="text-[var(--on-surface)]">{wireGuardReadiness?.service_available ? "Yes" : "No"}</span></div>
                <div>Version: <span className="text-[var(--on-surface)]">{wireGuardReadiness?.version ?? "Unknown"}</span></div>
              </div>
              <div className="flex flex-wrap items-center gap-3">
                <button type="button" onClick={() => void checkWireGuard()} disabled={wireGuardBusy} className="inline-flex items-center justify-center rounded-xl border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.03)] px-4 py-2 text-sm text-[var(--on-surface)] hover:bg-[color:rgba(255,255,255,0.08)] disabled:opacity-50">
                  {wireGuardBusy ? "Checking..." : "Re-check"}
                </button>
                <button
                  type="button"
                  onClick={() => void onInstallWireGuardFromApp()}
                  disabled={wireGuardInstallBusy || wireGuardBusy}
                  className="inline-flex items-center justify-center rounded-xl border border-[color:rgba(171,199,255,0.25)] bg-[var(--primary)] px-4 py-2 text-sm font-medium text-[#10131b] disabled:opacity-50"
                >
                  {wireGuardInstallBusy ? "Installing WireGuard..." : "Install WireGuard from App (Admin)"}
                </button>
              </div>
              {wireGuardInstallMessage ? (
                <div className="text-xs text-[var(--on-surface-variant)]">{wireGuardInstallMessage}</div>
              ) : null}
            </div>
          ) : null}

          {step === 2 ? (
            <div className="space-y-4">
              <p className="text-sm text-[var(--on-surface-variant)]">Add your DigitalOcean API token to provision and manage the VPN node.</p>
              <label className="block space-y-2 text-sm text-[var(--on-surface-variant)]">
                <span>API Token</span>
                <input
                  type="password"
                  value={apiToken}
                  onChange={(event) => {
                    const nextValue = event.target.value;
                    setApiToken(nextValue);
                    if (nextValue) {
                      setState((current) => ({
                        ...current,
                        tokenSaved: false,
                        tokenValidated: false,
                      }));
                    }
                  }}
                  className="w-full rounded-xl border border-[color:var(--outline-variant)] bg-[color:rgba(8,10,16,0.9)] px-3 py-2.5 text-[var(--on-surface)] outline-none"
                  placeholder="dop_v1_..."
                />
              </label>
              <div className="flex flex-wrap gap-3">
                <button type="button" onClick={() => void onSaveToken()} disabled={tokenBusy} className="inline-flex items-center justify-center rounded-xl border border-[color:rgba(171,199,255,0.25)] bg-[var(--primary)] px-4 py-2 text-sm font-medium text-[#10131b] disabled:opacity-50">Save + Validate Token</button>
                <button type="button" onClick={() => void onValidateToken()} disabled={tokenBusy || hasUnsavedTokenInput || (!state.tokenSaved && !hasStoredProviderToken)} className="inline-flex items-center justify-center rounded-xl border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.03)] px-4 py-2 text-sm text-[var(--on-surface)] disabled:opacity-50">Validate Token</button>
              </div>
              <div className="text-xs text-[var(--on-surface-variant)]">
                Step 3 unlocks only after a successful Validate Token action.
              </div>
              <div className="rounded-xl border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.02)] px-4 py-3 text-sm text-[var(--on-surface-variant)]">{tokenMessage}</div>
            </div>
          ) : null}

          {step === 3 ? (
            <div className="space-y-5">
              <p className="text-sm text-[var(--on-surface-variant)]">Choose where to host your VPN node and how much capacity you need.</p>
              <div>
                <h2 className="text-sm font-medium text-[var(--on-surface)]">Region</h2>
                <div className="mt-3 grid gap-3 md:grid-cols-3">
                  {availableRegions.map((regionValue) => (
                    <button
                      key={regionValue}
                      type="button"
                      onClick={() => setState((current) => ({ ...current, region: regionValue, plan: "" }))}
                      className={`rounded-xl border px-4 py-3 text-left ${state.region === regionValue ? "border-[color:rgba(171,199,255,0.35)] bg-[color:rgba(171,199,255,0.12)]" : "border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.02)]"}`}
                    >
                      <div className="font-medium text-[var(--on-surface)]">{regionValue}</div>
                      <div className="mt-1 text-xs text-[var(--on-surface-variant)]">Live region option from provider API.</div>
                    </button>
                  ))}
                  {availableRegions.length === 0 ? <div className="rounded-xl border border-dashed border-[color:var(--outline-variant)] px-4 py-3 text-xs text-[var(--on-surface-variant)]">No regions available yet.</div> : null}
                </div>
              </div>
              <div>
                <h2 className="text-sm font-medium text-[var(--on-surface)]">Plan</h2>
                <div className="mt-3 grid gap-3 md:grid-cols-3">
                  {livePricing.map((plan) => (
                    <button
                      key={plan.plan_key}
                      type="button"
                      onClick={() => setState((current) => ({ ...current, plan: plan.plan_key }))}
                      className={`rounded-xl border px-4 py-3 text-left ${state.plan === plan.plan_key ? "border-[color:rgba(171,199,255,0.35)] bg-[color:rgba(171,199,255,0.12)]" : "border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.02)]"}`}
                    >
                      <div className="font-medium text-[var(--on-surface)]">{plan.plan_label}</div>
                      <div className="mt-1 font-mono text-xs text-[var(--on-surface-variant)]">{plan.size_slug}</div>
                      <div className="mt-2 text-xs text-[var(--on-surface-variant)]">{plan.vcpus} vCPU · {plan.memory_mb} MB · {plan.disk_gb} GB disk</div>
                      <div className="mt-2 text-xs text-[var(--secondary)]">${plan.monthly_usd.toFixed(2)}/mo</div>
                    </button>
                  ))}
                  {livePricing.length === 0 ? <div className="rounded-xl border border-dashed border-[color:var(--outline-variant)] px-4 py-3 text-xs text-[var(--on-surface-variant)]">No plans available for this region.</div> : null}
                </div>
              </div>
              <div className="rounded-xl border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.02)] px-4 py-3 text-sm text-[var(--on-surface-variant)]">
                <div className="flex items-center justify-between gap-3">
                  <span className="font-medium text-[var(--on-surface)]">Live pricing</span>
                  <button
                    type="button"
                    onClick={() => void loadLivePricing()}
                    disabled={pricingBusy || tokenBusy}
                    className="rounded-lg border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.03)] px-2.5 py-1 text-xs text-[var(--on-surface)] disabled:opacity-50"
                  >
                    {pricingBusy ? "Refreshing..." : "Refresh"}
                  </button>
                </div>
                {pricingError ? (
                  <div className="mt-2 text-xs text-[#ffd5d5]">{pricingError}</div>
                ) : selectedPlanPricing ? (
                  <div className="mt-2 grid gap-1 text-xs md:grid-cols-2">
                    <div>Size: <span className="text-[var(--on-surface)]">{selectedPlanPricing.size_slug}</span></div>
                    <div>Region: <span className="text-[var(--on-surface)]">{selectedPlanPricing.region}</span></div>
                    <div>Monthly: <span className="text-[var(--on-surface)]">${selectedPlanPricing.monthly_usd.toFixed(2)}</span></div>
                    <div>Hourly: <span className="text-[var(--on-surface)]">${selectedPlanPricing.hourly_usd.toFixed(5)}</span></div>
                    <div className="md:col-span-2">Source timestamp: <span className="text-[var(--on-surface)]">{selectedPlanPricing.source_timestamp}</span></div>
                  </div>
                ) : (
                  <div className="mt-2 text-xs">Select a plan to view live pricing.</div>
                )}
              </div>
              <div className="rounded-xl border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.02)] px-4 py-3 text-xs text-[var(--on-surface-variant)]">
                Cost vs latency: nearby regions usually improve latency; larger plans improve throughput and concurrent load.
              </div>
            </div>
          ) : null}

          {step === 4 ? (
            <div className="space-y-4">
              <p className="text-sm text-[var(--on-surface-variant)]">Create and bootstrap your server. This can take a few minutes.</p>
              <div className="grid gap-2 rounded-xl border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.02)] px-4 py-3 text-sm text-[var(--on-surface-variant)] md:grid-cols-2">
                <div>Node ID: <span className="text-[var(--on-surface)]">{state.nodeId}</span></div>
                <div>Node Name: <span className="text-[var(--on-surface)]">{state.nodeName}</span></div>
                <div>Region: <span className="text-[var(--on-surface)]">{state.region}</span></div>
                <div>Plan: <span className="text-[var(--on-surface)]">{state.plan}</span></div>
                <div>Size: <span className="text-[var(--on-surface)]">{selectedPlanPricing?.size_slug ?? "n/a"}</span></div>
                <div>Monthly est: <span className="text-[var(--on-surface)]">{selectedPlanPricing ? `$${selectedPlanPricing.monthly_usd.toFixed(2)}` : "n/a"}</span></div>
                <div>Hourly est: <span className="text-[var(--on-surface)]">{selectedPlanPricing ? `$${selectedPlanPricing.hourly_usd.toFixed(5)}` : "n/a"}</span></div>
              </div>
              <div className={`rounded-xl border px-4 py-3 text-sm ${provisionPhase === "ready" ? "border-[color:rgba(78,222,163,0.25)] bg-[color:rgba(78,222,163,0.08)] text-[var(--secondary)]" : provisionPhase === "error" ? "border-[color:rgba(255,107,107,0.25)] bg-[color:rgba(255,107,107,0.08)] text-[#ffd5d5]" : "border-[color:rgba(255,185,95,0.25)] bg-[color:rgba(255,185,95,0.08)] text-[var(--tertiary)]"}`}>
                <div className="font-medium">{provisionPhase === "idle" ? "Not started" : provisionPhase === "provisioning" ? "Provisioning" : provisionPhase === "bootstrapping" ? "Bootstrapping" : provisionPhase === "ready" ? "Ready" : "Error"}</div>
                <div className="mt-1 text-xs opacity-90">{provisionMessage}</div>
              </div>
              <button type="button" onClick={() => void startProvision()} disabled={provisionBusy || provisionPhase === "ready" || !selectedPlanPricing || !!pricingError} className="inline-flex items-center justify-center rounded-xl border border-[color:rgba(171,199,255,0.25)] bg-[var(--primary)] px-4 py-2 text-sm font-medium text-[#10131b] disabled:opacity-50">
                {provisionBusy ? "Provisioning..." : provisionPhase === "ready" ? "Provisioned" : "Start Provisioning"}
              </button>
            </div>
          ) : null}

          {step === 5 ? (
            <div className="space-y-4">
              <p className="text-sm text-[var(--on-surface-variant)]">Setup is complete. We will run one final WireGuard readiness check before returning you to Dashboard.</p>
              <div className="grid gap-2 rounded-xl border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.02)] px-4 py-3 text-sm text-[var(--on-surface-variant)] md:grid-cols-2">
                <div>Node: <span className="text-[var(--on-surface)]">{provisionNode?.name ?? state.nodeName}</span></div>
                <div>Region: <span className="text-[var(--on-surface)]">{provisionNode?.region ?? state.region}</span></div>
                <div>Public IP: <span className="text-[var(--on-surface)]">{provisionNode?.public_ipv4 ?? state.lastNodePublicIp ?? "n/a"}</span></div>
                <div>Status: <span className="text-[var(--on-surface)]">{provisionNode?.lifecycle_status ?? (state.nodeReady ? "ready" : "unknown")}</span></div>
              </div>
              <div className="rounded-xl border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.02)] px-4 py-3 text-sm text-[var(--on-surface-variant)]">
                WireGuard check: <span className="text-[var(--on-surface)]">{state.lastWireGuardPassed ? "Passed" : "Required"}</span>
              </div>
              <button type="button" onClick={() => void onFinish()} disabled={finalCheckBusy} className="inline-flex items-center justify-center rounded-xl border border-[color:rgba(171,199,255,0.25)] bg-[var(--primary)] px-5 py-2.5 text-sm font-medium text-[#10131b] disabled:opacity-50">
                {finalCheckBusy ? "Checking..." : "Go to Dashboard"}
              </button>
              {!state.lastWireGuardPassed && wireGuardReadiness ? (
                <div className="rounded-xl border border-[color:rgba(255,185,95,0.25)] bg-[color:rgba(255,185,95,0.08)] px-4 py-3 text-xs text-[var(--tertiary)]">
                  {wireGuardReadiness.message} {wireGuardReadiness.action_guidance}
                </div>
              ) : null}
            </div>
          ) : null}
        </div>

        <div className="mt-6 flex items-center justify-between">
          <button
            type="button"
            onClick={goBack}
            disabled={step === 1 || provisionBusy || finalCheckBusy}
            className="inline-flex items-center justify-center rounded-xl border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.03)] px-4 py-2 text-sm text-[var(--on-surface)] disabled:opacity-40"
          >
            Back
          </button>

          {step < 5 ? (
            <button
              type="button"
              onClick={goNext}
              disabled={!canGoNext || provisionBusy || tokenBusy || wireGuardBusy}
              className="inline-flex items-center justify-center rounded-xl border border-[color:rgba(171,199,255,0.25)] bg-[var(--primary)] px-4 py-2 text-sm font-medium text-[#10131b] disabled:opacity-40"
            >
              Next
            </button>
          ) : <div />}
        </div>
      </section>
    </main>
  );
}
