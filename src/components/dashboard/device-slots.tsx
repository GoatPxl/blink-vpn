"use client";

import { useEffect, useState } from "react";

import { buildQrPath } from "@/lib/qr-display";
import type { NodePeerRecord, PeerConfigPayload } from "@/lib/tauri";

import { Button, Field, Modal, type DashboardBusyAction } from "./shared";

/**
 * The device slots baked into a server.
 *
 * A node is provisioned with a fixed set of `[Peer]` slots and can never gain more — the app cannot
 * reach a server once it exists, so the slot list it boots with is final. Adding a device is
 * therefore a purely local act: name one of the slots that is already there.
 *
 * Renders nothing when a node has no slot above 0. Those nodes were provisioned before multi-peer
 * support, carry a single peer for this PC, and would need to be recreated to gain any.
 */
export function DeviceSlots({
  nodeId,
  peers,
  busyAction,
  onClaimDevice,
  onLoadDeviceConfig,
  onExportDeviceConfig,
  onCopyText,
}: {
  nodeId: string;
  peers: NodePeerRecord[];
  busyAction: DashboardBusyAction;
  onClaimDevice: (nodeId: string, peerId: string, label: string) => Promise<boolean>;
  onLoadDeviceConfig: (nodeId: string, peerId: string) => Promise<PeerConfigPayload | null>;
  onExportDeviceConfig: (nodeId: string, peerId: string) => Promise<void>;
  onCopyText: (value: string, label: string) => Promise<void>;
}) {
  const [claimingPeerId, setClaimingPeerId] = useState<string | null>(null);
  const [draftLabel, setDraftLabel] = useState("");
  const [qrPeer, setQrPeer] = useState<NodePeerRecord | null>(null);

  const deviceSlots = peers.filter((peer) => peer.slot_index > 0);

  if (deviceSlots.length === 0) {
    return null;
  }

  const claimed = deviceSlots.filter((peer) => peer.label !== null);
  const free = deviceSlots.filter((peer) => peer.label === null);

  async function confirmClaim(peer: NodePeerRecord) {
    const ok = await onClaimDevice(nodeId, peer.id, draftLabel);
    if (ok) {
      setClaimingPeerId(null);
      setDraftLabel("");
      setQrPeer({ ...peer, label: draftLabel.trim() });
    }
  }

  return (
    <section className="mt-6">
      <div className="text-[11px] uppercase tracking-[0.24em] text-[var(--on-surface-variant)]">
        Devices
      </div>
      <p className="mt-2 text-sm text-[var(--on-surface-variant)]">
        Scan a device&apos;s code with the official WireGuard app on a phone or tablet.{" "}
        {free.length} of {deviceSlots.length} slots free.
      </p>

      <ul className="mt-3 space-y-2">
        {claimed.map((peer) => (
          <li
            key={peer.id}
            className="flex items-center justify-between gap-3 rounded-[14px] border border-[color:var(--outline-variant)] bg-[color:var(--surface-container)] px-3 py-2"
          >
            <div className="min-w-0">
              <div className="truncate text-sm text-[var(--on-surface)]">{peer.label}</div>
              <div className="font-mono text-[11px] text-[var(--on-surface-variant)]">
                {peer.client_address_cidr}
              </div>
            </div>
            <Button tone="secondary" className="px-3 py-2 text-xs" onClick={() => setQrPeer(peer)}>
              Show code
            </Button>
          </li>
        ))}
      </ul>

      {free.length > 0 ? (
        claimingPeerId ? (
          <div className="mt-3 rounded-[14px] border border-[color:var(--outline-variant)] bg-[color:var(--surface-container)] p-3">
            <Field label="Device name">
              <input
                autoFocus
                value={draftLabel}
                maxLength={40}
                placeholder="Pixel 9"
                onChange={(event) => setDraftLabel(event.target.value)}
                className="w-full rounded-[10px] border border-[color:var(--outline-variant)] bg-[color:var(--surface)] px-3 py-2 text-sm text-[var(--on-surface)]"
              />
            </Field>
            <div className="mt-3 flex items-center justify-end gap-2">
              <Button
                tone="secondary"
                className="px-3 py-2 text-xs"
                onClick={() => {
                  setClaimingPeerId(null);
                  setDraftLabel("");
                }}
              >
                Cancel
              </Button>
              <Button
                className="px-3 py-2 text-xs"
                disabled={draftLabel.trim() === "" || busyAction !== null}
                onClick={() => {
                  const peer = free.find((candidate) => candidate.id === claimingPeerId);
                  if (peer) {
                    void confirmClaim(peer);
                  }
                }}
              >
                {busyAction === "claim-device" ? "Adding" : "Add device"}
              </Button>
            </div>
          </div>
        ) : (
          <Button
            tone="secondary"
            className="mt-3 px-3 py-2 text-xs"
            onClick={() => setClaimingPeerId(free[0].id)}
          >
            Add device
          </Button>
        )
      ) : (
        <p className="mt-3 text-xs text-[var(--on-surface-variant)]">
          Every slot on this server is in use. Slots are fixed when a server is created, so another
          device needs a new server.
        </p>
      )}

      {qrPeer ? (
        <DeviceQrModal
          nodeId={nodeId}
          peer={qrPeer}
          busyAction={busyAction}
          onClose={() => setQrPeer(null)}
          onLoadDeviceConfig={onLoadDeviceConfig}
          onExportDeviceConfig={onExportDeviceConfig}
          onCopyText={onCopyText}
        />
      ) : null}
    </section>
  );
}

function DeviceQrModal({
  nodeId,
  peer,
  busyAction,
  onClose,
  onLoadDeviceConfig,
  onExportDeviceConfig,
  onCopyText,
}: {
  nodeId: string;
  peer: NodePeerRecord;
  busyAction: DashboardBusyAction;
  onClose: () => void;
  onLoadDeviceConfig: (nodeId: string, peerId: string) => Promise<PeerConfigPayload | null>;
  onExportDeviceConfig: (nodeId: string, peerId: string) => Promise<void>;
  onCopyText: (value: string, label: string) => Promise<void>;
}) {
  const [payload, setPayload] = useState<PeerConfigPayload | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    let cancelled = false;

    void (async () => {
      const loaded = await onLoadDeviceConfig(nodeId, peer.id);
      if (!cancelled) {
        setPayload(loaded);
        setLoading(false);
      }
    })();

    // The config is dropped when the dialog closes rather than kept around: nothing else in the UI
    // has a use for it, and re-reading it is one IPC call.
    return () => {
      cancelled = true;
    };
  }, [nodeId, peer.id, onLoadDeviceConfig]);

  const qrPath = payload ? buildQrPath(payload.qr_modules, payload.qr_size) : "";

  return (
    <Modal onClose={onClose} labelledBy="device-qr-title">
      <>
        <h2
          id="device-qr-title"
          className="text-xl font-semibold tracking-tight text-[var(--on-surface)]"
        >
          {peer.label}
        </h2>
        <p className="mt-1 text-sm text-[var(--on-surface-variant)]">
          In the official WireGuard app, choose Add tunnel, then Scan from QR code.
        </p>

        <div className="mt-5 flex justify-center">
          {loading ? (
            <div className="flex h-[260px] w-[260px] items-center justify-center rounded-[16px] border border-dashed border-[color:var(--outline-variant)] text-sm text-[var(--on-surface-variant)]">
              Building code
            </div>
          ) : qrPath ? (
            // White quiet zone regardless of theme: scanners expect dark-on-light with a margin,
            // and a code drawn straight onto the dark surface reads poorly or not at all.
            <svg
              viewBox={`-2 -2 ${payload!.qr_size + 4} ${payload!.qr_size + 4}`}
              role="img"
              aria-label={`WireGuard configuration for ${peer.label}`}
              className="h-[260px] w-[260px] rounded-[16px] bg-white p-2"
              shapeRendering="crispEdges"
            >
              <path d={qrPath} fill="#000000" />
            </svg>
          ) : (
            <div className="flex h-[260px] w-[260px] items-center justify-center rounded-[16px] border border-dashed border-[color:rgba(255,107,107,0.4)] px-4 text-center text-sm text-[var(--danger)]">
              The code could not be built. Copy or save the configuration instead.
            </div>
          )}
        </div>

        <p className="mt-4 text-xs text-[var(--on-surface-variant)]">
          This code contains the device&apos;s private key, so anyone who scans it can use this
          server. The server is destroyed with the node.
        </p>

        <div className="mt-6 flex items-center justify-end gap-2">
          <Button tone="secondary" onClick={onClose}>
            Close
          </Button>
          <Button
            tone="secondary"
            disabled={!payload}
            onClick={() => {
              if (payload) {
                void onCopyText(payload.config_text, "Configuration");
              }
            }}
          >
            Copy
          </Button>
          <Button
            disabled={busyAction !== null}
            onClick={() => void onExportDeviceConfig(nodeId, peer.id)}
          >
            {busyAction === "export-device-config" ? "Saving" : "Save .conf"}
          </Button>
        </div>
      </>
    </Modal>
  );
}
