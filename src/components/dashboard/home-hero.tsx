"use client";

import type { ReactNode } from "react";
import { Panel, Tag } from "./shared";

export type HeroTone = "secure" | "busy" | "off" | "failed";

const HEADLINE_COLOR: Record<HeroTone, string> = {
  secure: "text-lime",
  busy: "text-ember",
  off: "text-stone-2",
  failed: "text-alert",
};

/**
 * The Home hero: a state tag, one giant word, one line under it, the giant action to the right,
 * and a footer pinned to the bottom of the panel. It fills the window at any size.
 */
export function HomeHero({
  tone,
  tag,
  headline,
  subline,
  action,
  footer,
}: {
  tone: HeroTone;
  tag: ReactNode;
  headline: string;
  subline: ReactNode;
  action: ReactNode;
  footer: ReactNode;
}) {
  return (
    <Panel
      tint={tone === "secure" ? "secure" : tone === "busy" ? "busy" : tone === "failed" ? "failed" : "none"}
      className="flex min-h-[520px] flex-1 flex-col"
      bodyClassName="flex flex-1 flex-col px-[clamp(24px,4vw,60px)] pb-10 pt-[clamp(24px,4vw,54px)]"
    >
      <div className="flex flex-wrap items-start justify-between gap-8">
        <div className="min-w-0">
          <Tag state={tone}>{tag}</Tag>
          <h1
            className={`mt-2 font-display text-[clamp(72px,10vw,140px)] uppercase leading-[0.9] ${HEADLINE_COLOR[tone]}`}
          >
            {headline}
          </h1>
          <div className="mt-6 text-[clamp(20px,2vw,28px)] font-semibold text-bone">{subline}</div>
        </div>
        {action}
      </div>

      <div className="flex-1" />

      <div className="mt-8 border-t-2 border-line pt-7">{footer}</div>
    </Panel>
  );
}

/** One cell of the bottom strip: a tag over a big mono value. */
export function HeroStat({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="min-w-0 px-6 first:pl-0 last:pr-0 [&:not(:first-child)]:border-l-2 [&:not(:first-child)]:border-line">
      <Tag>{label}</Tag>
      <div className="mt-2">{children}</div>
    </div>
  );
}
