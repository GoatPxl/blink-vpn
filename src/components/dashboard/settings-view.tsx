"use client";

import { useEffect, useMemo, useRef, useState } from "react";
import {
  Button,
  Panel,
  Field,
  Modal,
  PageChrome,
  Toggle,
  useNow,
  type DashboardBusyAction,
  type ProviderRecord,
  type VpnNodeRecord,
} from "./shared";
import {
  visibleProviders as filterVisibleProviders,
  PROVIDER_KEY_STATUS_LABEL,
  buildRowActions,
  countAttachedNodes,
  deriveProviderKeyStatus,
  describeKeepalive,
  MAX_DEVICE_NAME_CHARS,
  MAX_MOBILE_DEVICES,
  validateMobileDevices,
  describeKeyState,
  validateDnsServer,
  validateKeepalive,
  type ProviderKeyStatus,
} from "@/lib/settings-display";
import { listProviderCredentialFields, type AppPreferences, type CredentialField, type TunnelSettings, type WireGuardReadiness } from "@/lib/tauri";

const STATUS_CLASSES: Record<ProviderKeyStatus, string> = {
  verified: "border-[color:rgba(78,222,163,0.25)] bg-[color:rgba(78,222,163,0.1)] text-[var(--secondary)]",
  unverified: "border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.04)] text-[var(--on-surface-variant)]",
  missing: "border-[color:rgba(255,185,95,0.25)] bg-[color:rgba(255,185,95,0.08)] text-[var(--tertiary)]",
  suspended: "border-[color:rgba(255,185,95,0.25)] bg-[color:rgba(255,185,95,0.08)] text-[var(--tertiary)]",
  invalid: "border-[color:rgba(255,155,155,0.35)] bg-[color:rgba(255,155,155,0.08)] text-[#ff9b9b]",
};

export function SettingsView({
  dockerDevEnabled,
  providers,
  nodes,
  validatingProviderId,
  onSaveKey,
  onRevalidate,
  onSuspendProvider,
  onResumeProvider,
  onDeleteProvider,
  tunnelSettings,
  onSaveTunnelSettings,
  tunnelConnected,
  onReconnect,
  readiness,
  busyAction,
  reconnecting,
  appPreferences,
  onSaveAppPreferences,
}: {
  dockerDevEnabled: boolean;
  providers: ProviderRecord[];
  nodes: VpnNodeRecord[];
  validatingProviderId: string | null;
  onSaveKey: (providerId: string, credentials: Record<string, string>) => Promise<boolean>;
  onRevalidate: (provider: ProviderRecord) => Promise<void>;
  onSuspendProvider: (provider: ProviderRecord) => Promise<void>;
  onResumeProvider: (provider: ProviderRecord) => Promise<void>;
  onDeleteProvider: (provider: ProviderRecord) => Promise<void>;
  tunnelSettings: TunnelSettings | null;
  onSaveTunnelSettings: (settings: TunnelSettings) => Promise<boolean>;
  tunnelConnected: boolean;
  onReconnect: (() => Promise<void>) | null;
  readiness: WireGuardReadiness | null;
  busyAction: DashboardBusyAction;
  /** A reconnect started from this page is still in flight. */
  reconnecting: boolean;
  appPreferences: AppPreferences | null;
  onSaveAppPreferences: (preferences: AppPreferences) => Promise<boolean>;
}) {
  const [keyModalProviderId, setKeyModalProviderId] = useState<string | null>(null);
  const [openMenuProviderId, setOpenMenuProviderId] = useState<string | null>(null);

  // Docker's credential is a local dev stand-in; without the flag there is nothing to configure,
  // and offering the row led to a Save button that silently did nothing.
  const visibleProviders = useMemo(
    () => filterVisibleProviders(providers, dockerDevEnabled),
    [dockerDevEnabled, providers],
  );

  const modalProvider = providers.find((provider) => provider.id === keyModalProviderId) ?? null;

  return (
    <PageChrome>
      <Panel
        title="Providers"
        description="Cloud accounts this app can provision servers on. Keys stay on this device."
        action={
          <Button
            onClick={() => setKeyModalProviderId(visibleProviders[0]?.id ?? null)}
            disabled={busyAction !== null || visibleProviders.length === 0}
          >
            Add API key
          </Button>
        }
      >
        <div className="overflow-visible">
          <table className="min-w-full text-left text-sm">
            <thead className="text-[11px] uppercase tracking-[0.24em] text-[var(--on-surface-variant)]">
              <tr>
                <th className="px-4 py-3">Provider</th>
                <th className="px-4 py-3">Status</th>
                <th className="px-4 py-3">Servers</th>
                <th className="px-4 py-3 text-right">Actions</th>
              </tr>
            </thead>
            <tbody>
              {visibleProviders.map((provider) => {
                const hasKey = !!provider.token_ref;
                const attachedNodeCount = countAttachedNodes(nodes, provider.id);
                const status = deriveProviderKeyStatus({ provider, hasKey });
                const busy = busyAction !== null;
                const checkingThisRow = validatingProviderId === provider.id;
                const actions = buildRowActions({ provider, hasKey, attachedNodeCount, busy });

                return (
                  <tr key={provider.id} className="border-t border-[color:var(--outline-variant)]">
                    <td className="px-4 py-4 align-middle">
                      <div className="font-medium text-[var(--on-surface)]">{provider.name}</div>
                      {/* Says a key exists, and when it was last checked, without printing a
                          masked reference that looks like part of the secret but is only an
                          opaque handle. */}
                      <div className="mt-0.5 text-xs text-[var(--on-surface-variant)]">
                        <KeyState provider={provider} />
                      </div>
                    </td>
                    <td className="px-4 py-4 align-middle">
                      <span className={`inline-flex rounded-full border px-2.5 py-1 text-[10px] uppercase tracking-[0.2em] ${STATUS_CLASSES[status]}`}>
                        {PROVIDER_KEY_STATUS_LABEL[status]}
                      </span>
                    </td>
                    <td className="px-4 py-4 align-middle text-[var(--on-surface)]">{attachedNodeCount}</td>
                    <td className="px-4 py-4 align-middle">
                      <div className="flex items-center justify-end gap-2">
                        <Button
                          tone="ghost"
                          onClick={() => void onRevalidate(provider)}
                          disabled={busy || !hasKey}
                          className="px-3 py-2 text-xs"
                        >
                          {/* Only the row being checked says so; busyAction is global. */}
                          {checkingThisRow ? "Checking" : "Re-check"}
                        </Button>
                        <RowMenu
                          open={openMenuProviderId === provider.id}
                          onToggle={() => setOpenMenuProviderId((current) => (current === provider.id ? null : provider.id))}
                          onClose={() => setOpenMenuProviderId(null)}
                          label={`Actions for ${provider.name}`}
                          actions={actions}
                          onSelect={(id) => {
                            setOpenMenuProviderId(null);
                            if (id === "edit") setKeyModalProviderId(provider.id);
                            if (id === "suspend") {
                              void (provider.status === "suspended" ? onResumeProvider(provider) : onSuspendProvider(provider));
                            }
                            if (id === "delete") void onDeleteProvider(provider);
                          }}
                        />
                      </div>
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      </Panel>

      {tunnelSettings ? (
        <WireGuardSettingsCard
          settings={tunnelSettings}
          onSave={onSaveTunnelSettings}
          tunnelConnected={tunnelConnected}
          onReconnect={onReconnect}
          busyAction={busyAction}
          reconnecting={reconnecting}
        />
      ) : (
        <Panel title="WireGuard" description="Loading tunnel settings…">
          <div className="h-16" />
        </Panel>
      )}

      {appPreferences ? (
        <MobileDevicesSettingsCard
          preferences={appPreferences}
          onSave={onSaveAppPreferences}
          busyAction={busyAction}
        />
      ) : (
        <Panel title="Mobile devices" description="Loading saved devices…">
          <div className="h-16" />
        </Panel>
      )}

      <Panel title="Connecting" description="What happens around a connect.">
        <div className="divide-y-2 divide-line-2">
          <PreferenceToggle
            title="Connect when a new server is ready"
            detail="A server you build connects by itself the moment it is ready. Windows still asks for permission once, exactly as when you click Connect."
            checked={appPreferences?.auto_connect_after_provision ?? true}
            disabled={appPreferences === null || busyAction !== null}
            onChange={(checked) => {
              if (appPreferences) void onSaveAppPreferences({ ...appPreferences, auto_connect_after_provision: checked });
            }}
          />
          <PreferenceToggle
            title="Explain before connecting"
            detail="Before each connect, say that Windows will ask for permission and that the internet pauses for about 15 seconds."
            checked={appPreferences?.explain_uac ?? true}
            disabled={appPreferences === null || busyAction !== null}
            onChange={(checked) => {
              if (appPreferences) void onSaveAppPreferences({ ...appPreferences, explain_uac: checked });
            }}
          />
        </div>
      </Panel>

      <Panel title="Window" description="How Blink behaves when you close its window.">
        <div className="flex items-start justify-between gap-6">
          <div>
            <div className="text-sm font-medium text-[var(--on-surface)]">Close to system tray</div>
            <p className="mt-1 text-sm text-[var(--on-surface-variant)]">
              Closing the window keeps Blink running in the notification area. Open it again from the
              tray icon; choose Quit there to exit. A connected tunnel stays up either way.
            </p>
          </div>
          <Toggle
            label="Close to system tray"
            checked={appPreferences?.close_to_tray ?? true}
            disabled={appPreferences === null || busyAction !== null}
            onChange={(checked) => {
              if (appPreferences) {
                void onSaveAppPreferences({ ...appPreferences, close_to_tray: checked });
              }
            }}
          />
        </div>
      </Panel>

      <Panel title="About" description="Versions of the app and the WireGuard runtime it drives.">
        <dl className="grid gap-3 sm:grid-cols-2">
          <div className="rounded-[10px] border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.02)] px-4 py-3">
            <dt className="text-[11px] uppercase tracking-[0.24em] text-[var(--on-surface-variant)]">Blink VPN</dt>
            <dd className="mt-1 font-mono text-sm text-[var(--on-surface)]">{readiness?.app_version ?? "unknown"}</dd>
          </div>
          <div className="rounded-[10px] border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.02)] px-4 py-3">
            <dt className="text-[11px] uppercase tracking-[0.24em] text-[var(--on-surface-variant)]">WireGuard</dt>
            <dd className="mt-1 font-mono text-sm text-[var(--on-surface)]">{readiness?.version ?? "not detected"}</dd>
          </div>
        </dl>
        {/* Names where the credential actually lives, which is the fact that lets someone go and
            revoke it. "Encrypted at rest" was true only by delegation and only on Windows. */}
        <p className="mt-4 text-xs leading-5 text-[var(--on-surface-variant)]">
          API keys are held in Windows Credential Manager under <span className="font-mono">app.blink.vpn</span>,
          never in this app&apos;s database. They are used only for calls to your provider from this
          device, and are stripped from diagnostics exports.
        </p>
      </Panel>

      {modalProvider ? (
        <KeyModal
          // Remount on provider change so the credential inputs reset with it. Switching provider
          // must not carry a half-typed value across to a different account's form.
          key={modalProvider.id}
          provider={modalProvider}
          providers={visibleProviders}
          onChangeProvider={setKeyModalProviderId}
          onClose={() => setKeyModalProviderId(null)}
          onSave={onSaveKey}
          busyAction={busyAction}
        />
      ) : null}
    </PageChrome>
  );
}

/** A title, one sentence, and a switch. */
function PreferenceToggle({
  title,
  detail,
  checked,
  disabled,
  onChange,
}: {
  title: string;
  detail: string;
  checked: boolean;
  disabled: boolean;
  onChange: (checked: boolean) => void;
}) {
  return (
    <div className="flex items-start justify-between gap-6 py-4 first:pt-0 last:pb-0">
      <div>
        <div className="text-[15px] font-medium text-bone">{title}</div>
        <p className="mt-1 text-[13.5px] text-stone">{detail}</p>
      </div>
      <Toggle label={title} checked={checked} disabled={disabled} onChange={onChange} />
    </div>
  );
}

/** Keeps the "checked N ago" line current without a re-render of the whole page. */
function KeyState({ provider }: { provider: ProviderRecord }) {
  const now = useNow();
  return <>{describeKeyState(provider, now)}</>;
}

function RowMenu({
  open,
  onToggle,
  onClose,
  label,
  actions,
  onSelect,
}: {
  open: boolean;
  onToggle: () => void;
  onClose: () => void;
  label: string;
  actions: ReturnType<typeof buildRowActions>;
  onSelect: (id: "edit" | "suspend" | "delete") => void;
}) {
  const containerRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (!open) return;

    const closeOnOutside = (event: MouseEvent) => {
      if (!containerRef.current?.contains(event.target as Node)) {
        onClose();
      }
    };
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };

    document.addEventListener("mousedown", closeOnOutside);
    document.addEventListener("keydown", closeOnEscape);
    return () => {
      document.removeEventListener("mousedown", closeOnOutside);
      document.removeEventListener("keydown", closeOnEscape);
    };
  }, [onClose, open]);

  return (
    <div className="relative" ref={containerRef}>
      <button
        type="button"
        aria-label={label}
        aria-expanded={open}
        aria-haspopup="menu"
        onClick={onToggle}
        className="rounded-lg px-2 py-2 text-[var(--on-surface-variant)] transition-colors hover:bg-[color:rgba(255,255,255,0.06)] hover:text-[var(--on-surface)]"
      >
        <svg viewBox="0 0 24 24" className="h-4 w-4" aria-hidden="true"><path d="M6 12h.01M12 12h.01M18 12h.01" className="fill-none stroke-current stroke-[2.5]" strokeLinecap="round" /></svg>
      </button>
      {open ? (
        <div role="menu" className="absolute right-0 top-full z-20 mt-1 w-[260px] overflow-hidden rounded-xl border border-[color:var(--outline-variant)] bg-[linear-gradient(180deg,rgba(26,30,38,0.99),rgba(16,19,27,0.99))] py-1 shadow-[0_20px_60px_rgba(0,0,0,0.45)]">
          {actions.map((action) => (
            <button
              key={action.id}
              type="button"
              role="menuitem"
              disabled={action.disabled}
              // The reason lives here rather than as a permanent line of prose in every row.
              title={action.disabledReason ?? undefined}
              onClick={() => onSelect(action.id)}
              className={`block w-full px-3 py-2 text-left text-sm transition-colors disabled:cursor-not-allowed disabled:opacity-45 ${action.danger ? "text-[#ff9b9b] hover:bg-[color:rgba(255,155,155,0.08)]" : "text-[var(--on-surface)] hover:bg-[color:rgba(255,255,255,0.06)]"}`}
            >
              {action.label}
            </button>
          ))}
        </div>
      ) : null}
    </div>
  );
}

function WireGuardSettingsCard({
  settings,
  onSave,
  tunnelConnected,
  onReconnect,
  busyAction,
  reconnecting,
}: {
  settings: TunnelSettings;
  onSave: (settings: TunnelSettings) => Promise<boolean>;
  tunnelConnected: boolean;
  onReconnect: (() => Promise<void>) | null;
  busyAction: DashboardBusyAction;
  /** A reconnect started here is still in flight. */
  reconnecting: boolean;
}) {
  // Initialised from the loaded settings on mount. The card is only rendered once they exist, so
  // there is no empty-then-populate step to synchronise, and a later refresh cannot overwrite
  // whatever the user is midway through typing.
  const [dns, setDns] = useState(settings.dns_server ?? "");
  const [keepalive, setKeepalive] = useState(String(settings.keepalive_seconds));
  const [dirty, setDirty] = useState(false);
  const [saved, setSaved] = useState(false);

  const dnsValidation = validateDnsServer(dns);
  const keepaliveValidation = validateKeepalive(keepalive);
  const canSave = dnsValidation.valid && keepaliveValidation.valid && dirty && busyAction === null;

  return (
    <Panel
      title="WireGuard"
      description="Applied the next time a tunnel starts. A running tunnel keeps the settings it started with."
    >
      <div className="grid gap-4 md:grid-cols-2">
        <Field label="DNS server">
          <input
            value={dns}
            onChange={(event) => { setDns(event.target.value); setDirty(true); setSaved(false); }}
            placeholder="1.1.1.1"
            spellCheck={false}
            className="w-full rounded-xl border border-[color:var(--outline-variant)] bg-[color:rgba(8,10,16,0.9)] px-3 py-2.5 font-mono text-[var(--on-surface)] outline-none"
          />
          <p className={`mt-1 text-xs ${dnsValidation.valid ? "text-[var(--on-surface-variant)]" : "text-[#ff9b9b]"}`}>
            {dnsValidation.valid
              ? (dns.trim() === "" ? "Empty uses your system DNS, which can leak lookups outside the tunnel." : "Resolver used while connected.")
              : dnsValidation.message}
          </p>
        </Field>

        <Field label="Persistent keepalive (seconds)">
          <input
            value={keepalive}
            onChange={(event) => { setKeepalive(event.target.value); setDirty(true); setSaved(false); }}
            inputMode="numeric"
            placeholder="25"
            className="w-full rounded-xl border border-[color:var(--outline-variant)] bg-[color:rgba(8,10,16,0.9)] px-3 py-2.5 font-mono text-[var(--on-surface)] outline-none"
          />
          <p className={`mt-1 text-xs ${keepaliveValidation.valid ? "text-[var(--on-surface-variant)]" : "text-[#ff9b9b]"}`}>
            {keepaliveValidation.valid ? describeKeepalive(Number(keepalive || 0)) : keepaliveValidation.message}
          </p>
        </Field>
      </div>

      <div className="mt-4 flex flex-wrap items-center justify-end gap-2">
        {saved && tunnelConnected && onReconnect ? (
          <>
            <span className="mr-auto text-xs text-[var(--tertiary)]">Reconnect for these to take effect.</span>
            <Button
              tone="ghost"
              onClick={() => void onReconnect()}
              disabled={busyAction !== null || reconnecting}
            >
              {reconnecting ? "Reconnecting" : "Reconnect now"}
            </Button>
          </>
        ) : null}
        <Button
          onClick={() => {
            void (async () => {
              const stored = await onSave({
                dns_server: dns.trim() === "" ? null : dns.trim(),
                keepalive_seconds: Number(keepalive),
              });
              if (stored) {
                setDirty(false);
                setSaved(true);
              }
            })();
          }}
          disabled={!canSave}
        >
          {busyAction === "save-tunnel-settings" ? "Saving" : "Save changes"}
        </Button>
      </div>
    </Panel>
  );
}

/**
 * The devices every new server should come with.
 *
 * Saved names are given to a new server's spare peer slots when it is created, so its QR codes
 * are on the dashboard the moment it is ready. Slots are fixed at creation (AGENTS.md §12), so a
 * change here reaches only servers created afterwards.
 */
function MobileDevicesSettingsCard({
  preferences,
  onSave,
  busyAction,
}: {
  preferences: AppPreferences;
  onSave: (preferences: AppPreferences) => Promise<boolean>;
  busyAction: DashboardBusyAction;
}) {
  // Initialised once from the loaded preferences, like the WireGuard card, so a refresh cannot
  // overwrite a list the user is editing.
  const [names, setNames] = useState<string[]>(preferences.mobile_devices);
  const [dirty, setDirty] = useState(false);
  const [saved, setSaved] = useState(false);

  const validation = validateMobileDevices(names);
  const canSave = validation.valid && dirty && busyAction === null;

  function update(next: string[]) {
    setNames(next);
    setDirty(true);
    setSaved(false);
  }

  return (
    <Panel
      title="Mobile devices"
      description="New servers come with these devices already set up, with QR codes ready on the dashboard. Servers you already have keep their current devices."
    >
      {names.length === 0 ? (
        <p className="text-sm text-[var(--on-surface-variant)]">
          No devices saved. Add the phones and tablets you use with the WireGuard app.
        </p>
      ) : (
        <ul className="space-y-2">
          {names.map((name, index) => (
            <li key={index} className="flex items-center gap-2">
              <input
                value={name}
                maxLength={MAX_DEVICE_NAME_CHARS}
                placeholder="Pixel 9"
                aria-label={`Device ${index + 1} name`}
                onChange={(event) => update(names.map((current, at) => (at === index ? event.target.value : current)))}
                className="w-full max-w-sm rounded-xl border border-[color:var(--outline-variant)] bg-[color:rgba(8,10,16,0.9)] px-3 py-2 text-sm text-[var(--on-surface)] outline-none"
              />
              <Button
                tone="ghost"
                className="px-3 py-2 text-xs"
                onClick={() => update(names.filter((_, at) => at !== index))}
              >
                Remove
              </Button>
            </li>
          ))}
        </ul>
      )}

      {!validation.valid ? (
        <p className="mt-2 text-xs text-[#ff9b9b]">{validation.message}</p>
      ) : null}

      <div className="mt-4 flex flex-wrap items-center justify-end gap-2">
        {saved ? (
          <span className="mr-auto text-xs text-[var(--on-surface-variant)]">
            Saved. Your next server will include these devices.
          </span>
        ) : null}
        <Button
          tone="ghost"
          disabled={names.length >= MAX_MOBILE_DEVICES}
          onClick={() => update([...names, ""])}
        >
          Add device
        </Button>
        <Button
          disabled={!canSave}
          onClick={() => {
            void (async () => {
              const stored = await onSave({ ...preferences, mobile_devices: names.map((name) => name.trim()) });
              if (stored) {
                setNames((current) => current.map((name) => name.trim()));
                setDirty(false);
                setSaved(true);
              }
            })();
          }}
        >
          {busyAction === "save-app-preferences" ? "Saving" : "Save devices"}
        </Button>
      </div>
    </Panel>
  );
}

function KeyModal({
  provider,
  providers,
  onChangeProvider,
  onClose,
  onSave,
  busyAction,
}: {
  provider: ProviderRecord;
  providers: ProviderRecord[];
  onChangeProvider: (providerId: string) => void;
  onClose: () => void;
  onSave: (providerId: string, credentials: Record<string, string>) => Promise<boolean>;
  busyAction: DashboardBusyAction;
}) {
  // The fields come from the provider's adapter rather than from a slug comparison here, so a
  // provider needing two of them (an AWS access key id and its secret) needs no change in this file.
  const [fields, setFields] = useState<CredentialField[] | null>(null);
  const [fieldsError, setFieldsError] = useState<string | null>(null);
  const [values, setValues] = useState<Record<string, string>>({});
  const [revealed, setRevealed] = useState<Record<string, boolean>>({});
  const busy = busyAction !== null;

  // The modal is remounted per provider (see its `key`), so there is no stale state to clear here.
  useEffect(() => {
    let cancelled = false;

    void (async () => {
      try {
        const loaded = await listProviderCredentialFields(provider.id);
        if (!cancelled) setFields(loaded);
      } catch (error) {
        if (!cancelled) {
          setFieldsError(error instanceof Error ? error.message : "Could not load this provider's credential fields.");
        }
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [provider.id]);

  const allFilled = (fields ?? []).length > 0 && (fields ?? []).every((field) => (values[field.key] ?? "").trim() !== "");

  return (
    <Modal onClose={onClose} labelledBy="credential-modal-title">
      <>
        <h2 id="credential-modal-title" className="text-xl font-semibold tracking-tight text-[var(--on-surface)]">
          {provider.token_ref ? `Replace ${provider.name} key` : `Add ${provider.name} key`}
        </h2>
        <p className="mt-1 text-sm text-[var(--on-surface-variant)]">
          Saved to Windows Credential Manager and checked against the provider straight away.
        </p>

        <div className="mt-5 space-y-4">
          {providers.length > 1 ? (
            <Field label="Provider">
              {/* Changing this must not touch the app-wide selected provider: browsing the list
                  used to reassign it, and Cancel did not put it back. */}
              <select
                value={provider.id}
                onChange={(event) => onChangeProvider(event.target.value)}
                disabled={busy}
                className="w-full rounded-xl border border-[color:var(--outline-variant)] bg-[color:rgba(8,10,16,0.9)] px-3 py-2.5 text-[var(--on-surface)] outline-none"
              >
                {providers.map((entry) => <option key={entry.id} value={entry.id}>{entry.name}</option>)}
              </select>
            </Field>
          ) : null}

          {fieldsError ? (
            <p className="text-sm text-[var(--error)]">{fieldsError}</p>
          ) : fields === null ? (
            <p className="text-sm text-[var(--on-surface-variant)]">Loading credential fields…</p>
          ) : (
            fields.map((field) => {
              const reveal = revealed[field.key] ?? false;
              return (
                <Field key={field.key} label={field.label}>
                  <div className="flex gap-2">
                    <input
                      // A credential value is one line and should not sit on screen in plaintext;
                      // this was a full-height textarea.
                      type={field.secret && !reveal ? "password" : "text"}
                      value={values[field.key] ?? ""}
                      onChange={(event) =>
                        setValues((current) => ({ ...current, [field.key]: event.target.value }))
                      }
                      placeholder={field.placeholder}
                      autoComplete="off"
                      spellCheck={false}
                      disabled={busy}
                      className="w-full rounded-xl border border-[color:var(--outline-variant)] bg-[color:rgba(8,10,16,0.9)] px-3 py-2.5 font-mono text-[var(--on-surface)] outline-none"
                    />
                    {field.secret ? (
                      <Button
                        tone="ghost"
                        onClick={() => setRevealed((current) => ({ ...current, [field.key]: !reveal }))}
                        className="px-3 py-2 text-xs"
                      >
                        {reveal ? "Hide" : "Show"}
                      </Button>
                    ) : null}
                  </div>
                  {field.help ? (
                    <p className="mt-1 text-xs text-[var(--on-surface-variant)]">{field.help}</p>
                  ) : null}
                </Field>
              );
            })
          )}
        </div>

        <div className="mt-6 flex items-center justify-end gap-2">
          <Button tone="ghost" onClick={onClose} disabled={busy}>Cancel</Button>
          <Button
            onClick={() => {
              void (async () => {
                const trimmed = Object.fromEntries(
                  (fields ?? []).map((field) => [field.key, (values[field.key] ?? "").trim()]),
                );
                const stored = await onSave(provider.id, trimmed);
                // Only clear on a save that actually stored something — a failed save used to wipe
                // what was typed.
                if (stored) onClose();
              })();
            }}
            disabled={busy || !allFilled}
          >
            {busyAction === "save-token" ? "Saving" : busyAction === "validate-token" ? "Checking" : "Save & validate"}
          </Button>
        </div>
      </>
    </Modal>
  );
}
