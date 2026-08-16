"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";
import { useEffect, useState, type ReactNode } from "react";
import { useDashboard } from "./dashboard-provider";
import { AppIcon, type AppView, type DashboardBusyAction, VIEW_ROUTE } from "./shared";

const navItems: Array<{ id: AppView; label: string; icon: string; eyebrow: string }> = [
  { id: "dashboard", label: "Dashboard", icon: "grid", eyebrow: "Overview" },
  { id: "nodes", label: "Server Locations", icon: "server", eyebrow: "Provision" },
  { id: "health", label: "Kill Switch", icon: "shield", eyebrow: "Connection" },
  { id: "settings", label: "Settings", icon: "lock", eyebrow: "Provider" },
  { id: "diagnostics", label: "Support", icon: "support", eyebrow: "Logs" },
];

const titleMap: Record<AppView, string> = {
  dashboard: "Network Stats",
  nodes: "Server Locations",
  health: "Connection Health",
  diagnostics: "Diagnostics",
  settings: "Provider Integration",
};

const subtitleMap: Record<AppView, string> = {
  dashboard: "Monitor the active tunnel, throughput, and local node state.",
  nodes: "Choose where your traffic exits, and provision new servers.",
  health: "Inspect runtime handshake, endpoint, and tunnel health.",
  diagnostics: "Review redacted logs, recovery suggestions, and exportable bundles.",
  settings: "Register and validate provider tokens used for orchestration.",
};

function getActiveView(pathname: string | null): AppView {
  if (pathname?.startsWith(VIEW_ROUTE.nodes)) {
    return "nodes";
  }

  const matched = navItems.find((item) => VIEW_ROUTE[item.id] === pathname);
  return matched?.id ?? "dashboard";
}

/**
 * Turns a busy-action key into something readable.
 *
 * Deliberately mechanical rather than a lookup table: a table would silently fall out of step as
 * actions are added, and a missing entry would render blank.
 */
function describeBusyAction(action: NonNullable<DashboardBusyAction>): string {
  const words = action.replace(/-/g, " ");
  return words.charAt(0).toUpperCase() + words.slice(1);
}

/**
 * Live connection state, pinned under the brand in the sidebar.
 *
 * Sits in the sidebar rather than the page header so it stays in one place across routes and does
 * not compete with each page's title for the top of the screen.
 */
function SidebarStatus({
  heading,
  detail,
  tone,
  attention,
  onDismiss,
}: {
  heading: string;
  detail: string;
  tone: string;
  attention: boolean;
  /** Present only for a recovery message, which otherwise stays until the next success. */
  onDismiss?: () => void;
}) {
  return (
    <div
      className={`mt-5 rounded-2xl border px-3 py-2.5 ${attention
        ? "border-[color:rgba(255,185,95,0.25)] bg-[color:rgba(255,185,95,0.08)]"
        : "border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.03)]"}`}
    >
      <div className="flex items-center gap-2">
        <span className="h-2 w-2 shrink-0 rounded-full" style={{ backgroundColor: tone }} />
        <span className="flex-1 text-[10px] font-semibold uppercase tracking-[0.2em]" style={{ color: tone }}>{heading}</span>
        {onDismiss ? (
          <button
            type="button"
            onClick={onDismiss}
            aria-label="Dismiss"
            title="Dismiss"
            className="-mr-1 rounded p-0.5 text-[var(--on-surface-variant)] transition-colors hover:bg-[color:rgba(255,255,255,0.08)] hover:text-[var(--on-surface)]"
          >
            <svg viewBox="0 0 24 24" className="h-3 w-3" aria-hidden="true"><path d="m6 6 12 12M18 6 6 18" className="fill-none stroke-current stroke-[2.5]" /></svg>
          </button>
        ) : null}
      </div>
      <p className="mt-1.5 text-xs leading-5 text-[var(--on-surface-variant)]">{detail}</p>
    </div>
  );
}

export function DashboardShell({ children }: { children: ReactNode }) {
  const pathname = usePathname();
  const activeView = getActiveView(pathname);
  const [mobileNavOpen, setMobileNavOpen] = useState(false);
  const {
    activeNode,
    busyAction,
    connectedTunnel,
    connectPrimary,
    connectTunnel,
    disconnectTunnel,
    dismissRecovery,
    health,
    recovery,
    toasts,
    tunnelConnected,
  } = useDashboard();

  // Live connection state, shown in the sidebar under the brand. Replaces a developer note about
  // a routing refactor that had been left rendering as the header banner's idle text.
  const statusHeading = recovery
    ? recovery.title
    : busyAction
      ? "Working"
      : tunnelConnected
        ? "Connected"
        : "Idle";
  const statusDetail = recovery
    ? recovery.nextAction
    : busyAction
      ? `${describeBusyAction(busyAction)}…`
      : tunnelConnected
        ? [activeNode?.name, health?.external_ipv4 ? `via ${health.external_ipv4}` : null]
          .filter(Boolean)
          .join(" · ") || "Tunnel is up."
        : activeNode
          ? `${activeNode.name} is provisioned and not connected.`
          : "No server is configured yet.";
  const statusTone = recovery
    ? "var(--tertiary)"
    : tunnelConnected
      ? "var(--secondary)"
      : "var(--on-surface-variant)";

  useEffect(() => {
    const closeOnDesktop = () => {
      if (window.innerWidth >= 1024) {
        setMobileNavOpen(false);
      }
    };

    closeOnDesktop();
    window.addEventListener("resize", closeOnDesktop);
    return () => window.removeEventListener("resize", closeOnDesktop);
  }, []);

  return (
    <div className="min-h-screen bg-[var(--surface-container-lowest)] text-[var(--on-surface)]">
      <div className="flex min-h-screen">
        <div
          className={`fixed inset-0 z-30 bg-[color:rgba(3,5,11,0.72)] transition-opacity lg:hidden ${mobileNavOpen ? "opacity-100" : "pointer-events-none opacity-0"}`}
          onClick={() => setMobileNavOpen(false)}
          aria-hidden="true"
        />

        <aside className={`fixed inset-y-0 left-0 z-40 flex w-[240px] flex-col overflow-y-auto border-r border-[color:var(--outline-variant)] bg-[linear-gradient(180deg,rgba(17,20,28,0.98),rgba(13,16,23,0.98))] px-4 py-5 shadow-[20px_0_60px_rgba(0,0,0,0.25)] transition-transform duration-200 lg:hidden ${mobileNavOpen ? "translate-x-0" : "-translate-x-full"}`}>
          <div className="flex items-center justify-between gap-3 rounded-2xl px-1 py-1.5">
            <div className="flex items-center gap-3">
              <div className="flex h-12 w-12 items-center justify-center rounded-xl border border-[color:rgba(171,199,255,0.25)] bg-[color:rgba(78,122,255,0.85)] text-white shadow-[0_8px_30px_rgba(78,122,255,0.28)]">
                <AppIcon name="shield" className="h-6 w-6" />
              </div>
              <div>
                <div className="text-xl font-semibold tracking-tight text-[var(--on-surface)]">GoatStare VPN</div>
                <div className="text-sm text-[var(--on-surface-variant)]">Enterprise Edition</div>
              </div>
            </div>
            <button type="button" onClick={() => setMobileNavOpen(false)} className="inline-flex h-9 w-9 items-center justify-center rounded-lg border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.03)] text-[var(--on-surface-variant)] hover:bg-[color:rgba(255,255,255,0.08)]" aria-label="Close menu">
              <svg viewBox="0 0 24 24" className="h-4 w-4" aria-hidden="true"><path d="m6 6 12 12M18 6 6 18" className="fill-none stroke-current stroke-[1.8]" /></svg>
            </button>
          </div>

          <SidebarStatus heading={statusHeading} detail={statusDetail} tone={statusTone} attention={!!recovery} onDismiss={recovery ? dismissRecovery : undefined} />

          <nav className="mt-6 space-y-2">
            {navItems.map((item) => {
              const active = activeView === item.id;
              return (
                <Link
                  key={`mobile-${item.id}`}
                  href={VIEW_ROUTE[item.id]}
                  onClick={() => setMobileNavOpen(false)}
                  className={`flex w-full items-center gap-3 rounded-2xl border px-3 py-3 text-left transition-colors ${active ? "border-[color:rgba(171,199,255,0.35)] bg-[color:rgba(171,199,255,0.12)] text-[var(--primary)]" : "border-transparent text-[var(--on-surface-variant)] hover:bg-[color:rgba(255,255,255,0.04)] hover:text-[var(--on-surface)]"}`}
                >
                  <div className={`flex h-10 w-10 items-center justify-center rounded-xl ${active ? "bg-[color:rgba(171,199,255,0.2)]" : "bg-[color:rgba(255,255,255,0.04)]"}`}>
                    <AppIcon name={item.icon} className="h-5 w-5" />
                  </div>
                  <div>
                    <div className="font-medium">{item.label}</div>
                    <div className="text-xs uppercase tracking-[0.24em] opacity-70">{item.eyebrow}</div>
                  </div>
                </Link>
              );
            })}
          </nav>
        </aside>

        <aside className="fixed inset-y-0 left-0 hidden w-[240px] flex-col overflow-y-auto border-r border-[color:var(--outline-variant)] bg-[linear-gradient(180deg,rgba(17,20,28,0.98),rgba(13,16,23,0.98))] px-4 py-5 shadow-[20px_0_60px_rgba(0,0,0,0.25)] lg:flex">
          <div className="flex items-center gap-3 rounded-2xl px-1 py-1.5">
            <div className="flex h-12 w-12 items-center justify-center rounded-xl border border-[color:rgba(171,199,255,0.25)] bg-[color:rgba(78,122,255,0.85)] text-white shadow-[0_8px_30px_rgba(78,122,255,0.28)]">
              <AppIcon name="shield" className="h-6 w-6" />
            </div>
            <div>
              <div className="text-xl font-semibold tracking-tight text-[var(--on-surface)]">GoatStare VPN</div>
              <div className="text-sm text-[var(--on-surface-variant)]">Enterprise Edition</div>
            </div>
          </div>

          <SidebarStatus heading={statusHeading} detail={statusDetail} tone={statusTone} attention={!!recovery} onDismiss={recovery ? dismissRecovery : undefined} />

          <nav className="mt-6 space-y-2">
            {navItems.map((item) => {
              const active = activeView === item.id;
              return (
                <Link
                  key={item.id}
                  href={VIEW_ROUTE[item.id]}
                  className={`flex w-full items-center gap-3 rounded-2xl border px-3 py-3 text-left transition-colors ${active ? "border-[color:rgba(171,199,255,0.35)] bg-[color:rgba(171,199,255,0.12)] text-[var(--primary)]" : "border-transparent text-[var(--on-surface-variant)] hover:bg-[color:rgba(255,255,255,0.04)] hover:text-[var(--on-surface)]"}`}
                >
                  <div className={`flex h-10 w-10 items-center justify-center rounded-xl ${active ? "bg-[color:rgba(171,199,255,0.2)]" : "bg-[color:rgba(255,255,255,0.04)]"}`}>
                    <AppIcon name={item.icon} className="h-5 w-5" />
                  </div>
                  <div>
                    <div className="font-medium">{item.label}</div>
                    <div className="text-xs uppercase tracking-[0.24em] opacity-70">{item.eyebrow}</div>
                  </div>
                </Link>
              );
            })}
          </nav>

          {/* `mt-auto` pins this to the bottom, so the status block above cannot push it off-screen. */}
          <div className="mt-auto rounded-[24px] border border-[color:rgba(171,199,255,0.22)] bg-[color:rgba(171,199,255,0.12)] p-4">
            <div className="flex items-center gap-2 text-[var(--primary)]">
              <AppIcon name="shield" className="h-4 w-4" />
              <span className="text-[11px] uppercase tracking-[0.28em]">Quick Connect</span>
            </div>
            <p className="mt-3 text-sm text-[var(--on-surface)]/90">
              {activeNode
                ? tunnelConnected
                  ? `Connected to ${activeNode.name}.`
                  : `${activeNode.name} is ready to connect.`
                : "Provision a server to connect."}
            </p>
            {/* Actually connects. It was a card named "Quick Connect" whose only control linked to
                the page the user was already on. */}
            <button
              type="button"
              disabled={busyAction !== null || !activeNode}
              onClick={() => {
                if (!activeNode) return;
                if (connectedTunnel) {
                  if (tunnelConnected) {
                    void disconnectTunnel(connectedTunnel);
                  } else {
                    void connectTunnel(connectedTunnel);
                  }
                  return;
                }
                void connectPrimary();
              }}
              className="mt-4 inline-flex w-full items-center justify-center gap-2 rounded-xl border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.03)] px-4 py-2.5 text-sm font-medium text-[var(--on-surface)] transition-colors hover:bg-[color:rgba(255,255,255,0.06)] disabled:cursor-not-allowed disabled:opacity-50"
            >
              <AppIcon name="power" className="h-4 w-4" />
              {tunnelConnected
                ? (busyAction === "disconnect-tunnel" ? "Disconnecting" : "Disconnect")
                : (busyAction === "connect-tunnel" || busyAction === "create-tunnel" ? "Connecting" : "Connect")}
            </button>
          </div>
        </aside>

        <main className="flex-1 lg:pl-[240px]">
          <div className="mx-auto flex min-h-screen w-full max-w-[1420px] flex-col gap-6 px-4 py-4 lg:px-6 lg:py-5">
            <div className="flex items-center justify-between rounded-2xl border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.02)] px-4 py-3 text-sm text-[var(--on-surface-variant)] lg:hidden">
              <div className="font-medium text-[var(--on-surface)]">GoatStare VPN</div>
              <button type="button" onClick={() => setMobileNavOpen((current) => !current)} className="inline-flex items-center gap-2 rounded-xl border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.03)] px-3 py-2 text-[var(--on-surface)]" aria-label="Open menu">
                <svg viewBox="0 0 24 24" className="h-4 w-4" aria-hidden="true"><path d="M4 7h16M4 12h16M4 17h16" className="fill-none stroke-current stroke-[1.8]" /></svg>
                Menu
              </button>
            </div>

            <header className="rounded-[28px] border border-[color:var(--outline-variant)] bg-[linear-gradient(180deg,rgba(16,19,27,0.85),rgba(14,17,24,0.85))] px-5 py-4 shadow-[var(--shadow-elevated)] backdrop-blur-md">
              <h1 className="text-3xl font-semibold tracking-tight text-[var(--on-surface)]">{titleMap[activeView]}</h1>
              <p className="mt-2 max-w-3xl text-sm leading-6 text-[var(--on-surface-variant)]">{subtitleMap[activeView]}</p>
            </header>

            {children}
          </div>
        </main>

        {/* Transient per-action feedback. Kept distinguishable from the persistent recovery
            banner so tests can tell "this action just failed" from "an earlier one did". */}
        <div
          data-testid="toast-region"
          className={`fixed right-4 top-4 z-50 flex w-[min(92vw,24rem)] flex-col gap-3 ${toasts.length === 0 ? "pointer-events-none" : ""}`}
        >
          {toasts.map((toast) => (
            <div key={toast.id} data-testid={`toast-${toast.kind}`} className={`rounded-2xl border px-4 py-3 shadow-[var(--shadow-elevated)] ${toast.kind === "success" ? "border-[color:rgba(78,222,163,0.24)] bg-[color:rgba(78,222,163,0.12)] text-[var(--on-surface)]" : toast.kind === "warning" ? "border-[color:rgba(255,185,95,0.24)] bg-[color:rgba(255,185,95,0.12)] text-[var(--on-surface)]" : "border-[color:rgba(255,107,107,0.24)] bg-[color:rgba(255,107,107,0.12)] text-[var(--on-surface)]"}`}>
              <div className="text-sm font-medium">{toast.title}</div>
              <div className="mt-1 text-sm text-[var(--on-surface)]/90">{toast.message}</div>
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}