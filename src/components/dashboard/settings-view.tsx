"use client";

import { AppIcon, Button, Card, Field, PageChrome, type AppView, type ProviderAccessState, type ProviderRecord, type RecoveryState, type DashboardBusyAction } from "./shared";
import { maskTokenReference, providerAccessBadge } from "./shared";

export function SettingsView({
  providerAccessState,
  providerStatusMessage,
  providerFeedback,
  providerHasStoredToken,
  providerBadge,
  configuredProvider,
  providerId,
  providerName,
  apiToken,
  setApiToken,
  onSaveToken,
  onValidateToken,
  busyAction,
  onNavigateToView,
}: {
  providerAccessState: ProviderAccessState;
  providerStatusMessage: string;
  providerFeedback: string;
  providerHasStoredToken: boolean;
  providerBadge: { label: string; classes: string };
  configuredProvider: ProviderRecord | null;
  providerId: string;
  providerName: string;
  apiToken: string;
  setApiToken: (value: string) => void;
  onSaveToken: () => Promise<void>;
  onValidateToken: () => Promise<void>;
  busyAction: DashboardBusyAction;
  onNavigateToView: (view: AppView) => void;
}) {
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
            <Card title="Provider Readiness" description="This status drives whether provisioning is available from the Nodes page.">
              <div className="grid gap-3 md:grid-cols-2">
                <div className="rounded-[18px] border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.02)] p-4">
                  <div className="text-[11px] uppercase tracking-[0.24em] text-[var(--on-surface-variant)]">Stored Token</div>
                  <div className="mt-2 flex items-center gap-2">
                    <span className={`inline-flex rounded-full border px-2.5 py-1 text-xs ${providerHasStoredToken ? "border-[color:rgba(78,222,163,0.25)] bg-[color:rgba(78,222,163,0.1)] text-[var(--secondary)]" : "border-[color:rgba(255,185,95,0.25)] bg-[color:rgba(255,185,95,0.08)] text-[var(--tertiary)]"}`}>{providerHasStoredToken ? "Configured" : "Missing"}</span>
                    <span className="text-sm text-[var(--on-surface)]">{maskTokenReference(configuredProvider?.token_ref ?? null)}</span>
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

            <Card title="Current Provider" description="Saved provider configuration for this device.">
              <div className="overflow-hidden rounded-xl border border-[color:var(--outline-variant)]">
                <table className="w-full text-sm">
                  <tbody>
                    <tr className="border-b border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.02)]">
                      <th className="w-[42%] px-4 py-3 text-left font-medium text-[var(--on-surface-variant)]">Provider</th>
                      <td className="px-4 py-3 text-[var(--on-surface)]">{configuredProvider?.name ?? providerName}</td>
                    </tr>
                    <tr className="border-b border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.01)]">
                      <th className="px-4 py-3 text-left font-medium text-[var(--on-surface-variant)]">Provider ID</th>
                      <td className="px-4 py-3 font-mono text-[13px] text-[var(--on-surface)]">{configuredProvider?.id ?? providerId}</td>
                    </tr>
                    <tr className="border-b border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.02)]">
                      <th className="px-4 py-3 text-left font-medium text-[var(--on-surface-variant)]">Token Status</th>
                      <td className="px-4 py-3">
                        <span className={`inline-flex rounded-full border px-2.5 py-1 text-xs ${providerHasStoredToken ? "border-[color:rgba(78,222,163,0.25)] bg-[color:rgba(78,222,163,0.1)] text-[var(--secondary)]" : "border-[color:rgba(255,185,95,0.25)] bg-[color:rgba(255,185,95,0.08)] text-[var(--tertiary)]"}`}>
                          {providerHasStoredToken ? "Configured" : "Missing"}
                        </span>
                      </td>
                    </tr>
                    <tr className="border-b border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.01)]">
                      <th className="px-4 py-3 text-left font-medium text-[var(--on-surface-variant)]">Provisioning Status</th>
                      <td className="px-4 py-3">
                        <span className={`inline-flex rounded-full border px-2.5 py-1 text-xs ${providerBadge.classes}`}>{providerBadge.label}</span>
                      </td>
                    </tr>
                    <tr className="bg-[color:rgba(255,255,255,0.01)]">
                      <th className="px-4 py-3 text-left font-medium text-[var(--on-surface-variant)]">Stored Token Ref</th>
                      <td className="px-4 py-3 font-mono text-[13px] text-[var(--on-surface)]">{maskTokenReference(configuredProvider?.token_ref ?? null)}</td>
                    </tr>
                  </tbody>
                </table>
              </div>
              <p className="mt-3 text-xs text-[var(--on-surface-variant)]">For security, raw API tokens are never retrievable after save.</p>
            </Card>
          </div>

          <Card title="Update API Token" description="Paste a new DigitalOcean Personal Access Token.">
            <div className="space-y-5">
              <Field label="Personal Access Token">
                <div className="relative">
                  <input value={apiToken} onChange={(event) => setApiToken(event.target.value)} placeholder="dop_v1_..." className="w-full rounded-xl border border-[color:var(--outline-variant)] bg-[color:rgba(8,10,16,0.9)] px-3 py-2.5 pr-10 text-[var(--on-surface)] outline-none ring-0 placeholder:text-[var(--on-surface-variant)]/50 focus:border-[color:rgba(171,199,255,0.45)]" />
                  <span className="pointer-events-none absolute right-3 top-1/2 -translate-y-1/2 text-[var(--on-surface-variant)]"><AppIcon name="search" className="h-4 w-4" /></span>
                </div>
              </Field>
              <div className="flex items-center justify-between text-xs text-[var(--on-surface-variant)]">
                <a href="#" className="text-[var(--primary)]">How to create a DO token</a>
                <span>{providerFeedback}</span>
              </div>
              <div className="flex flex-wrap items-center justify-between gap-3 pt-1">
                <button type="button" onClick={() => onNavigateToView("dashboard")} className="text-sm text-[var(--on-surface)]">Cancel</button>
                <div className="flex items-center gap-2">
                  <Button tone="secondary" onClick={() => void onSaveToken()} disabled={busyAction !== null || !apiToken.trim()}>
                    {busyAction === "save-token" ? "Saving" : "Save Token"}
                  </Button>
                  <Button onClick={() => void onValidateToken()} disabled={busyAction !== null || !(configuredProvider?.token_ref || apiToken.trim())}>
                    {busyAction === "validate-token" ? "Validating" : "Validate Token"} <span aria-hidden="true">→</span>
                  </Button>
                </div>
              </div>
            </div>
          </Card>
        </div>
        <div className="text-center text-xs text-[var(--on-surface-variant)]">Tokens are encrypted at rest, used only for DigitalOcean API calls from this device, and never transmitted to GoatStare services.</div>
      </div>
    </PageChrome>
  );
}
