"use client";

import Link from "next/link";
import { useEffect, useState } from "react";

import {
  getPeerConfig,
  listNodePeers,
  toCommandError,
  type NodePeerRecord,
  type PeerConfigPayload,
} from "@/lib/tauri";

import { DeviceQrModal, QrCode } from "./device-slots";
import { VIEW_ROUTE, type DashboardBusyAction, type VpnNodeRecord } from "./shared";

type DeviceTile = {
  peer: NodePeerRecord;
  payload: PeerConfigPayload | null;
};

type LoadState =
  | { kind: "loading" }
  | { kind: "loaded"; tiles: DeviceTile[] }
  /** The server can hand out no device config at all, e.g. one only this PC can reach. */
  | { kind: "unavailable"; message: string };

/**
 * QR codes for the current server's mobile devices, ready to scan from the dashboard.
 *
 * Devices saved in Settings are named onto a new server's spare slots when it is created, so this
 * card is where they show up with no further steps. It loads its own peers rather than reading the
 * provider's `nodePeers`, which follows whichever node the Nodes page has selected.
 *
 * Hidden for a server with no spare slots — one provisioned before multi-peer support.
 */
export function MobileDevicesCard({
  node,
  busyAction,
  onLoadDeviceConfig,
  onExportDeviceConfig,
  onCopyText,
}: {
  node: VpnNodeRecord;
  busyAction: DashboardBusyAction;
  onLoadDeviceConfig: (nodeId: string, peerId: string) => Promise<PeerConfigPayload | null>;
  onExportDeviceConfig: (nodeId: string, peerId: string) => Promise<void>;
  onCopyText: (value: string, label: string) => Promise<void>;
}) {
  const [state, setState] = useState<LoadState & { nodeId: string } | null>(null);
  const [hasSpareSlots, setHasSpareSlots] = useState(true);
  const [enlarged, setEnlarged] = useState<NodePeerRecord | null>(null);
  const ready = node.lifecycle_status === "ready" || node.lifecycle_status === "connected";

  useEffect(() => {
    if (!ready) return;
    let cancelled = false;

    void (async () => {
      let peers: NodePeerRecord[];
      try {
        peers = await listNodePeers(node.id);
      } catch {
        if (!cancelled) setHasSpareSlots(false);
        return;
      }
      if (cancelled) return;

      setHasSpareSlots(peers.some((peer) => peer.slot_index > 0));
      const devices = peers.filter((peer) => peer.slot_index > 0 && peer.label !== null);

      // Read one by one, not in parallel: each read touches Credential Manager, and the first
      // refusal says the same thing about every slot on this server.
      const tiles: DeviceTile[] = [];
      for (const peer of devices) {
        try {
          tiles.push({ peer, payload: await getPeerConfig({ nodeId: node.id, peerId: peer.id }) });
        } catch (error) {
          const commandError = toCommandError(error);
          if (commandError.kind === "invalid-input") {
            if (!cancelled) setState({ nodeId: node.id, kind: "unavailable", message: commandError.message });
            return;
          }
          tiles.push({ peer, payload: null });
        }
        if (cancelled) return;
      }

      if (!cancelled) setState({ nodeId: node.id, kind: "loaded", tiles });
    })();

    return () => {
      cancelled = true;
    };
  }, [node.id, ready]);

  if (!ready || !hasSpareSlots) return null;

  // A result for a previous server is never shown for this one.
  const current: LoadState = state && state.nodeId === node.id ? state : { kind: "loading" };

  return (
    <section
      data-testid="mobile-devices-card"
      className="rounded-[10px] border border-[color:var(--outline-variant)] bg-[linear-gradient(180deg,rgba(15,18,26,0.98),rgba(15,18,26,0.98))] px-5 py-4"
    >
      <div className="flex items-start justify-between gap-4">
        <div>
          <div className="text-sm text-[var(--on-surface-variant)]">Mobile devices</div>
          <p className="mt-1 text-xs text-[var(--on-surface-variant)]">
            In the official WireGuard app, choose Add tunnel, then Scan from QR code.
          </p>
        </div>
        <Link
          href={VIEW_ROUTE.settings}
          className="shrink-0 text-xs text-[var(--primary)] hover:underline"
        >
          Manage devices
        </Link>
      </div>

      {current.kind === "loading" ? (
        <p className="mt-4 text-sm text-[var(--on-surface-variant)]">Building codes…</p>
      ) : current.kind === "unavailable" ? (
        <p className="mt-4 text-sm text-[var(--on-surface-variant)]">{current.message}</p>
      ) : current.tiles.length === 0 ? (
        <p className="mt-4 text-sm text-[var(--on-surface-variant)]">
          No devices on this server. Add your devices in Settings and they will be ready on your next
          server, or add one to this server from its details on the Nodes page.
        </p>
      ) : (
        <ul className="mt-4 flex flex-wrap gap-4">
          {current.tiles.map(({ peer, payload }) => (
            <li key={peer.id}>
              <button
                type="button"
                onClick={() => setEnlarged(peer)}
                className="flex w-[152px] flex-col items-center gap-2 rounded-[14px] p-1 text-center hover:bg-[color:rgba(255,255,255,0.04)]"
                title={`Enlarge the code for ${peer.label}`}
              >
                {payload ? (
                  <QrCode
                    payload={payload}
                    label={`WireGuard configuration for ${peer.label}`}
                    className="h-[144px] w-[144px]"
                  />
                ) : (
                  <span className="flex h-[144px] w-[144px] items-center justify-center rounded-[16px] border border-dashed border-[color:var(--outline-variant)] px-3 text-xs text-[var(--on-surface-variant)]">
                    Code unavailable
                  </span>
                )}
                <span className="w-full truncate text-sm text-[var(--on-surface)]">{peer.label}</span>
              </button>
            </li>
          ))}
        </ul>
      )}

      {enlarged ? (
        <DeviceQrModal
          nodeId={node.id}
          peer={enlarged}
          busyAction={busyAction}
          onClose={() => setEnlarged(null)}
          onLoadDeviceConfig={onLoadDeviceConfig}
          onExportDeviceConfig={onExportDeviceConfig}
          onCopyText={onCopyText}
        />
      ) : null}
    </section>
  );
}
