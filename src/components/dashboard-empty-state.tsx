"use client";

import Link from "next/link";

export function DashboardEmptyState() {
  return (
    <div className="flex min-h-[58vh] items-center justify-center">
      <div className="w-full max-w-xl rounded-[24px] border border-[color:var(--outline-variant)] bg-[linear-gradient(180deg,rgba(28,32,39,0.96),rgba(16,19,27,0.96))] p-8 text-center shadow-[var(--shadow-elevated)]">
        <h2 className="text-2xl font-semibold tracking-tight text-[var(--on-surface)]">No VPN server configured yet</h2>
        <p className="mt-3 text-sm leading-6 text-[var(--on-surface-variant)]">
          Complete the guided setup to verify WireGuard readiness, save your DigitalOcean token, and provision your first VPN node.
        </p>
        <div className="mt-7">
          <Link
            href="/onboarding"
            className="inline-flex items-center justify-center rounded-xl border border-[color:rgba(171,199,255,0.25)] bg-[var(--primary)] px-5 py-2.5 text-sm font-medium text-[#10131b] transition-colors hover:brightness-105"
          >
            Get Started
          </Link>
        </div>
      </div>
    </div>
  );
}
