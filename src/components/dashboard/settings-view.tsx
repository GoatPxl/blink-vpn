"use client";

import { useMemo, useState } from "react";
import { AppIcon, Button, Card, Field, PageChrome, type AppView, type ProviderAccessState, type ProviderRecord, type DashboardBusyAction } from "./shared";
import { maskTokenReference, providerAccessBadge } from "./shared";

export function SettingsView({
  providerAccessState,
  providerStatusMessage,
  providerFeedback,
  dockerDevEnabled,
  dockerReadinessState,
  dockerReadinessMessage,
  dockerReadiness,
  dockerFauxToken,
  setDockerFauxToken,
  dockerFauxTokenConfigured,
  dockerFauxTokenConfiguredAt,
  providerHasStoredToken,
  providerBadge,
  configuredProvider,
  providerId,
  providers,
  onSelectProvider,
  apiToken,
  setApiToken,
  onSaveToken,
  onSaveDockerFauxToken,
  onValidateToken,
  onValidateDockerReadiness,
  onSuspendProvider,
  onResumeProvider,
  onDeleteProvider,
  busyAction,
  onNavigateToView,
}: {
  providerAccessState: ProviderAccessState;
  providerStatusMessage: string;
  providerFeedback: string;
  dockerDevEnabled: boolean;
  dockerReadinessState: ProviderAccessState;
  dockerReadinessMessage: string;
  dockerReadiness: { cli_version: string | null; daemon_version: string | null; action_guidance: string } | null;
  dockerFauxToken: string;
  setDockerFauxToken: (value: string) => void;
  dockerFauxTokenConfigured: boolean;
  dockerFauxTokenConfiguredAt: string | null;
  providerHasStoredToken: boolean;
  providerBadge: { label: string; classes: string };
  configuredProvider: ProviderRecord | null;
  providerId: string;
  providers: ProviderRecord[];
  onSelectProvider: (providerId: string) => void;
  apiToken: string;
  setApiToken: (value: string) => void;
  onSaveToken: () => Promise<void>;
  onSaveDockerFauxToken: () => Promise<void>;
  onValidateToken: () => Promise<void>;
  onValidateDockerReadiness: () => Promise<void>;
  onSuspendProvider: (provider: ProviderRecord) => Promise<void>;
  onResumeProvider: (provider: ProviderRecord) => Promise<void>;
  onDeleteProvider: (provider: ProviderRecord) => Promise<void>;
  busyAction: DashboardBusyAction;
  onNavigateToView: (view: AppView) => void;
}) {
  const dockerBadge = providerAccessBadge(dockerReadinessState);
  const [tokenModalProviderId, setTokenModalProviderId] = useState<string | null>(null);

  const selectedProvider = useMemo(
    () => providers.find((provider) => provider.id === providerId) ?? configuredProvider ?? providers[0] ?? null,
    [configuredProvider, providerId, providers],
  );
  const tokenModalProvider = useMemo(
    () => providers.find((provider) => provider.id === tokenModalProviderId) ?? selectedProvider,
    [providers, selectedProvider, tokenModalProviderId],
  );

  const isTokenModalOpen = tokenModalProviderId !== null;
  const isDockerProviderModal = tokenModalProvider?.slug === "docker";
  const canDeleteProviders = providers.length > 1;
  const modalPlaceholder = isDockerProviderModal ? "dev-docker-token" : "dop_v1_...";
  const modalHint = isDockerProviderModal
    ? "Docker development token is local-only emulation state."
    : "Personal access token is encrypted at rest and stored as a write-only reference.";
  const modalTokenValue = isDockerProviderModal ? dockerFauxToken : apiToken;

  function providerIconName(provider: ProviderRecord): string {
    return provider.slug === "docker" ? "server" : "cloud";
  }

  function providerStatusBadge(provider: ProviderRecord): { label: string; classes: string } {
    if (provider.status === "suspended") {
      return {
        label: "Suspended",
        classes: "border-[color:rgba(255,185,95,0.25)] bg-[color:rgba(255,185,95,0.08)] text-[var(--tertiary)]",
      };
    }
    return {
      label: "Connected",
      classes: "border-[color:rgba(78,222,163,0.25)] bg-[color:rgba(78,222,163,0.1)] text-[var(--secondary)]",
    };
  }

  function openTokenModal(provider: ProviderRecord) {
    onSelectProvider(provider.id);
    setTokenModalProviderId(provider.id);
  }

  function closeTokenModal() {
    setTokenModalProviderId(null);
  }

  async function handleSaveFromModal() {
    if (isDockerProviderModal) {
      await onSaveDockerFauxToken();
      return;
    }
    await onSaveToken();
  }

  async function handleValidateFromModal() {
    if (isDockerProviderModal) {
      await onValidateDockerReadiness();
      return;
    }
    await onValidateToken();
  }

  return (
    <PageChrome>
      <div className="mx-auto w-full max-w-5xl space-y-4">
        <div className="text-xs text-[var(--on-surface-variant)]">
          <span className="inline-flex items-center gap-2"><AppIcon name="settings" className="h-4 w-4" />Settings</span>
          <span className="mx-2">›</span>
          <span>Provider Integration</span>
        </div>
        <div className="grid gap-4 xl:grid-cols-[minmax(0,1.1fr)_minmax(0,0.9fr)]">
          <div className="space-y-4">
            <Card title="Select Provider" description="Choose a provider card to connect or update token access.">
              <div className="grid gap-3 sm:grid-cols-2">
                {providers.map((provider) => {
                  const hasToken = provider.slug === "docker" ? dockerFauxTokenConfigured : !!provider.token_ref;
                  const statusBadge = providerStatusBadge(provider);
                  const isSelected = selectedProvider?.id === provider.id;
                  return (
                    <button
                      key={provider.id}
                      type="button"
                      onClick={() => openTokenModal(provider)}
                      className={`rounded-2xl border p-4 text-left transition-colors ${isSelected ? "border-[color:rgba(171,199,255,0.5)] bg-[color:rgba(171,199,255,0.09)]" : "border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.02)] hover:bg-[color:rgba(255,255,255,0.05)]"}`}
                      aria-label={`Select ${provider.name} provider`}
                    >
                      <div className="flex items-center justify-between gap-3">
                        <div className="inline-flex items-center gap-2 text-[var(--on-surface)]">
                          <AppIcon name={providerIconName(provider)} className="h-4 w-4" />
                          <span className="font-medium">{provider.name}</span>
                        </div>
                        <span className={`inline-flex rounded-full border px-2.5 py-1 text-xs ${statusBadge.classes}`}>{statusBadge.label}</span>
                      </div>
                      <div className="mt-2 font-mono text-[12px] text-[var(--on-surface-variant)]">{provider.slug}</div>
                      <div className="mt-3 flex items-center gap-2 text-xs text-[var(--on-surface-variant)]">
                        <span className={`inline-flex rounded-full border px-2 py-0.5 ${hasToken ? "border-[color:rgba(78,222,163,0.25)] bg-[color:rgba(78,222,163,0.1)] text-[var(--secondary)]" : "border-[color:rgba(255,185,95,0.25)] bg-[color:rgba(255,185,95,0.08)] text-[var(--tertiary)]"}`}>{hasToken ? "Token configured" : "Token missing"}</span>
                        <span>{isSelected ? "Selected" : "Click to connect"}</span>
                      </div>
                    </button>
                  );
                })}
              </div>
            </Card>

            <Card title="Provider Readiness" description="This status drives whether provisioning is available from the Nodes page.">
              <div className="grid gap-3 md:grid-cols-2">
                <div className="rounded-[18px] border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.02)] p-4">
                  <div className="text-[11px] uppercase tracking-[0.24em] text-[var(--on-surface-variant)]">Stored Token</div>
                  <div className="mt-2 flex items-center gap-2">
                    <span className={`inline-flex rounded-full border px-2.5 py-1 text-xs ${providerHasStoredToken ? "border-[color:rgba(78,222,163,0.25)] bg-[color:rgba(78,222,163,0.1)] text-[var(--secondary)]" : "border-[color:rgba(255,185,95,0.25)] bg-[color:rgba(255,185,95,0.08)] text-[var(--tertiary)]"}`}>{providerHasStoredToken ? "Configured" : "Missing"}</span>
                    <span className="text-sm text-[var(--on-surface)]">{maskTokenReference(selectedProvider?.token_ref ?? null)}</span>
                  </div>
                  <p className="mt-2 text-sm text-[var(--on-surface-variant)]">Raw provider tokens remain write-only. This view only exposes the stored reference.</p>
                </div>
                <div className="rounded-[18px] border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.02)] p-4">
                  <div className="text-[11px] uppercase tracking-[0.24em] text-[var(--on-surface-variant)]">Provider Access</div>
                  <div className="mt-2 flex items-center gap-2">
                    <span className={`inline-flex rounded-full border px-2.5 py-1 text-xs ${providerBadge.classes}`}>{providerBadge.label}</span>
                  </div>
                  <p className="mt-2 text-sm text-[var(--on-surface-variant)]">{providerStatusMessage}</p>
                </div>
              </div>
            </Card>

            <Card title="Connected Providers" description="View status and manage each provider integration.">
              <div className="overflow-hidden rounded-xl border border-[color:var(--outline-variant)]">
                <table className="w-full text-sm">
                  <thead>
                    <tr className="border-b border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.02)]">
                      <th className="px-4 py-3 text-left font-medium text-[var(--on-surface-variant)]">Provider</th>
                      <th className="px-4 py-3 text-left font-medium text-[var(--on-surface-variant)]">Token</th>
                      <th className="px-4 py-3 text-left font-medium text-[var(--on-surface-variant)]">Status</th>
                      <th className="px-4 py-3 text-right font-medium text-[var(--on-surface-variant)]">Management</th>
                    </tr>
                  </thead>
                  <tbody>
                    {providers.map((provider) => {
                      const hasToken = provider.slug === "docker" ? dockerFauxTokenConfigured : !!provider.token_ref;
                      const statusBadge = providerStatusBadge(provider);
                      return (
                        <tr key={provider.id} className="border-b border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.01)] last:border-b-0">
                          <td className="px-4 py-3">
                            <div className="font-medium text-[var(--on-surface)]">{provider.name}</div>
                            <div className="mt-1 font-mono text-[12px] text-[var(--on-surface-variant)]">{provider.slug}</div>
                          </td>
                          <td className="px-4 py-3">
                            <span className={`inline-flex rounded-full border px-2.5 py-1 text-xs ${hasToken ? "border-[color:rgba(78,222,163,0.25)] bg-[color:rgba(78,222,163,0.1)] text-[var(--secondary)]" : "border-[color:rgba(255,185,95,0.25)] bg-[color:rgba(255,185,95,0.08)] text-[var(--tertiary)]"}`}>{hasToken ? "Configured" : "Missing"}</span>
                            <div className="mt-1 text-xs text-[var(--on-surface-variant)]">{maskTokenReference(provider.token_ref)}</div>
                          </td>
                          <td className="px-4 py-3">
                            <span className={`inline-flex rounded-full border px-2.5 py-1 text-xs ${statusBadge.classes}`}>{statusBadge.label}</span>
                          </td>
                          <td className="px-4 py-3 text-right">
                            <div className="inline-flex flex-wrap items-center justify-end gap-2">
                              <Button tone="secondary" onClick={() => openTokenModal(provider)} disabled={busyAction !== null}>Change Token</Button>
                              {provider.status === "suspended" ? (
                                <Button tone="secondary" onClick={() => void onResumeProvider(provider)} disabled={busyAction !== null}>Resume</Button>
                              ) : (
                                <Button
                                  tone="secondary"
                                  onClick={() => {
                                    if (window.confirm(`Suspend ${provider.name}? Provisioning operations for this provider will be blocked until resumed.`)) {
                                      void onSuspendProvider(provider);
                                    }
                                  }}
                                  disabled={busyAction !== null}
                                >
                                  Suspend
                                </Button>
                              )}
                              <Button
                                tone="danger"
                                onClick={() => {
                                  if (window.confirm(`Delete ${provider.name}? This removes its token reference and provider record from this device.`)) {
                                    void onDeleteProvider(provider);
                                  }
                                }}
                                disabled={busyAction !== null || !canDeleteProviders}
                              >
                                Delete
                              </Button>
                            </div>
                          </td>
                        </tr>
                      );
                    })}
                  </tbody>
                </table>
              </div>
              <p className="mt-3 text-xs text-[var(--on-surface-variant)]">
                {canDeleteProviders
                  ? "Suspending blocks provider operations. Deleting removes provider metadata and any stored token reference."
                  : "At least one provider record must remain configured on this device."}
              </p>
            </Card>

            {dockerDevEnabled ? (
              <Card title="Docker Service (Development)" description="Development-only mirror checks and faux access validation.">
                <div className="grid gap-3 md:grid-cols-2">
                  <div className="rounded-[18px] border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.02)] p-4">
                    <div className="text-[11px] uppercase tracking-[0.24em] text-[var(--on-surface-variant)]">Faux Token</div>
                    <div className="mt-2 flex items-center gap-2">
                      <span className={`inline-flex rounded-full border px-2.5 py-1 text-xs ${dockerFauxTokenConfigured ? "border-[color:rgba(78,222,163,0.25)] bg-[color:rgba(78,222,163,0.1)] text-[var(--secondary)]" : "border-[color:rgba(255,185,95,0.25)] bg-[color:rgba(255,185,95,0.08)] text-[var(--tertiary)]"}`}>{dockerFauxTokenConfigured ? "Configured" : "Missing"}</span>
                    </div>
                    <p className="mt-2 text-sm text-[var(--on-surface-variant)]">{dockerFauxTokenConfiguredAt ? `Saved at ${new Date(dockerFauxTokenConfiguredAt).toLocaleString()}` : "No faux token saved yet."}</p>
                  </div>
                  <div className="rounded-[18px] border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.02)] p-4">
                    <div className="text-[11px] uppercase tracking-[0.24em] text-[var(--on-surface-variant)]">Docker Readiness</div>
                    <div className="mt-2 flex items-center gap-2">
                      <span className={`inline-flex rounded-full border px-2.5 py-1 text-xs ${dockerBadge.classes}`}>{dockerBadge.label}</span>
                    </div>
                    <p className="mt-2 text-sm text-[var(--on-surface-variant)]">{dockerReadinessMessage}</p>
                    <p className="mt-2 text-xs text-[var(--on-surface-variant)]">CLI: {dockerReadiness?.cli_version ?? "n/a"} · Daemon: {dockerReadiness?.daemon_version ?? "n/a"}</p>
                    <p className="mt-2 text-xs text-[var(--on-surface-variant)]">{dockerReadiness?.action_guidance ?? "Save faux token and validate readiness to emulate provider access."}</p>
                  </div>
                </div>
              </Card>
            ) : null}
          </div>

          <Card title="Selected Provider" description="Current provider context used by token validation and provisioning checks.">
            <div className="space-y-4">
              <div className="rounded-[18px] border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.02)] p-4">
                <div className="text-[11px] uppercase tracking-[0.24em] text-[var(--on-surface-variant)]">Selected Provider</div>
                <div className="mt-2 flex items-center gap-2 text-[var(--on-surface)]">
                  <AppIcon name={selectedProvider ? providerIconName(selectedProvider) : "cloud"} className="h-4 w-4" />
                  <span className="font-medium">{selectedProvider?.name ?? "Not selected"}</span>
                </div>
                <div className="mt-2 font-mono text-[12px] text-[var(--on-surface-variant)]">{selectedProvider?.id ?? "n/a"}</div>
                <div className="mt-3">
                  <Button tone="secondary" onClick={() => selectedProvider ? openTokenModal(selectedProvider) : undefined} disabled={!selectedProvider || busyAction !== null}>
                    Open Token Prompt
                  </Button>
                </div>
              </div>
              <div className="rounded-[18px] border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.02)] p-4 text-xs text-[var(--on-surface-variant)]">
                {providerFeedback}
              </div>
            </div>
          </Card>

          <Card title="Provider Action Feedback" description="Validation and readiness updates are surfaced here.">
            <div className="space-y-5">
              <div className="rounded-[18px] border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.02)] p-4 text-sm text-[var(--on-surface)]">
                {providerStatusMessage}
              </div>
              <div className="flex items-center justify-end">
                <Button tone="secondary" onClick={() => onNavigateToView("dashboard")}>Back to Dashboard</Button>
              </div>
            </div>
          </Card>

          {dockerDevEnabled ? (
            <Card title="Docker Faux Token" description="Development-only access emulation for Docker workflows.">
              <div className="space-y-5">
                <Field label="Faux Token">
                  <div className="relative">
                    <input value={dockerFauxToken} onChange={(event) => setDockerFauxToken(event.target.value)} placeholder="dev-docker-token" className="w-full rounded-xl border border-[color:var(--outline-variant)] bg-[color:rgba(8,10,16,0.9)] px-3 py-2.5 pr-10 text-[var(--on-surface)] outline-none ring-0 placeholder:text-[var(--on-surface-variant)]/50 focus:border-[color:rgba(171,199,255,0.45)]" />
                    <span className="pointer-events-none absolute right-3 top-1/2 -translate-y-1/2 text-[var(--on-surface-variant)]"><AppIcon name="lock" className="h-4 w-4" /></span>
                  </div>
                </Field>
                <div className="text-xs text-[var(--on-surface-variant)]">This token is local-only emulation state and does not authenticate to remote infrastructure.</div>
                <div className="flex items-center justify-end gap-2">
                  <Button tone="secondary" onClick={() => void onSaveDockerFauxToken()} disabled={busyAction !== null || !dockerFauxToken.trim()}>
                    {busyAction === "save-docker-token" ? "Saving" : "Save Faux Token"}
                  </Button>
                  <Button onClick={() => void onValidateDockerReadiness()} disabled={busyAction !== null || !dockerFauxTokenConfigured}>
                    {busyAction === "validate-docker" ? "Validating" : "Validate Docker"} <span aria-hidden="true">→</span>
                  </Button>
                </div>
              </div>
            </Card>
          ) : null}
        </div>
        <div className="text-center text-xs text-[var(--on-surface-variant)]">Tokens are encrypted at rest, used only for provider API calls from this device, and never transmitted to GoatStare services.</div>
      </div>

      {isTokenModalOpen && tokenModalProvider ? (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/55 p-4">
          <div className="w-full max-w-2xl rounded-2xl border border-[color:var(--outline-variant)] bg-[color:rgba(10,12,18,0.96)] p-6 shadow-2xl">
            <div className="flex items-start justify-between gap-3">
              <div>
                <p className="text-xs uppercase tracking-[0.24em] text-[var(--on-surface-variant)]">Connect Provider</p>
                <h2 className="mt-2 text-xl font-semibold text-[var(--on-surface)]">{tokenModalProvider.name}</h2>
                <p className="mt-1 text-sm text-[var(--on-surface-variant)]">Paste or update credentials for {tokenModalProvider.slug}.</p>
              </div>
              <Button tone="secondary" onClick={closeTokenModal} disabled={busyAction !== null}>Close</Button>
            </div>

            <div className="mt-6 space-y-5">
              <Field label={isDockerProviderModal ? "Docker Faux Token" : "Provider API Token"} hint={modalHint}>
                <textarea
                  value={modalTokenValue}
                  onChange={(event) => {
                    if (isDockerProviderModal) {
                      setDockerFauxToken(event.target.value);
                    } else {
                      setApiToken(event.target.value);
                    }
                  }}
                  placeholder={modalPlaceholder}
                  rows={5}
                  className="w-full resize-y rounded-xl border border-[color:var(--outline-variant)] bg-[color:rgba(8,10,16,0.9)] px-3 py-3 text-[var(--on-surface)] outline-none ring-0 placeholder:text-[var(--on-surface-variant)]/50 focus:border-[color:rgba(171,199,255,0.45)]"
                />
              </Field>

              <div className="rounded-xl border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.02)] px-3 py-2 text-xs text-[var(--on-surface-variant)]">
                {providerFeedback}
              </div>

              <div className="flex flex-wrap items-center justify-end gap-2">
                <Button tone="secondary" onClick={closeTokenModal} disabled={busyAction !== null}>Cancel</Button>
                <Button
                  tone="secondary"
                  onClick={() => void handleSaveFromModal()}
                  disabled={busyAction !== null || !modalTokenValue.trim()}
                >
                  {busyAction === "save-token" || busyAction === "save-docker-token" ? "Saving" : "Save Token"}
                </Button>
                <Button
                  onClick={() => void handleValidateFromModal()}
                  disabled={busyAction !== null || (isDockerProviderModal ? !dockerFauxTokenConfigured : !(tokenModalProvider.token_ref || modalTokenValue.trim()))}
                >
                  {busyAction === "validate-token" || busyAction === "validate-docker" ? "Validating" : "Validate Token"} <span aria-hidden="true">→</span>
                </Button>
              </div>
            </div>
          </div>
        </div>
      ) : null}
    </PageChrome>
  );
}
