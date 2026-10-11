"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";
import type { ReactNode } from "react";
import { GoatMark, PixelIcon, type PixelIconName } from "@/components/ui/pixel-icon";
import { deriveShellTone, SHELL_TONE_COLOR, type ShellTone } from "@/lib/shell-tone";
import { useDashboard } from "./dashboard-provider";
import { Button, type AppView, VIEW_ROUTE } from "./shared";
import { ConnectProgress } from "./connect-progress";
import { OperationProgress } from "./operation-progress";

const NAV_ITEMS: Array<{ id: AppView; label: string; icon: PixelIconName }> = [
  { id: "home", label: "Home", icon: "home" },
  { id: "map", label: "Map", icon: "map" },
  { id: "shield", label: "Shield", icon: "shield" },
  { id: "setup", label: "Setup", icon: "setup" },
];

/** Which rail item a path belongs to. Nested routes (the map's build form) belong to their parent. */
function getActiveView(pathname: string | null): AppView {
  const path = pathname?.replace(/\/$/, "") ?? "";
  const nested = NAV_ITEMS.filter((item) => item.id !== "home").find(
    (item) => path === VIEW_ROUTE[item.id] || path.startsWith(`${VIEW_ROUTE[item.id]}/`),
  );
  return nested?.id ?? "home";
}

/** What the logo means, for anyone who cannot see its colour. */
const TONE_LABEL: Record<ShellTone, string> = {
  secure: "Blink: secure",
  busy: "Blink: working",
  off: "Blink: off",
  failed: "Blink: needs attention",
};

function Rail({ activeView, tone }: { activeView: AppView; tone: ShellTone }) {
  const color = SHELL_TONE_COLOR[tone];

  return (
    <nav
      aria-label="Main"
      className="flex w-[var(--rail-w)] shrink-0 flex-col items-center gap-3 border-r-2 border-line bg-ink py-6 max-[560px]:w-full max-[560px]:flex-row max-[560px]:border-b-2 max-[560px]:border-r-0 max-[560px]:px-4 max-[560px]:py-2"
    >
      <div className="mb-6 max-[560px]:mb-0 max-[560px]:mr-3" style={{ color }}>
        <GoatMark className="h-12 w-11" label={TONE_LABEL[tone]} />
      </div>
      {NAV_ITEMS.map((item) => {
        const active = activeView === item.id;
        return (
          <Link
            key={item.id}
            href={VIEW_ROUTE[item.id]}
            aria-current={active ? "page" : undefined}
            className="flex h-[84px] w-[84px] flex-col items-center justify-center gap-2 transition-colors max-[560px]:h-14 max-[560px]:w-14"
            style={active ? { backgroundColor: color, color: "var(--ink)" } : { color: "var(--stone)" }}
          >
            <PixelIcon name={item.icon} className="h-7 w-7" />
            <span className="font-display text-[10px] uppercase">{item.label}</span>
          </Link>
        );
      })}
    </nav>
  );
}

export function DashboardShell({ children }: { children: ReactNode }) {
  const pathname = usePathname();
  const activeView = getActiveView(pathname);
  const { connectAttempt, dismissRecovery, operationAttempt, recovery, toasts, tunnelPhase } = useDashboard();

  const tone = deriveShellTone({
    tunnelPhase,
    connecting: connectAttempt !== null,
    operating: operationAttempt !== null,
    failed: recovery !== null,
  });

  return (
    <div className="flex min-h-0 flex-1 max-[560px]:flex-col">
      <Rail activeView={activeView} tone={tone} />

      <main className="scroll-area min-h-0 min-w-0 flex-1 overflow-y-auto">
        <div className="flex min-h-full flex-col gap-5 px-9 pb-9 pt-7 max-[560px]:px-4">
          {recovery ? (
            <div role="alert" className="flex items-start gap-4 border-2 border-alert bg-alert-ink px-5 py-3">
              <PixelIcon name="alert" className="mt-0.5 h-5 w-5 shrink-0 text-alert" />
              <div className="min-w-0 flex-1">
                <div className="text-[15px] font-semibold text-bone">{recovery.title}</div>
                <p className="mt-0.5 text-[13px] text-stone">{recovery.nextAction}</p>
              </div>
              <Button tone="quiet" size="sm" onClick={dismissRecovery}>Dismiss</Button>
            </div>
          ) : null}

          {children}
        </div>
      </main>

      {/* Transient per-action feedback. Kept distinguishable from the persistent recovery strip so
          tests can tell "this action just failed" from "an earlier one did". */}
      <div
        data-testid="toast-region"
        className={`fixed bottom-6 right-6 z-50 flex w-[min(92vw,520px)] flex-col gap-3 ${toasts.length === 0 ? "pointer-events-none" : ""}`}
      >
        {toasts.map((toast) => (
          <div
            key={toast.id}
            data-testid={`toast-${toast.kind}`}
            className={`border-2 border-bone bg-panel p-[14px] ${toast.kind === "error" ? "shadow-hard-alert" : toast.kind === "warning" ? "shadow-[4px_4px_0_var(--ember)]" : "shadow-hard-lime"}`}
          >
            <div className="text-[15px] font-semibold text-bone">{toast.title}</div>
            <div className="mt-1 text-[12.5px] text-stone">{toast.message}</div>
          </div>
        ))}
      </div>

      {/* Mounted at the shell so a connect started on one page keeps explaining itself after the
          user navigates away from it. */}
      <ConnectProgress />
      <OperationProgress />
    </div>
  );
}
