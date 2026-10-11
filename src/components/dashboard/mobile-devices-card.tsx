"use client";

import { useEffect, useMemo, useState } from "react";

import { PixelIcon } from "@/components/ui/pixel-icon";
import {
  getPeerConfig,
  listNodePeers,
  toCommandError,
  type NodePeerRecord,
  type PeerConfigPayload,
} from "@/lib/tauri";

import { DeviceQrModal, QrCode } from "./device-slots";
import { Button, Modal, Panel, type DashboardBusyAction, type VpnNodeRecord } from "./shared";

type DeviceTile = {
  peer: NodePeerRecord;
  payload: PeerConfigPayload | null;
};

type CodesState =
  | { kind: "loading" }
  | { kind: "loaded"; tiles: DeviceTile[] }
  /** The server can hand out no device config at all, e.g. one only this PC can reach. */
  | { kind: "unavailable"; message: string };

/** "Pixel 8 Pro", "Pixel 8 Pro and Pixel 10a", "A, B and C". */
function joinNames(names: string[]): string {
  if (names.length <= 1) return names[0] ?? "";
  return `${names.slice(0, -1).join(", ")} and ${names[names.length - 1]}`;
}

/**
 * Which phones can use the current server, with their QR codes one click away.
 *
 * Devices saved in Setup are named onto a new server's spare slots when it is created, so this card
 * is where they show up with no further steps. It loads its own peers rather than reading the
 * provider's `nodePeers`, which follows whichever node the server list has selected.
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
  const [peers, setPeers] = useState<{ nodeId: string; list: NodePeerRecord[] } | null>(null);
  const [codesOpen, setCodesOpen] = useState(false);
  const ready = node.lifecycle_status === "ready" || node.lifecycle_status === "connected";

  useEffect(() => {
    if (!ready) return;
    let cancelled = false;
    void listNodePeers(node.id)
      .then((list) => {
        if (!cancelled) setPeers({ nodeId: node.id, list });
      })
      .catch(() => {
        if (!cancelled) setPeers({ nodeId: node.id, list: [] });
      });
    return () => {
      cancelled = true;
    };
  }, [node.id, ready]);

  // A result for a previous server is never shown for this one.
  const list = peers && peers.nodeId === node.id ? peers.list : null;
  // Memoised on the loaded list: the codes dialog reloads whenever this array changes identity.
  const spareSlots = useMemo(() => list?.filter((peer) => peer.slot_index > 0) ?? [], [list]);
  const devices = useMemo(() => spareSlots.filter((peer) => peer.label !== null), [spareSlots]);
  if (!ready || (list !== null && spareSlots.length === 0)) return null;

  const free = spareSlots.length - devices.length;
  const names = devices.map((peer) => peer.label as string);

  return (
    <Panel bodyClassName="flex items-center gap-6 px-8 py-5">
      <div data-testid="mobile-devices-card" className="flex min-w-0 flex-1 items-center gap-6">
        <PixelIcon name="phone" className="h-8 w-8 shrink-0 text-bone" />
        <div className="min-w-0 flex-1">
          <div className="truncate text-[18px] font-semibold text-bone">
            {list === null
              ? "Phones"
              : names.length > 0
                ? `${joinNames(names)} can use this server`
                : "No phones on this server yet"}
          </div>
          <p className="text-[14px] text-stone">
            {names.length > 0 ? "Scan a code once per server" : "Name your phones in Setup and the next server has their codes"}
            {list !== null ? ` · ${free} spare ${free === 1 ? "slot" : "slots"}` : ""}
          </p>
        </div>
      </div>
      <Button tone="ghost" onClick={() => setCodesOpen(true)} disabled={devices.length === 0}>
        Show codes
      </Button>

      {codesOpen ? (
        <DeviceCodesDialog
          node={node}
          devices={devices}
          busyAction={busyAction}
          onClose={() => setCodesOpen(false)}
          onLoadDeviceConfig={onLoadDeviceConfig}
          onExportDeviceConfig={onExportDeviceConfig}
          onCopyText={onCopyText}
        />
      ) : null}
    </Panel>
  );
}

/** Every claimed device's QR code at once, each enlargeable. Loads only when opened. */
function DeviceCodesDialog({
  node,
  devices,
  busyAction,
  onClose,
  onLoadDeviceConfig,
  onExportDeviceConfig,
  onCopyText,
}: {
  node: VpnNodeRecord;
  devices: NodePeerRecord[];
  busyAction: DashboardBusyAction;
  onClose: () => void;
  onLoadDeviceConfig: (nodeId: string, peerId: string) => Promise<PeerConfigPayload | null>;
  onExportDeviceConfig: (nodeId: string, peerId: string) => Promise<void>;
  onCopyText: (value: string, label: string) => Promise<void>;
}) {
  const [state, setState] = useState<CodesState>({ kind: "loading" });
  const [enlarged, setEnlarged] = useState<NodePeerRecord | null>(null);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      // Read one by one, not in parallel: each read touches Credential Manager, and the first
      // refusal says the same thing about every slot on this server.
      const tiles: DeviceTile[] = [];
      for (const peer of devices) {
        try {
          tiles.push({ peer, payload: await getPeerConfig({ nodeId: node.id, peerId: peer.id }) });
        } catch (error) {
          const commandError = toCommandError(error);
          if (commandError.kind === "invalid-input") {
            if (!cancelled) setState({ kind: "unavailable", message: commandError.message });
            return;
          }
          tiles.push({ peer, payload: null });
        }
        if (cancelled) return;
      }
      if (!cancelled) setState({ kind: "loaded", tiles });
    })();
    return () => {
      cancelled = true;
    };
  }, [devices, node.id]);

  if (enlarged) {
    return (
      <DeviceQrModal
        nodeId={node.id}
        peer={enlarged}
        busyAction={busyAction}
        onClose={() => setEnlarged(null)}
        onLoadDeviceConfig={onLoadDeviceConfig}
        onExportDeviceConfig={onExportDeviceConfig}
        onCopyText={onCopyText}
      />
    );
  }

  return (
    <Modal onClose={onClose} labelledBy="device-codes-title" width="lg">
      <div className="flex items-start justify-between gap-4 border-b-2 border-line pb-4">
        <div>
          <h2 id="device-codes-title" className="font-display text-[18px] uppercase text-bone">Phone codes</h2>
          <p className="mt-1 text-[13.5px] text-stone">
            In the official WireGuard app, choose Add tunnel, then Scan from QR code.
          </p>
        </div>
        <Button tone="quiet" size="sm" onClick={onClose} label="Close">
          <PixelIcon name="close" className="h-3 w-3" />
        </Button>
      </div>

      <div className="pt-5">
        {state.kind === "loading" ? (
          <p className="text-[14px] text-stone">Building codes…</p>
        ) : state.kind === "unavailable" ? (
          <p className="text-[14px] text-stone">{state.message}</p>
        ) : (
          <ul className="flex flex-wrap gap-5">
            {state.tiles.map(({ peer, payload }) => (
              <li key={peer.id}>
                <button
                  type="button"
                  onClick={() => setEnlarged(peer)}
                  className="flex w-[176px] flex-col items-center gap-2 border-2 border-line p-2 text-center hover:border-lime"
                  title={`Enlarge the code for ${peer.label}`}
                >
                  {payload ? (
                    <QrCode payload={payload} label={`WireGuard configuration for ${peer.label}`} className="h-[156px] w-[156px]" />
                  ) : (
                    <span className="flex h-[156px] w-[156px] items-center justify-center border-2 border-dashed border-line-3 px-3 text-[12.5px] text-stone">
                      Code unavailable
                    </span>
                  )}
                  <span className="w-full truncate text-[14px] text-bone">{peer.label}</span>
                </button>
              </li>
            ))}
          </ul>
        )}
      </div>
    </Modal>
  );
}
