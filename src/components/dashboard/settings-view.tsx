"use client";

import { useMemo, useState } from "react";
import {
  AppIcon,
  Button,
  Card,
  Field,
  PageChrome,
  type AppView,
  type ProviderRecord,
  type DashboardBusyAction,
} from "./shared";
import { maskTokenReference } from "./shared";

export function SettingsView({
  providerStatusMessage,
  providerFeedback,
  dockerDevEnabled,
  dockerFauxToken,
  setDockerFauxToken,
  dockerFauxTokenConfigured,
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
  onDeleteProvider,
  busyAction,
  onNavigateToView,
}: {
  providerStatusMessage: string;
  providerFeedback: string;
  dockerDevEnabled: boolean;
  dockerFauxToken: string;
  setDockerFauxToken: (value: string) => void;
  dockerFauxTokenConfigured: boolean;
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
  onDeleteProvider: (provider: ProviderRecord) => Promise<void>;
  busyAction: DashboardBusyAction;
  onNavigateToView: (view: AppView) => void;
}) {
  const [tokenModalProviderId, setTokenModalProviderId] = useState<string>(providerId);
  const [isTokenModalOpen, setIsTokenModalOpen] = useState(false);

  const selectedProvider = useMemo(() => {
    return providers.find((provider) => provider.id === tokenModalProviderId)
      ?? providers.find((provider) => provider.id === providerId)
      ?? configuredProvider
      ?? providers[0]
      ?? null;
  }, [configuredProvider, providerId, providers, tokenModalProviderId]);

  const isDockerProviderModal = selectedProvider?.slug === "docker";
  const canDeleteProviders = providers.length > 1;
  const modalPlaceholder = isDockerProviderModal ? "dev-docker-token" : "dop_v1_...";
  const modalHint = isDockerProviderModal
    ? "Local-only token used for Docker development emulation."
    : "Token is encrypted at rest and stored as a write-only reference.";
  const modalTokenValue = isDockerProviderModal ? dockerFauxToken : apiToken;

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

  function openTokenModal() {
    setTokenModalProviderId(providerId);
    setIsTokenModalOpen(true);
  }

  function closeTokenModal() {
    setIsTokenModalOpen(false);
  }

  function changeModalProvider(nextProviderId: string) {
    setTokenModalProviderId(nextProviderId);
    onSelectProvider(nextProviderId);
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

  const providerRows = useMemo(() => {
    return providers.map((provider) => {
      const tokenConfigured = provider.slug === "docker" ? dockerFauxTokenConfigured : !!provider.token_ref;
      return {
        provider,
        tokenConfigured,
        tokenReference: provider.slug === "docker"
          ? (dockerFauxTokenConfigured ? "dev-token-configured" : null)
          : provider.token_ref,
      };
    });
  }, [providers, dockerFauxTokenConfigured]);

  return (
    <PageChrome>
      <div className="mx-auto w-full max-w-6xl space-y-5">
        <div className="text-xs text-[var(--on-surface-variant)]">
          <span className="inline-flex items-center gap-2"><AppIcon name="settings" className="h-4 w-4" />Settings</span>
          <span className="mx-2">›</span>
          <span>Provider Integration</span>
        </div>
        <Card
          title="Provider API Keys"
          description="This page exists to collect API keys. Choose provider, paste key, save, then validate."
          action={<Button onClick={openTokenModal} disabled={busyAction !== null || providers.length === 0}>Open Key Entry Modal</Button>}
        >
          <div className="grid gap-4 lg:grid-cols-[minmax(0,1fr)_minmax(0,2fr)]">
            <div className="rounded-2xl border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.03)] p-4">
              <div className="text-[11px] uppercase tracking-[0.24em] text-[var(--on-surface-variant)]">Current Provider Context</div>
              <div className="mt-3 text-lg font-semibold text-[var(--on-surface)]">{selectedProvider?.name ?? "No provider"}</div>
              <div className="mt-1 font-mono text-[12px] text-[var(--on-surface-variant)]">{selectedProvider?.slug ?? "n/a"}</div>
              <div className="mt-4 text-sm text-[var(--on-surface-variant)]">{providerStatusMessage}</div>
              <div className="mt-3 rounded-xl border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.02)] px-3 py-2 text-xs text-[var(--on-surface-variant)]">
                {providerFeedback}
              </div>
            </div>

            <div className="rounded-2xl border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.02)] p-4">
              <div className="mb-3 text-sm font-medium text-[var(--on-surface)]">Installed API keys by provider</div>
              <div className="overflow-hidden rounded-xl border border-[color:var(--outline-variant)]">
                <table className="w-full text-sm">
                  <thead>
                    <tr className="border-b border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.02)]">
                      <th className="px-4 py-3 text-left font-medium text-[var(--on-surface-variant)]">Provider</th>
                      <th className="px-4 py-3 text-left font-medium text-[var(--on-surface-variant)]">API Key</th>
                      <th className="px-4 py-3 text-left font-medium text-[var(--on-surface-variant)]">Status</th>
                      <th className="px-4 py-3 text-right font-medium text-[var(--on-surface-variant)]">Actions</th>
                    </tr>
                  </thead>
                  <tbody>
                    {providerRows.map(({ provider, tokenConfigured, tokenReference }) => {
                      const statusBadge = providerStatusBadge(provider);
                      return (
                        <tr key={provider.id} className="border-b border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.01)] last:border-b-0">
                          <td className="px-4 py-3">
                            <div className="font-medium text-[var(--on-surface)]">{provider.name}</div>
                            <div className="mt-1 font-mono text-[12px] text-[var(--on-surface-variant)]">{provider.slug}</div>
                          </td>
                          <td className="px-4 py-3">
                            <span className={`inline-flex rounded-full border px-2.5 py-1 text-xs ${tokenConfigured ? "border-[color:rgba(78,222,163,0.25)] bg-[color:rgba(78,222,163,0.1)] text-[var(--secondary)]" : "border-[color:rgba(255,185,95,0.25)] bg-[color:rgba(255,185,95,0.08)] text-[var(--tertiary)]"}`}>{tokenConfigured ? "Installed" : "Missing"}</span>
                            <div className="mt-1 text-xs text-[var(--on-surface-variant)]">{maskTokenReference(tokenReference)}</div>
                          </td>
                          <td className="px-4 py-3">
                            <span className={`inline-flex rounded-full border px-2.5 py-1 text-xs ${statusBadge.classes}`}>{statusBadge.label}</span>
                          </td>
                          <td className="px-4 py-3 text-right">
                            <div className="inline-flex flex-wrap items-center justify-end gap-2">
                              <Button
                                tone="secondary"
                                onClick={() => {
                                  if (window.confirm(`Suspend ${provider.name}? Provisioning operations for this provider will be blocked until resumed.`)) {
                                    void onSuspendProvider(provider);
                                  }
                                }}
                                disabled={busyAction !== null || provider.status === "suspended"}
                              >
                                {provider.status === "suspended" ? "Suspended" : "Suspend"}
                              </Button>
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
            </div>
          </div>
          <div className="flex items-center justify-end">
            <Button tone="secondary" onClick={() => onNavigateToView("dashboard")}>Back to Dashboard</Button>
          </div>
        </Card>

        <div className="text-center text-xs text-[var(--on-surface-variant)]">Tokens are encrypted at rest, used only for provider API calls from this device, and never transmitted to GoatStare services.</div>
      </div>

      {isTokenModalOpen && selectedProvider ? (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/70 p-4 backdrop-blur-[2px]">
          <div className="w-full max-w-4xl rounded-3xl border border-[color:var(--outline-variant)] bg-[linear-gradient(180deg,rgba(22,25,34,0.98),rgba(11,14,22,0.98))] p-7 shadow-2xl">
            <div className="flex items-start justify-between gap-3">
              <div>
                <p className="text-xs uppercase tracking-[0.24em] text-[var(--on-surface-variant)]">API Key Entry</p>
                <h2 className="mt-2 text-3xl font-semibold tracking-tight text-[var(--on-surface)]">Provider Credential</h2>
                <p className="mt-2 text-sm text-[var(--on-surface-variant)]">Pick a provider, paste key, save, and validate.</p>
              </div>
              <Button tone="secondary" onClick={closeTokenModal} disabled={busyAction !== null}>Close</Button>
            </div>

            <div className="mt-7 space-y-6">
              <Field label="Provider">
                <select
                  value={selectedProvider.id}
                  onChange={(event) => changeModalProvider(event.target.value)}
                  className="w-full rounded-xl border border-[color:var(--outline-variant)] bg-[color:rgba(8,10,16,0.95)] px-3 py-3 text-[var(--on-surface)] outline-none ring-0 focus:border-[color:rgba(171,199,255,0.45)]"
                >
                  {providers.map((provider) => (
                    <option key={provider.id} value={provider.id}>
                      {provider.name} ({provider.slug})
                    </option>
                  ))}
                </select>
              </Field>

              <Field label={isDockerProviderModal ? "Docker Key" : "API Key"} hint={modalHint}>
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
                  rows={8}
                  className="w-full resize-y rounded-2xl border border-[color:var(--outline-variant)] bg-[color:rgba(8,10,16,0.95)] px-4 py-4 font-mono text-base text-[var(--on-surface)] outline-none ring-0 placeholder:text-[var(--on-surface-variant)]/50 focus:border-[color:rgba(171,199,255,0.55)]"
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
                  disabled={busyAction !== null || (isDockerProviderModal ? !dockerFauxTokenConfigured : !(selectedProvider.token_ref || modalTokenValue.trim()))}
                >
                  {busyAction === "validate-token" || busyAction === "validate-docker" ? "Validating" : "Validate Key"} <span aria-hidden="true">→</span>
                </Button>
              </div>

              {dockerDevEnabled && isDockerProviderModal ? (
                <p className="text-xs text-[var(--on-surface-variant)]">
                  Docker provider uses a local development key path and can be validated with the same modal flow.
                </p>
              ) : null}
            </div>
          </div>
        </div>
      ) : null}
    </PageChrome>
  );
}
