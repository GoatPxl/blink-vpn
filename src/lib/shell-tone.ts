/**
 * The one colour the app wears at any moment: rail logo, active rail item, and the tray icon.
 *
 * Four states, each one signal colour, so the app is readable from the taskbar without opening it.
 * Kept pure so the rail and the tray can never disagree about what state the app is in.
 */

import type { TunnelPhase } from "./tauri";

export type ShellTone = "secure" | "busy" | "off" | "failed";

export type ShellToneInput = {
  tunnelPhase: TunnelPhase;
  /** A connect is in flight: the user is in HOLD ON. */
  connecting: boolean;
  /** A provision, destroy or cleanup is in flight. */
  operating: boolean;
  /** The last operation failed and the user has not dismissed it. */
  failed: boolean;
};

/**
 * Precedence is deliberate.
 *
 * Work in progress wins, because during a connect the user has no internet and must be told why.
 * A secure tunnel then outranks a stale failure: a rejected key edit does not make the tunnel
 * unsafe, and painting the rail red over a working tunnel is the louder lie. A stalled tunnel is a
 * failure on its own evidence.
 */
export function deriveShellTone(input: ShellToneInput): ShellTone {
  if (input.connecting || input.operating) return "busy";
  if (input.tunnelPhase === "connected" || input.tunnelPhase === "degraded") return "secure";
  if (input.tunnelPhase === "stalled" || input.failed) return "failed";
  if (input.tunnelPhase === "connecting") return "busy";
  return "off";
}

/** CSS colour for a tone. */
export const SHELL_TONE_COLOR: Record<ShellTone, string> = {
  secure: "var(--lime)",
  busy: "var(--ember)",
  off: "var(--stone-2)",
  failed: "var(--alert)",
};
