"use client";

import { useRouter } from "next/navigation";
import { useCallback, useEffect, useState } from "react";

import { Button, INPUT_CLASS, Panel, Tag, Tile } from "@/components/dashboard/shared";
import { GoatMark, PixelIcon } from "@/components/ui/pixel-icon";
import { formatHourly, regionCode } from "@/lib/home-display";
import { buildMapRegions, type MapRegion } from "@/lib/map/map-regions";
import { loadProviderPricingCatalog } from "@/lib/provider-pricing-cache";
import { hasUsableKey, visibleProviders } from "@/lib/settings-display";
import { localTimeZone, suggestRegion } from "@/lib/suggest-region";
import {
  checkWireGuardReadiness,
  installWireGuardWithPrerequisites,
  listProviderCredentialFields,
  listProviders,
  openProviderTokenPage,
  openWireGuardInstallPage,
  registerProviderCredential,
  toCommandError,
  validateProviderToken,
  type CredentialField,
  type ProviderRecord,
  type WireGuardReadiness,
} from "@/lib/tauri";
import { buildHandoffUrl } from "./build-handoff";

type KeyStatus =
  | { kind: "idle" }
  | { kind: "checking" }
  | { kind: "verified" }
  | { kind: "failed"; message: string };

/** "DO" for DigitalOcean, "EC2" for Amazon EC2, "GCE" for Google Compute Engine — from the name. */
function providerCode(name: string): string {
  const words = name.split(/\s+/).filter(Boolean);
  const last = words[words.length - 1] ?? name;
  if (words.length > 1 && /^[A-Z0-9]{2,4}$/.test(last)) return last;
  const initials = words.length > 1 ? words.map((word) => word[0]).join("") : (name.match(/[A-Z]/g) ?? []).join("");
  return (initials.length >= 2 ? initials : name.slice(0, 2)).toUpperCase().slice(0, 3);
}

function isReady(readiness: WireGuardReadiness | null): boolean {
  return !!readiness && readiness.installed && readiness.executable_found && readiness.service_available;
}

function StepIndicator({ step }: { step: 1 | 2 | 3 }) {
  return (
    <div className="flex items-center gap-3" aria-label={`Step ${step} of 3`}>
      {[1, 2, 3].map((index) => (
        <span
          key={index}
          className={`flex h-11 w-11 items-center justify-center border-2 font-mono text-[15px] ${index < step ? "border-lime bg-lime text-ink" : index === step ? "border-lime text-lime" : "border-line-3 text-stone"}`}
        >
          {index < step ? <PixelIcon name="check" className="h-4 w-4" label="done" /> : index}
        </span>
      ))}
      <span className="ml-2 font-display text-[12px] uppercase text-stone">key → build → connect</span>
    </div>
  );
}

/**
 * First run: one key, one suggested city, one button.
 *
 * WireGuard readiness is checked on arrival and only becomes a screen if it fails. The build itself
 * is handed to the dashboard, which owns the build screen and the auto-connect — the wizard used to
 * run a second, parallel provisioning loop of its own.
 */
export function OnboardingWizard() {
  const router = useRouter();
  const dockerDevEnabled = process.env.NEXT_PUBLIC_DEV_DOCKER === "1";

  const [readiness, setReadiness] = useState<WireGuardReadiness | null>(null);
  const [readinessChecked, setReadinessChecked] = useState(false);
  const [installMessage, setInstallMessage] = useState<string | null>(null);
  const [installing, setInstalling] = useState(false);

  const [providers, setProviders] = useState<ProviderRecord[]>([]);
  const [providerId, setProviderId] = useState<string | null>(null);
  const [fields, setFields] = useState<CredentialField[]>([]);
  const [values, setValues] = useState<Record<string, string>>({});
  const [status, setStatus] = useState<KeyStatus>({ kind: "idle" });

  const [suggestion, setSuggestion] = useState<MapRegion | null>(null);
  const [suggestionError, setSuggestionError] = useState<string | null>(null);

  const refreshReadiness = useCallback(async () => {
    try {
      setReadiness(await checkWireGuardReadiness());
    } catch {
      setReadiness(null);
    } finally {
      setReadinessChecked(true);
    }
  }, []);

  useEffect(() => {
    const timer = window.setTimeout(() => void refreshReadiness(), 0);
    return () => window.clearTimeout(timer);
  }, [refreshReadiness]);

  // Providers once; preselect one that already has a working key, else the first.
  useEffect(() => {
    let active = true;
    void listProviders().then((loaded) => {
      if (!active) return;
      const offered = visibleProviders(loaded, dockerDevEnabled);
      setProviders(offered);
      const preferred = offered.find(hasUsableKey) ?? offered[0] ?? null;
      setProviderId(preferred?.id ?? null);
      if (preferred && hasUsableKey(preferred)) setStatus({ kind: "verified" });
    }).catch((error) => {
      if (active) setStatus({ kind: "failed", message: toCommandError(error).message });
    });
    return () => {
      active = false;
    };
  }, [dockerDevEnabled]);

  const provider = providers.find((candidate) => candidate.id === providerId) ?? null;

  // The adapter declares its inputs, so one token or a key and secret render without branching here.
  useEffect(() => {
    if (!providerId) return;
    let active = true;
    void listProviderCredentialFields(providerId)
      .then((loaded) => {
        if (active) {
          setFields(loaded);
          setValues({});
        }
      })
      .catch(() => {
        if (active) setFields([]);
      });
    return () => {
      active = false;
    };
  }, [providerId]);

  // A verified key earns a suggestion: nearest continent, cheapest size, no network beyond the provider.
  useEffect(() => {
    if (status.kind !== "verified" || !provider) return;
    let active = true;
    void loadProviderPricingCatalog(provider.id)
      .then((catalog) => {
        if (!active) return;
        const regions = buildMapRegions({
          providers: [{ id: provider.id, name: provider.name, verified: true }],
          regionsByProvider: { [provider.id]: catalog.regions },
          pricingByProvider: { [provider.id]: catalog.pricingByRegion },
        });
        setSuggestion(suggestRegion(regions, localTimeZone()));
        setSuggestionError(null);
      })
      .catch((error) => {
        if (active) setSuggestionError(`${toCommandError(error).message} ${toCommandError(error).nextAction}`);
      });
    return () => {
      active = false;
    };
  }, [provider, status.kind]);

  const tokenField = fields.find((field) => field.token_page_url) ?? fields[0] ?? null;
  const allFilled = fields.length > 0 && fields.every((field) => (values[field.key] ?? "").trim().length > 0);

  const saveAndValidate = useCallback(async (nextValues: Record<string, string>) => {
    if (!provider || fields.length === 0) return;
    if (!fields.every((field) => (nextValues[field.key] ?? "").trim().length > 0)) return;
    setStatus({ kind: "checking" });
    try {
      await registerProviderCredential(
        provider.id,
        provider.name,
        Object.fromEntries(fields.map((field) => [field.key, (nextValues[field.key] ?? "").trim()])),
      );
      await validateProviderToken(provider.id);
      setValues({});
      setStatus({ kind: "verified" });
      setProviders((current) => current.map((candidate) => (
        candidate.id === provider.id ? { ...candidate, token_ref: candidate.token_ref ?? "stored", last_validation_status: "ok" } : candidate
      )));
    } catch (error) {
      // Displayed, never parsed: the kind decides nothing here but the words the user reads.
      const commandError = toCommandError(error);
      setStatus({ kind: "failed", message: `${commandError.message} ${commandError.nextAction}` });
    }
  }, [fields, provider]);

  async function pasteInto(field: CredentialField) {
    try {
      const text = (await navigator.clipboard.readText()).trim();
      if (!text) return;
      const next = { ...values, [field.key]: text };
      setValues(next);
      await saveAndValidate(next);
    } catch {
      setStatus({ kind: "failed", message: "Blink could not read the clipboard. Click the box and press Ctrl+V instead." });
    }
  }

  function chooseProvider(id: string) {
    setProviderId(id);
    setSuggestion(null);
    const chosen = providers.find((candidate) => candidate.id === id);
    setStatus(chosen && hasUsableKey(chosen) ? { kind: "verified" } : { kind: "idle" });
  }

  const step: 1 | 2 | 3 = !isReady(readiness) ? 1 : status.kind === "verified" ? 3 : 2;

  return (
    <div className="mx-auto flex w-full max-w-[1440px] flex-col gap-8 px-[clamp(20px,6vw,120px)] py-10">
      <header className="flex flex-wrap items-center justify-between gap-6">
        <div className="flex items-center gap-5">
          <GoatMark className="h-16 w-14 text-lime" />
          <div>
            <div className="font-display text-[22px] uppercase text-bone">Blink VPN</div>
            <p className="text-[15px] text-stone">Your server. Your key. Gone when you&apos;re done.</p>
          </div>
        </div>
        <StepIndicator step={step} />
      </header>

      {readinessChecked && !isReady(readiness) ? (
        <Panel tint="busy" bodyClassName="flex flex-col gap-5 p-8">
          <Tag state="busy">Step 1 · WireGuard for Windows</Tag>
          <h1 className="font-display text-[clamp(32px,4vw,52px)] uppercase leading-none text-bone">Install WireGuard first</h1>
          <p className="max-w-[720px] text-[16px] text-stone">
            Blink runs your tunnel through the official WireGuard service. {readiness?.message ?? ""} {readiness?.action_guidance ?? ""}
          </p>
          {installMessage ? <p className="text-[14px] text-bone">{installMessage}</p> : null}
          <div className="flex flex-wrap gap-3">
            <Button
              disabled={installing}
              onClick={async () => {
                setInstalling(true);
                setInstallMessage(null);
                try {
                  const result = await installWireGuardWithPrerequisites();
                  setInstallMessage(`${result.message} ${result.next_action}`);
                } catch (error) {
                  setInstallMessage(`${toCommandError(error).message} ${toCommandError(error).nextAction}`);
                } finally {
                  setInstalling(false);
                  await refreshReadiness();
                }
              }}
            >
              {installing ? "Installing" : "Install WireGuard"}
            </Button>
            <Button tone="ghost" onClick={() => void openWireGuardInstallPage().catch(() => undefined)}>Download page</Button>
            <Button tone="quiet" onClick={() => void refreshReadiness()}>Check again</Button>
          </div>
        </Panel>
      ) : null}

      <div className="grid grid-cols-[minmax(0,1.3fr)_minmax(0,1fr)] gap-8 max-[1100px]:grid-cols-1">
        <section className="flex flex-col gap-6">
          <div>
            <Tag state="secure">Step 2 · One key from a cloud you already pay</Tag>
            <h1 className="mt-3 font-display text-[clamp(36px,4.5vw,56px)] uppercase leading-none text-bone">Add a cloud key</h1>
          </div>

          <div className="grid grid-cols-3 gap-4 max-[700px]:grid-cols-2">
            {providers.map((candidate) => (
              <Tile
                key={candidate.id}
                code={providerCode(candidate.name)}
                name={candidate.name}
                meta={hasUsableKey(candidate) ? "key saved" : candidate.id === providerId ? "selected" : "add a key"}
                selected={candidate.id === providerId}
                onClick={() => chooseProvider(candidate.id)}
              />
            ))}
            <Tile
              dashed
              code="?"
              name="No cloud yet"
              meta="Open an account with any of these first. DigitalOcean takes about 2 minutes."
              disabled
            />
          </div>

          {provider && status.kind !== "verified" ? (
            <Panel bodyClassName="flex flex-col gap-5 p-7">
              <div className="flex flex-wrap items-start gap-5">
                <span className="flex h-11 w-11 shrink-0 items-center justify-center border-2 border-lime font-mono text-lime">1</span>
                <div className="min-w-0 flex-1">
                  <p className="text-[17px] text-bone">
                    Open {provider.name}&apos;s key page and create a new key.
                    {tokenField?.scope_hint ? <> Give it: <span className="font-semibold">{tokenField.scope_hint}</span></> : null}
                  </p>
                  {tokenField?.help ? <p className="mt-1 text-[13.5px] text-stone">{tokenField.help}</p> : null}
                </div>
                {tokenField?.token_page_url ? (
                  <Button tone="ghost" onClick={() => void openProviderTokenPage(provider.id).catch(() => undefined)}>
                    Open key page <PixelIcon name="arrow-right" className="h-3 w-3 -rotate-45" />
                  </Button>
                ) : null}
              </div>
              <div className="flex items-start gap-5">
                <span className="flex h-11 w-11 shrink-0 items-center justify-center border-2 border-lime font-mono text-lime">2</span>
                <p className="pt-2 text-[17px] text-bone">Copy it, come back, paste.</p>
              </div>
              {fields.map((field) => (
                <div key={field.key} className="flex gap-3">
                  <input
                    type={field.secret ? "password" : "text"}
                    aria-label={field.label}
                    placeholder={field.placeholder}
                    value={values[field.key] ?? ""}
                    autoComplete="off"
                    spellCheck={false}
                    disabled={status.kind === "checking"}
                    onChange={(event) => setValues((current) => ({ ...current, [field.key]: event.target.value }))}
                    onBlur={() => {
                      if (allFilled) void saveAndValidate(values);
                    }}
                    className={`${INPUT_CLASS} h-14`}
                  />
                  <Button tone="ghost" className="h-14" disabled={status.kind === "checking"} onClick={() => void pasteInto(field)}>
                    Paste
                  </Button>
                </div>
              ))}
              <div className="flex flex-wrap items-center justify-between gap-4">
                <p className="text-[13px] text-stone">
                  Saved to Windows Credential Manager. Never in Blink&apos;s database, logs, or bug reports. One read-only call checks it works.
                </p>
                {fields.length > 1 ? (
                  <Button disabled={!allFilled || status.kind === "checking"} onClick={() => void saveAndValidate(values)}>
                    Save &amp; check
                  </Button>
                ) : null}
              </div>
              {status.kind === "checking" ? <p className="text-[14px] text-ember">Checking the key with {provider.name}…</p> : null}
              {status.kind === "failed" ? <p role="alert" className="text-[14px] text-alert">{status.message}</p> : null}
            </Panel>
          ) : provider ? (
            <Panel bodyClassName="flex flex-wrap items-center justify-between gap-4 px-7 py-5">
              <span className="flex items-center gap-3 text-[16px] text-lime">
                <PixelIcon name="check" className="h-5 w-5" /> {provider.name} key saved and working.
              </span>
              <Button tone="quiet" size="sm" onClick={() => setStatus({ kind: "idle" })}>Replace key</Button>
            </Panel>
          ) : null}
        </section>

        <Panel tint={step === 3 ? "secure" : "none"} className={step === 3 ? "" : "opacity-50"} bodyClassName="flex flex-col gap-5 p-8">
          <Tag state={step === 3 ? "secure" : undefined}>Step 3 · Your first server (suggested)</Tag>
          {step !== 3 ? (
            <p className="text-[16px] text-stone">Add a working key and Blink picks a city near you.</p>
          ) : suggestion ? (
            <>
              <div className="font-display text-[clamp(44px,5vw,64px)] uppercase leading-none text-lime">{regionCode(suggestion.slug)}</div>
              <div className="text-[clamp(20px,2vw,28px)] font-semibold text-bone">{suggestion.city ? `${suggestion.city}${suggestion.countryCode ? `, ${suggestion.countryCode}` : ""}` : suggestion.slug}</div>
              <p className="text-[15px] text-stone">Nearest to your time zone. Cheapest size — plenty for a VPN. You can move anywhere later in one click.</p>
              <div className="flex flex-wrap items-baseline justify-between gap-3 border-y-2 border-line py-4">
                <span className="font-mono text-[15px] text-stone">
                  <span className="text-[28px] font-bold text-bone">{suggestion.cheapest?.monthly_usd != null ? `$${suggestion.cheapest.monthly_usd.toFixed(2)}` : "—"}</span>/mo
                  {suggestion.cheapest ? ` · ${formatHourly(suggestion.cheapest.hourly_usd) ?? ""}` : ""}
                </span>
                <span className="font-display text-[12px] uppercase text-stone">Ready in ~2 min</span>
              </div>
              <Button
                className="h-16 text-[16px]"
                disabled={!suggestion.cheapest}
                onClick={() => {
                  if (!suggestion.cheapest) return;
                  router.push(buildHandoffUrl({
                    providerId: suggestion.providerId,
                    region: suggestion.slug,
                    planKey: suggestion.cheapest.plan_key,
                    place: suggestion.city ?? suggestion.slug,
                  }));
                }}
              >
                Build it &amp; connect <PixelIcon name="arrow-right" className="h-4 w-4" />
              </Button>
              <div className="flex flex-wrap items-center justify-between gap-3">
                <button type="button" onClick={() => router.push("/dashboard/map")} className="border-b-2 border-lime text-[15px] text-lime">
                  Pick another city
                </button>
                <span className="text-[13.5px] text-stone">Connects automatically when ready</span>
              </div>
            </>
          ) : suggestionError ? (
            <p role="alert" className="text-[14px] text-alert">{suggestionError}</p>
          ) : (
            <p className="text-[15px] text-stone">Reading {provider?.name ?? "the provider"}&apos;s regions and prices…</p>
          )}
        </Panel>
      </div>
    </div>
  );
}
