"use client";

import { useEffect, useMemo, useState } from "react";

import { PixelIcon } from "@/components/ui/pixel-icon";
import { formatHourly, formatMonthly } from "@/lib/home-display";
import type { MapRegion } from "@/lib/map/map-regions";
import { listProviderPricing, toCommandError, type ProviderPlanPricing, type VpnNodeRecord } from "@/lib/tauri";
import type { MoveInput } from "./dashboard-provider";
import { Button, INPUT_CLASS, Modal, Tag } from "./shared";

/** Priced plans, cheapest first, keeping the cheapest and one step up: a VPN rarely needs more. */
function offeredSizes(plans: readonly ProviderPlanPricing[]): ProviderPlanPricing[] {
  return [...plans]
    .filter((plan) => plan.monthly_usd !== null)
    .sort((a, b) => (a.monthly_usd ?? 0) - (b.monthly_usd ?? 0))
    .slice(0, 2);
}

function describeSize(plan: ProviderPlanPricing): string {
  const memory = plan.memory_mb >= 1024 ? `${(plan.memory_mb / 1024).toFixed(plan.memory_mb % 1024 === 0 ? 0 : 1)} GB` : `${plan.memory_mb} MB`;
  return `${plan.vcpus} vCPU · ${memory}`;
}

/**
 * Moving the tunnel to a new city, or building the first server there.
 *
 * A new city is a new server, so the sheet says so: what gets built, what it costs, and that the old
 * server is powered off once the new tunnel works — never before.
 */
export function MoveSheet({
  target,
  from,
  lastPlanKey,
  busy,
  onClose,
  onConfirm,
}: {
  target: MapRegion;
  /** The server the tunnel is on now, or null for a plain build. */
  from: VpnNodeRecord | null;
  /** The plan used the last time this location was on, so "same as last time" can be offered. */
  lastPlanKey: string | null;
  busy: boolean;
  onClose: () => void;
  onConfirm: (input: MoveInput) => void;
}) {
  const [plans, setPlans] = useState<ProviderPlanPricing[] | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [planKey, setPlanKey] = useState<string | null>(null);
  const [keepOld, setKeepOld] = useState(false);
  const place = target.city ?? target.slug;
  const [nodeName, setNodeName] = useState(place);

  useEffect(() => {
    let active = true;
    void listProviderPricing(target.providerId, target.slug)
      .then((response) => {
        if (!active) return;
        const sizes = offeredSizes(response.plans);
        setPlans(sizes);
        setPlanKey(sizes.find((plan) => plan.plan_key === lastPlanKey)?.plan_key ?? sizes[0]?.plan_key ?? null);
      })
      .catch((error) => {
        if (active) setLoadError(`${toCommandError(error).message} ${toCommandError(error).nextAction}`);
      });
    return () => {
      active = false;
    };
  }, [lastPlanKey, target.providerId, target.slug]);

  const chosen = useMemo(() => plans?.find((plan) => plan.plan_key === planKey) ?? null, [planKey, plans]);
  const fromPlace = from ? from.region_geo?.city ?? from.name : null;
  const moving = from !== null;

  const steps: Array<{ label: string; time: string }> = [
    { label: `Build ${place} on ${target.providerName}`, time: "~2:00" },
    moving
      ? { label: "Switch the tunnel over (Windows asks twice: stop, then start)", time: "~0:20" }
      : { label: "Connect there (one Windows prompt)", time: "~0:15" },
  ];
  if (moving && !keepOld) steps.push({ label: `Power off ${fromPlace} · its bill stops`, time: "~0:05" });

  return (
    <Modal onClose={onClose} labelledBy="move-sheet-title" width="lg">
      <div className="border-b-2 border-line pb-6">
        <Tag state="secure">{moving ? "Move" : "Build"}</Tag>
        <h2 id="move-sheet-title" className="mt-3 font-display text-[clamp(26px,3vw,40px)] uppercase leading-tight text-bone">
          {moving ? `${fromPlace} → ${place}` : `Build ${place}`}
        </h2>
        <p className="mt-3 max-w-[620px] text-[15px] text-stone">
          {moving
            ? `A new city means a new server. Blink builds ${place}, switches your tunnel to it, then powers off ${fromPlace}. Between stopping one tunnel and starting the other, traffic briefly uses your normal connection.`
            : `Blink builds ${place} on ${target.providerName} and connects you as soon as it is ready.`}
        </p>
        <div className="mt-5 flex flex-wrap items-end gap-x-6 gap-y-2">
          <span className="font-mono text-[40px] font-bold leading-none text-bone">
            {chosen?.monthly_usd != null ? `$${chosen.monthly_usd.toFixed(2)}` : "—"}
          </span>
          <span className="pb-1 font-mono text-[13px] uppercase text-stone">
            / month{chosen ? ` · ${formatHourly(chosen.hourly_usd) ?? ""}` : ""}
          </span>
          <span className="pb-1 font-display text-[12px] uppercase text-lime">Ready in ~2 min</span>
        </div>
      </div>

      <div className="grid grid-cols-2 gap-8 py-6 max-[760px]:grid-cols-1">
        <div>
          <Tag>What happens</Tag>
          <ol className="mt-3">
            {steps.map((step, index) => (
              <li key={step.label} className="flex items-center gap-4 border-b-2 border-line-2 py-3 last:border-b-0">
                <span className="flex h-9 w-9 shrink-0 items-center justify-center border-2 border-line-3 font-mono text-[13px] text-stone">{index + 1}</span>
                <span className="flex-1 text-[15px] text-bone">{step.label}</span>
                <span className="font-mono text-[12.5px] text-stone">{step.time}</span>
              </li>
            ))}
          </ol>
        </div>

        <div>
          <Tag>Size</Tag>
          {loadError ? (
            <p className="mt-3 text-[14px] text-alert">{loadError}</p>
          ) : plans === null ? (
            <p className="mt-3 text-[14px] text-stone">Reading today&apos;s prices…</p>
          ) : plans.length === 0 ? (
            <p className="mt-3 text-[14px] text-stone">{target.providerName} publishes no priced size here.</p>
          ) : (
            <div role="radiogroup" aria-label="Size" className="mt-3 flex flex-col gap-3">
              {plans.map((plan, index) => {
                const selected = plan.plan_key === planKey;
                return (
                  <button
                    key={plan.plan_key}
                    type="button"
                    role="radio"
                    aria-checked={selected}
                    onClick={() => setPlanKey(plan.plan_key)}
                    className={`flex items-start justify-between gap-4 border-2 px-5 py-4 text-left transition-colors ${selected ? "border-lime" : "border-line hover:border-line-3"}`}
                  >
                    <span className="min-w-0">
                      <span className="block text-[16px] font-semibold text-bone">{plan.plan_label}</span>
                      <span className="mt-1 block text-[13px] text-stone">
                        {describeSize(plan)} · {plan.plan_key === lastPlanKey ? "Same as last time." : index === 0 ? "Plenty for a VPN." : "If several phones stream at once."}
                      </span>
                    </span>
                    <span className="shrink-0 font-mono text-[15px] text-bone">{formatMonthly(plan.monthly_usd)}</span>
                  </button>
                );
              })}
            </div>
          )}

          {moving ? (
            <label className="mt-4 flex cursor-pointer items-start gap-3 text-[14px] text-stone">
              <input type="checkbox" checked={keepOld} onChange={(event) => setKeepOld(event.target.checked)} className="mt-0.5 h-5 w-5 accent-[var(--lime)]" />
              Keep {fromPlace} running too (doubles the bill)
            </label>
          ) : null}

          <details className="mt-4">
            <summary className="cursor-pointer text-[13.5px] text-stone">Advanced</summary>
            <label className="mt-3 block space-y-2">
              <Tag>Server name</Tag>
              <input
                className={INPUT_CLASS}
                value={nodeName}
                maxLength={64}
                onChange={(event) => setNodeName(event.target.value)}
                aria-label="Server name"
              />
            </label>
          </details>
        </div>
      </div>

      <div className="flex flex-wrap items-center justify-between gap-4 border-t-2 border-line pt-5">
        <p className="text-[13.5px] text-stone">
          {moving ? `Phones will need to scan ${place}'s new codes.` : "Billing starts when the server is created."}
        </p>
        <div className="flex gap-3">
          <Button tone="ghost" onClick={onClose}>Cancel</Button>
          <Button
            disabled={busy || !chosen || nodeName.trim().length === 0}
            onClick={() =>
              chosen &&
              onConfirm({
                providerId: target.providerId,
                region: target.slug,
                planKey: chosen.plan_key,
                place,
                nodeName: nodeName.trim(),
                keepOld,
              })
            }
          >
            {moving ? `Move to ${place}` : `Build ${place}`} <PixelIcon name="arrow-right" className="h-3.5 w-3.5" />
          </Button>
        </div>
      </div>
    </Modal>
  );
}
