"use client";

import { useMemo, useState, type ReactNode } from "react";

import { PixelIcon } from "@/components/ui/pixel-icon";
import {
  ACTIVITY_TABS,
  describeRecovery,
  describeRunHeadline,
  failingStep,
  filterActivity,
  formatRelativeTime,
  formatStepTime,
  groupActivity,
  selectUnresolvedFailure,
  type ActivityOutcome,
  type ActivityRun,
  type ActivityTab,
} from "@/lib/activity-display";
import { describeEgress } from "@/lib/connection-display";
import { formatDropAge, type CheckStatus, type ConnectivityCheck, type describeTunnelConfig } from "@/lib/health-display";
import type { AppLocation, NodePeerRecord, PeerConfigPayload, TunnelSettings, WireGuardReadiness } from "@/lib/tauri";

import { DeviceSlots } from "./device-slots";
import {
  Button,
  PageHeader,
  Panel,
  Row,
  Tabs,
  Tag,
  formatHandshakeAge,
  useNow,
  type DashboardBusyAction,
  type DiagnosticsBundle,
  type HealthPanelData,
  type VpnNodeRecord,
} from "./shared";

type TunnelConfigDisplay = ReturnType<typeof describeTunnelConfig>;
type ThroughputSample = { timestamp: number; downloadBytesPerSecond: number; uploadBytesPerSecond: number };
type ShieldTab = "status" | "checks" | "config" | "log";

/**
 * WireGuard's cipher suite is fixed by the protocol — there is no negotiation and no backend field
 * reporting it, which is why this is a constant rather than a value read from a snapshot.
 */
const WIREGUARD_CIPHER = "ChaCha20";
const WIREGUARD_CIPHER_DETAIL = "Poly1305 · Curve25519";

const CHECK_COLOR: Record<CheckStatus, string> = {
  pass: "bg-lime",
  fail: "bg-alert",
  warn: "bg-ember",
  unknown: "bg-stone-2",
};

const CHECK_WORD: Record<CheckStatus, string> = {
  pass: "Passing",
  fail: "Failed",
  warn: "Warning",
  unknown: "Unknown",
};

const OUTCOME_TEXT: Record<ActivityOutcome, string> = {
  success: "text-lime",
  error: "text-alert",
  info: "text-stone",
};

function formatRate(bytesPerSecond: number): string {
  if (!Number.isFinite(bytesPerSecond) || bytesPerSecond <= 0) return "0 B/s";
  const units = ["B/s", "KB/s", "MB/s", "GB/s"];
  let value = bytesPerSecond;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value.toFixed(value >= 100 || unit === 0 ? 0 : 1)} ${units[unit]}`;
}

/** How long ago the tunnel dropped, kept current. Mounted only when there is a drop. */
function DropAge({ observedAtEpochMs }: { observedAtEpochMs: number }) {
  const now = useNow();
  return <>{formatDropAge(observedAtEpochMs, now)}</>;
}

function StatTile({ label, value, meta, valueClass = "text-bone" }: { label: string; value: ReactNode; meta: ReactNode; valueClass?: string }) {
  return (
    <div className="min-w-0 border-2 border-line bg-panel px-6 py-5">
      <Tag>{label}</Tag>
      <div className={`mt-2 truncate font-mono text-[26px] font-bold leading-tight ${valueClass}`}>{value}</div>
      <div className="mt-2 truncate font-mono text-[12.5px] uppercase text-stone" title={typeof meta === "string" ? meta : undefined}>{meta}</div>
    </div>
  );
}

/**
 * Down and up as bar pairs, lime and bone. No lines, no gradients, no axes beyond min and max.
 *
 * An empty chart rather than flat zeros when nothing is measured: no measurement is a different
 * claim from a measured zero.
 */
function ThroughputBars({ samples, tunnelConnected }: { samples: ThroughputSample[]; tunnelConnected: boolean }) {
  const latest = samples[samples.length - 1] ?? null;
  const max = Math.max(1, ...samples.flatMap((sample) => [sample.downloadBytesPerSecond, sample.uploadBytesPerSecond]));
  const windowSeconds = samples.length > 1
    ? Math.round((samples[samples.length - 1].timestamp - samples[0].timestamp) / 1000)
    : 0;

  return (
    <div className="min-w-0">
      <div className="flex flex-wrap items-center justify-between gap-4">
        <h3 className="font-mono text-[18px] text-bone">Throughput{windowSeconds > 0 ? ` // ${windowSeconds}s` : ""}</h3>
        {latest ? (
          <div className="flex items-center gap-5 font-mono text-[13px] text-bone">
            <span className="flex items-center gap-2"><span className="h-3.5 w-3.5 bg-lime" />DN {formatRate(latest.downloadBytesPerSecond)}</span>
            <span className="flex items-center gap-2"><span className="h-3.5 w-3.5 bg-bone" />UP {formatRate(latest.uploadBytesPerSecond)}</span>
          </div>
        ) : null}
      </div>
      {samples.length > 1 ? (
        <div className="relative mt-5">
          <span className="absolute right-0 top-0 font-mono text-[11.5px] text-stone">{formatRate(max)}</span>
          <div aria-hidden="true" className="flex h-[190px] items-end gap-[6px] border-b-2 border-stone-2 pt-5">
            {samples.map((sample) => (
              <div key={sample.timestamp} className="flex h-full flex-1 items-end gap-[3px]">
                <span className="flex-1 bg-lime" style={{ height: `${Math.max(2, (sample.downloadBytesPerSecond / max) * 100)}%` }} />
                <span className="flex-1 bg-bone" style={{ height: `${Math.max(2, (sample.uploadBytesPerSecond / max) * 100)}%` }} />
              </div>
            ))}
          </div>
          <span className="mt-1 block font-mono text-[11.5px] text-stone">0 B/s</span>
        </div>
      ) : (
        <div className="mt-5 flex h-[190px] items-center justify-center border-2 border-dashed border-line text-[13px] text-stone">
          {tunnelConnected ? "Sampling throughput…" : "Connect to measure throughput"}
        </div>
      )}
    </div>
  );
}

function CheckRows({ checks, detailed }: { checks: ConnectivityCheck[]; detailed: boolean }) {
  return (
    <div>
      {checks.map((check) => (
        <Row key={check.id} className={detailed ? "items-start py-4" : ""}>
          <span aria-hidden="true" className={`mt-1 h-4 w-4 shrink-0 ${CHECK_COLOR[check.status]}`} />
          <div className="min-w-0 flex-1">
            <div className="text-[16px] text-bone">{check.title}</div>
            {detailed ? <p className="mt-1 text-[13.5px] text-stone">{check.detail}</p> : null}
          </div>
          <span className="shrink-0 font-mono text-[12.5px] uppercase text-stone">{CHECK_WORD[check.status]}</span>
        </Row>
      ))}
    </div>
  );
}

function RunEntry({ run, headline, defaultOpen }: { run: ActivityRun; headline: string; defaultOpen: boolean }) {
  const [open, setOpen] = useState(defaultOpen);

  return (
    <div className="border-2 border-line bg-ink-2">
      <button
        type="button"
        onClick={() => setOpen((value) => !value)}
        aria-expanded={open}
        className="flex w-full items-center gap-3 px-4 py-3 text-left transition-colors hover:bg-panel-2"
      >
        <span aria-hidden="true" className={`h-3 w-3 shrink-0 ${run.outcome === "error" ? "bg-alert" : run.outcome === "success" ? "bg-lime" : "bg-stone-2"}`} />
        <span className={`flex-1 text-[14px] ${run.outcome === "error" ? "text-alert" : "text-bone"}`}>{headline}</span>
        <span className="font-mono text-[12px] text-stone">
          {run.steps.length} {run.steps.length === 1 ? "step" : "steps"}
        </span>
        <PixelIcon name="arrow-down" className={`h-4 w-4 text-stone ${open ? "rotate-180" : ""}`} />
      </button>

      {open ? (
        <div className="border-t-2 border-line px-4 py-3 font-mono text-[12px] leading-6">
          {run.steps.map((step) => (
            <div key={step.id} className="flex gap-3">
              <span className="w-[96px] shrink-0 whitespace-nowrap text-stone">{formatStepTime(step.createdAt)}</span>
              <span className={`w-[140px] shrink-0 truncate ${OUTCOME_TEXT[step.outcome]}`} title={step.stage}>{step.stage}</span>
              <span className="flex-1 break-all text-bone">{step.detail}</span>
            </div>
          ))}
        </div>
      ) : null}
    </div>
  );
}

export type ShieldViewProps = {
  health: HealthPanelData | null;
  killSwitch: { headline: string; detail: string; status: CheckStatus };
  checks: ConnectivityCheck[];
  tunnelConfig: TunnelConfigDisplay;
  tunnelConnected: boolean;
  tunnelSettings: TunnelSettings | null;
  throughputSamples: ThroughputSample[];
  readiness: WireGuardReadiness | null;
  lastCheckedAt: number | null;
  node: VpnNodeRecord | null;
  nodePeers: NodePeerRecord[];
  nodes: VpnNodeRecord[];
  diagnostics: DiagnosticsBundle | null;
  diagnosticsExportPath: string | null;
  locations: AppLocation[];
  busyAction: DashboardBusyAction;
  reconnecting: boolean;
  onRunChecks: () => Promise<void>;
  onReconnect: (() => Promise<void>) | null;
  onCopyConfig: (() => Promise<void>) | null;
  onCopyDiagnostics: () => Promise<void>;
  onExportDiagnostics: () => Promise<void>;
  onRevealLocation: (key: string) => Promise<void>;
  onCopyText: (value: string, label: string) => Promise<void>;
  onRecoverNode: (nodeId: string) => Promise<void>;
  onClaimDevice: (nodeId: string, peerId: string, label: string) => Promise<boolean>;
  onLoadDeviceConfig: (nodeId: string, peerId: string) => Promise<PeerConfigPayload | null>;
  onExportDeviceConfig: (nodeId: string, peerId: string) => Promise<void>;
};

/**
 * Everything that proves the tunnel is doing its job: the receipts Home hides.
 *
 * Folds the former Kill Switch and Activity pages into four tabs. Nothing here is new behaviour;
 * it is the same data, moved.
 */
export function ShieldView(props: ShieldViewProps) {
  const { health, checks, diagnostics } = props;
  const [tab, setTab] = useState<ShieldTab>("status");
  const now = useNow();

  const runs = useMemo(() => groupActivity(diagnostics?.activity), [diagnostics?.activity]);
  const failedRuns = useMemo(() => filterActivity(runs, "failures").length, [runs]);
  const passing = checks.filter((check) => check.status === "pass").length;
  const allPassing = passing === checks.length;
  const drop = health?.unexpected_drop ?? null;

  const tabs: Array<{ id: ShieldTab; label: string }> = [
    { id: "status", label: "Status" },
    { id: "checks", label: "Checks" },
    { id: "config", label: "Tunnel config" },
    { id: "log", label: failedRuns > 0 ? `Log · ${failedRuns} failed` : "Log" },
  ];

  return (
    <>
      <PageHeader
        title="Shield"
        subtitle="Everything that proves the tunnel is doing its job. You never need this page — it's here so you can check."
        action={
          <span className={`flex items-center gap-3 font-display text-[14px] uppercase ${allPassing ? "text-lime" : "text-ember"}`}>
            <span aria-hidden="true" className="h-4 w-4 bg-current" />
            {passing} / {checks.length} passing
          </span>
        }
      />

      {/* A tunnel that stopped on its own took WireGuard's filters with it, so traffic is flowing
          unprotected right now. This is the one thing on the page that must not be missable. */}
      {drop ? (
        <div role="alert" className="flex flex-wrap items-start justify-between gap-4 border-2 border-alert bg-alert-ink px-6 py-4">
          <div className="min-w-0">
            <Tag state="failed">Tunnel dropped</Tag>
            <p className="mt-2 max-w-3xl text-[15px] text-bone">
              The tunnel stopped without being asked to, <DropAge observedAtEpochMs={drop.observed_at_epoch_ms} />.
              Leak protection stopped with it, so traffic is using your normal connection until you reconnect.
            </p>
          </div>
          {props.onReconnect ? (
            <Button onClick={() => void props.onReconnect?.()} disabled={props.busyAction !== null || props.reconnecting}>
              {props.reconnecting ? "Reconnecting" : "Reconnect"}
            </Button>
          ) : null}
        </div>
      ) : null}

      <div className="flex flex-1 flex-col">
        <Tabs tabs={tabs} selected={tab} onSelect={setTab} label="Shield" idPrefix="shield" />
        <div
          role="tabpanel"
          id={`shield-panel-${tab}`}
          aria-labelledby={`shield-tab-${tab}`}
          className="flex-1 border-2 border-t-0 border-lime bg-panel p-8 max-[700px]:p-4"
        >
          {tab === "status" ? <StatusTab {...props} allPassing={allPassing} /> : null}
          {tab === "checks" ? <ChecksTab {...props} /> : null}
          {tab === "config" ? <ConfigTab {...props} /> : null}
          {tab === "log" ? <LogTab {...props} runs={runs} now={now} /> : null}
        </div>
      </div>
    </>
  );
}

function StatusTab(props: ShieldViewProps & { allPassing: boolean }) {
  const { health, killSwitch, node, tunnelConnected, tunnelSettings } = props;
  const armed = killSwitch.status === "pass";
  const egress = describeEgress({
    tunnelConnected,
    observedEgress: health?.external_ipv4 ?? null,
    serverIp: health?.endpoint_ip ?? node?.public_ipv4 ?? null,
  });
  const keepalive = tunnelSettings
    ? tunnelSettings.keepalive_seconds > 0 ? `Keepalive ${tunnelSettings.keepalive_seconds}s` : "Keepalive off"
    : "Keepalive —";

  return (
    <div className="flex flex-col gap-8">
      <div className="grid grid-cols-4 gap-5 max-[1100px]:grid-cols-2">
        <StatTile
          label="Kill switch"
          value={armed ? "ARMED" : killSwitch.headline}
          valueClass={armed ? "text-lime" : killSwitch.status === "fail" ? "text-alert" : "text-ember"}
          meta={health?.kill_switch.routes.length ? health.kill_switch.routes.join(" · ") : "No routes"}
        />
        <StatTile label="Exit IP" value={egress.value} meta={egress.caption} />
        <StatTile
          label="Handshake"
          value={formatHandshakeAge(health?.handshake_age_seconds, health?.tunnel_status)}
          meta={keepalive}
        />
        <StatTile label="Cipher" value={WIREGUARD_CIPHER} meta={WIREGUARD_CIPHER_DETAIL} />
      </div>

      <div className="grid grid-cols-[minmax(0,1.5fr)_minmax(0,1fr)] gap-10 max-[1100px]:grid-cols-1">
        <ThroughputBars samples={props.throughputSamples} tunnelConnected={tunnelConnected} />
        <div className="min-w-0">
          <h3 className="font-mono text-[18px] text-bone">Checks</h3>
          <div className="mt-3">
            <CheckRows checks={props.checks} detailed={false} />
          </div>
          <div className="mt-5 flex flex-wrap gap-3">
            <Button tone="quiet" onClick={() => void props.onRunChecks()} disabled={props.busyAction !== null}>
              {props.busyAction === "refresh-health" ? "Checking" : "Run again"}
            </Button>
            <Button tone="quiet" onClick={() => void props.onCopyDiagnostics()} disabled={props.busyAction !== null || !props.diagnostics}>
              Copy bug report
            </Button>
          </div>
        </div>
      </div>

      <p className="text-[12.5px] text-stone">
        Server cost on Home is an estimate from the price when the server was created. Your provider&apos;s invoice is the truth.
      </p>

      {node && props.nodePeers.some((peer) => peer.slot_index > 0) ? (
        <section className="border-t-2 border-line pt-8">
          <h3 className="mb-4 font-mono text-[18px] text-bone">Phones</h3>
          <DeviceSlots
            nodeId={node.id}
            peers={props.nodePeers}
            busyAction={props.busyAction}
            onClaimDevice={props.onClaimDevice}
            onLoadDeviceConfig={props.onLoadDeviceConfig}
            onExportDeviceConfig={props.onExportDeviceConfig}
            onCopyText={props.onCopyText}
          />
        </section>
      ) : null}
    </div>
  );
}

function ChecksTab(props: ShieldViewProps) {
  const failing = props.checks.filter((check) => check.status === "fail").length;
  return (
    <div className="flex flex-col gap-5">
      <div className="flex flex-wrap items-center justify-between gap-4">
        <p className="text-[15px] text-stone">
          {failing > 0 ? `${failing} check${failing === 1 ? "" : "s"} failing.` : "All checks passing."}
          {props.lastCheckedAt ? ` Last check ${new Date(props.lastCheckedAt).toLocaleTimeString()}.` : " Waiting for the first health poll."}
        </p>
        <Button tone="ghost" onClick={() => void props.onRunChecks()} disabled={props.busyAction !== null}>
          {props.busyAction === "refresh-health" ? "Checking" : "Run again"}
        </Button>
      </div>
      <CheckRows checks={props.checks} detailed />
      <div className="border-t-2 border-line pt-5">
        <Tag>Kill switch</Tag>
        <p className="mt-2 max-w-3xl text-[14px] text-stone">{props.killSwitch.detail}</p>
      </div>
    </div>
  );
}

/**
 * What the tunnel service was handed, in full — private key included. WireGuard key material is not
 * a secret here: the server it belongs to lives for hours (AGENTS.md §6).
 */
function ConfigTab(props: ShieldViewProps) {
  const { tunnelConfig } = props;
  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-wrap items-center justify-between gap-4">
        <p className="text-[15px] text-stone">What the tunnel service was handed, in full.</p>
        {props.onCopyConfig ? (
          <Button tone="ghost" onClick={() => void props.onCopyConfig?.()}>Copy</Button>
        ) : null}
      </div>
      {tunnelConfig.available ? (
        <>
          <pre className="scroll-area max-h-[420px] overflow-auto border-2 border-line bg-ink p-5 font-mono text-[13px] leading-6 text-bone">
            {tunnelConfig.configText}
          </pre>
          <div className="flex flex-wrap gap-x-6 gap-y-1 text-[12.5px] text-stone">
            {tunnelConfig.tunnelName ? <span>Tunnel <span className="font-mono text-bone">{tunnelConfig.tunnelName}</span></span> : null}
            {tunnelConfig.sourcePath ? <span className="break-all">Source <span className="font-mono text-bone">{tunnelConfig.sourcePath}</span></span> : null}
          </div>
        </>
      ) : (
        <div className="border-2 border-dashed border-line px-5 py-8 text-[14px] text-stone">{tunnelConfig.emptyReason}</div>
      )}
    </div>
  );
}

function LogTab(props: ShieldViewProps & { runs: ActivityRun[]; now: number }) {
  const { runs, now, diagnostics, busyAction } = props;
  const [filter, setFilter] = useState<ActivityTab>("all");
  const visible = useMemo(() => filterActivity(runs, filter), [runs, filter]);
  const existingNodeIds = useMemo(() => new Set(props.nodes.map((node) => node.id)), [props.nodes]);
  const latestFailure = useMemo(() => selectUnresolvedFailure(runs, existingNodeIds), [existingNodeIds, runs]);
  const failure = latestFailure ? failingStep(latestFailure) : null;
  const recovery = latestFailure ? describeRecovery(latestFailure) : null;
  const busy = busyAction !== null;

  return (
    <div className="flex flex-col gap-8">
      {/* A failed provision used to leave nothing on screen that said what went wrong. */}
      {latestFailure && failure ? (
        <div className="flex flex-wrap items-start justify-between gap-4 border-2 border-alert bg-alert-ink px-5 py-4">
          <div className="min-w-0">
            <Tag state="failed">Last failure · {formatRelativeTime(latestFailure.latestAt, now) ?? "unknown time"}</Tag>
            <p className="mt-2 max-w-3xl break-words text-[14px] text-bone">{failure.detail}</p>
            {recovery ? <p className="mt-1 max-w-3xl text-[13.5px] text-stone">{recovery.label}</p> : null}
          </div>
          {recovery?.action === "cleanup" && latestFailure.nodeId ? (
            <Button tone="destructive" disabled={busy} onClick={() => void props.onRecoverNode(latestFailure.nodeId as string)}>
              {busyAction === "force-cleanup" ? "Cleaning up" : "Force cleanup"}
            </Button>
          ) : null}
        </div>
      ) : null}

      <section>
        <div className="mb-4 flex flex-wrap items-center justify-between gap-4">
          <h3 className="font-mono text-[18px] text-bone">History</h3>
          <div className="flex flex-wrap gap-2">
            {ACTIVITY_TABS.map((option) => (
              <Button
                key={option.id}
                size="sm"
                tone={filter === option.id ? "ghost" : "quiet"}
                pressed={filter === option.id}
                onClick={() => setFilter(option.id)}
              >
                {option.label}
              </Button>
            ))}
          </div>
        </div>
        {visible.length === 0 ? (
          <div className="border-2 border-dashed border-line px-5 py-6 text-[14px] text-stone">
            {diagnostics === null
              ? "Loading history."
              : filter === "failures"
                ? "Nothing has failed. That is the intended state."
                : runs.length === 0
                  ? "Nothing recorded yet. Building a server or connecting will show up here."
                  : "No entries in this category."}
          </div>
        ) : (
          <div className="flex flex-col gap-2">
            {visible.map((run, index) => (
              // The newest run is opened by default when it failed: it is what the user came to read.
              <RunEntry key={run.id} run={run} headline={describeRunHeadline(run, now)} defaultOpen={index === 0 && run.outcome === "error"} />
            ))}
          </div>
        )}
      </section>

      <div className="grid grid-cols-2 gap-6 max-[1000px]:grid-cols-1">
        <Panel title="Report a problem" description="Export a snapshot to attach to a bug report.">
          <p className="text-[13.5px] leading-6 text-stone">
            Provider, server and tunnel records plus this history. Cloud API keys are stripped before it is written.
          </p>
          <div className="mt-4 flex flex-wrap gap-3">
            <Button onClick={() => void props.onExportDiagnostics()} disabled={busy}>
              {busyAction === "export-diagnostics" ? "Exporting" : "Export bundle"}
            </Button>
            <Button tone="ghost" onClick={() => void props.onCopyDiagnostics()} disabled={busy || !diagnostics}>
              {busyAction === "copy-diagnostics" ? "Copying" : "Copy"}
            </Button>
          </div>
          {props.diagnosticsExportPath ? (
            <div className="mt-4 border-2 border-line px-4 py-3">
              <Tag>Last export</Tag>
              <div className="mt-1 break-all font-mono text-[12px] text-bone">{props.diagnosticsExportPath}</div>
            </div>
          ) : null}
          <div className="mt-4 font-mono text-[12px] text-stone">
            WireGuard {props.readiness?.version ?? "version unknown"} · Blink {props.readiness?.app_version ?? "—"}
          </div>
          <details className="mt-4 border-2 border-line">
            <summary className="cursor-pointer px-4 py-3 text-[13.5px] text-stone">Raw bundle</summary>
            <pre className="scroll-area max-h-[360px] overflow-auto border-t-2 border-line bg-ink p-4 font-mono text-[12px] leading-6 text-bone">
              {diagnostics ? JSON.stringify(diagnostics, null, 2) : "Loading."}
            </pre>
          </details>
        </Panel>

        <Panel title="Where things are" description="The folders Blink writes to on this PC.">
          {props.locations.length === 0 ? (
            <p className="text-[14px] text-stone">Locations are still loading.</p>
          ) : (
            <div>
              {props.locations.map((location) => (
                <Row key={location.key} className="flex-wrap items-start">
                  <div className="min-w-0 flex-1">
                    <div className="text-[15px] text-bone">{location.label}</div>
                    <p className="text-[12.5px] text-stone">{location.description}</p>
                    <div className="mt-1 break-all font-mono text-[11.5px] text-stone">{location.path}</div>
                  </div>
                  <div className="flex gap-2">
                    <Button tone="quiet" size="sm" onClick={() => void props.onCopyText(location.path, `${location.label} path`)}>Copy path</Button>
                    <Button tone="quiet" size="sm" onClick={() => void props.onRevealLocation(location.key)} disabled={!location.exists}>Open</Button>
                  </div>
                </Row>
              ))}
            </div>
          )}
        </Panel>
      </div>
    </div>
  );
}
