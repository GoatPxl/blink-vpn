//! How far along a connect is, judged from evidence an unelevated app can actually read.
//!
//! Kept pure — no I/O, no `Backend`, no clock of its own — so every combination of evidence can be
//! asserted directly in a unit test. That matters more here than usual: this module's failure mode
//! is *claiming Connected too early*, which is precisely the bug it exists to fix.
//!
//! # Why `sc query` is not enough
//!
//! Connect used to report success the moment the Windows service reached `RUNNING`. But the
//! WireGuard adapter captures the default route as soon as it comes up, so at that instant the
//! user's traffic is already inside a tunnel whose peer has not answered. Observed in the field:
//!
//! ```text
//! 10:22:07.851  Startup complete             <- service RUNNING, old code said "Connected"
//! 10:22:23.051  Receiving handshake response <- traffic actually works here
//! ```
//!
//! For fifteen seconds the app showed green while the user had no internet.
//!
//! # Why the handshake cannot be the signal
//!
//! `wg show` reads the tunnel service's pipe, which is restricted to administrators (AGENTS.md
//! §4), so `last_handshake_age_seconds` is `None` for the whole session in a normal unelevated
//! run. A verdict that depended on it would be permanently stuck at "connecting".
//!
//! # What is actually load-bearing: `rx_bytes > 0`
//!
//! WireGuard discards anything it cannot decrypt and authenticate, so the adapter's inbound octet
//! counter only advances for payload that was decrypted with a live session key and injected into
//! the OS stack. **Non-zero inbound bytes cannot exist without a completed handshake.** The
//! counter comes from Win32 `GetIfTable2` — in-process, no elevation, no subprocess.
//!
//! The converse does not hold: a completed handshake whose only traffic so far is a keepalive
//! decrypts to a zero-length payload that is never injected, so `rx == 0` can briefly outlive a
//! real handshake. That is why zero maps to `Connecting`, and why a slow peer becomes `Stalled`
//! rather than an error.

use serde::Serialize;

use crate::backend::wireguard::TunnelServiceState;

/// What the user is actually experiencing, as opposed to what the service reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TunnelPhase {
  /// No tunnel service, or it is stopped. Traffic uses the normal connection.
  Disconnected,
  /// The service is up but nothing has come back through it. The tunnel already owns the default
  /// route, so the user has no working internet in this state.
  Connecting,
  /// Still `Connecting` past the point where a working server would have answered. Same
  /// user-visible symptom, but now worth surfacing an escape hatch for.
  Stalled,
  /// Traffic has provably crossed the tunnel.
  Connected,
  /// A handshake happened, but long enough ago that the link may be dead.
  Degraded,
}

impl TunnelPhase {
  /// Whether this phase should read as a live, usable tunnel anywhere in the UI.
  ///
  /// `Degraded` counts: the tunnel did carry traffic and may still, so presenting it as "not
  /// connected" would be wrong. It is surfaced as a warning, not as an outage.
  pub fn is_usable(self) -> bool {
    matches!(self, Self::Connected | Self::Degraded)
  }
}

/// Everything the verdict is allowed to depend on.
///
/// A struct rather than four positional arguments so a caller cannot silently transpose
/// `handshake_age_seconds` and `elapsed_seconds`, which are both `Option<u64>` and mean opposite
/// things.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhaseEvidence {
  pub service_state: TunnelServiceState,
  /// Age of the last handshake. `None` whenever the app is unelevated, which is the normal case —
  /// so its absence carries no information at all.
  pub handshake_age_seconds: Option<u64>,
  /// Decrypted inbound octets. `Some(0)` means measured and idle; `None` means unreadable. The
  /// distinction matters: only `Some(n > 0)` is proof of a handshake.
  pub rx_bytes: Option<u64>,
  /// Seconds since this app asked the service to start. `None` for a tunnel that was already
  /// running when we first looked, which has no knowable attempt time.
  pub elapsed_seconds: Option<u64>,
}

/// How long a running tunnel may carry nothing before it is called stalled rather than connecting.
///
/// Sized from the observed failure: service `RUNNING` at 10:22:07, first handshake response at
/// 10:22:23. A threshold under ~16s would label that healthy connect as stalled.
pub const STALL_AFTER_SECONDS: u64 = 20;

/// How old a handshake may be before the link is presumed dead.
///
/// WireGuard refreshes handshakes well inside this window whenever traffic flows, so exceeding it
/// means the peer has gone away rather than that the tunnel is idle.
pub const HANDSHAKE_STALE_AFTER_SECONDS: u64 = 180;

/// The single place a tunnel's phase is decided.
///
/// Order matters and is not arbitrary:
///
/// 1. `Stopped` is unambiguous.
/// 2. `Unknown` — an `sc query` we could not parse — is an *absence* of evidence, so it can never
///    resolve to `Connected`. The previous code had two contradictory answers for this same input:
///    connect treated it as success, and the next health poll treated it as an error.
/// 3. `Starting` is the service's own "not yet".
/// 4. A readable handshake outranks byte counters: when elevated, the protocol's own statement is
///    better evidence than a counter that lags it.
/// 5. Otherwise inbound bytes decide, per the module docs.
/// 6. With no positive evidence either way, elapsed time separates "still coming up" from "wedged"
///    — and without a clock, we decline to guess.
pub fn classify_tunnel_phase(evidence: &PhaseEvidence) -> TunnelPhase {
  match evidence.service_state {
    TunnelServiceState::Stopped => TunnelPhase::Disconnected,
    TunnelServiceState::Unknown => TunnelPhase::Connecting,
    TunnelServiceState::Starting => TunnelPhase::Connecting,
    TunnelServiceState::Running => classify_running(evidence),
  }
}

fn classify_running(evidence: &PhaseEvidence) -> TunnelPhase {
  // Elevated: the handshake is the most direct evidence available, so it settles the matter even
  // when no payload has moved yet.
  if let Some(age) = evidence.handshake_age_seconds {
    return if age <= HANDSHAKE_STALE_AFTER_SECONDS {
      TunnelPhase::Connected
    } else {
      TunnelPhase::Degraded
    };
  }

  // Unelevated: decrypted inbound payload is proof a handshake completed.
  if evidence.rx_bytes.is_some_and(|rx| rx > 0) {
    return TunnelPhase::Connected;
  }

  // Running, but nothing has come back. Only a known elapsed time can distinguish a connect that
  // is progressing normally from one that has hung. Without one — a tunnel adopted at startup —
  // report the honest "still connecting" rather than inventing a stall from an unknown baseline.
  match evidence.elapsed_seconds {
    Some(elapsed) if elapsed >= STALL_AFTER_SECONDS => TunnelPhase::Stalled,
    _ => TunnelPhase::Connecting,
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  /// Every case names one exact expected variant. `matches!(x, Connected | Connecting)` would pass
  /// for almost any input and would not catch this classifier's actual failure mode — saying
  /// `Connected` too eagerly (AGENTS.md §9).
  fn evidence(
    service_state: TunnelServiceState,
    handshake_age_seconds: Option<u64>,
    rx_bytes: Option<u64>,
    elapsed_seconds: Option<u64>,
  ) -> PhaseEvidence {
    PhaseEvidence {
      service_state,
      handshake_age_seconds,
      rx_bytes,
      elapsed_seconds,
    }
  }

  #[test]
  fn stopped_service_is_disconnected() {
    let phase = classify_tunnel_phase(&evidence(TunnelServiceState::Stopped, None, None, Some(2)));
    assert_eq!(phase, TunnelPhase::Disconnected);
  }

  #[test]
  fn stopped_service_is_disconnected_even_with_stale_counters() {
    // Counters are lifetime totals and survive the service stopping. They must not resurrect it.
    let phase = classify_tunnel_phase(&evidence(
      TunnelServiceState::Stopped,
      Some(5),
      Some(9_999),
      Some(2),
    ));
    assert_eq!(phase, TunnelPhase::Disconnected);
  }

  #[test]
  fn unparseable_service_state_is_connecting_never_connected() {
    // Guards the removed `Unknown => TunnelStatus::Connected` mapping.
    let phase = classify_tunnel_phase(&evidence(TunnelServiceState::Unknown, None, None, Some(2)));
    assert_eq!(phase, TunnelPhase::Connecting);
  }

  #[test]
  fn unparseable_service_state_stays_connecting_past_the_stall_threshold() {
    let phase = classify_tunnel_phase(&evidence(TunnelServiceState::Unknown, None, None, Some(99)));
    assert_eq!(phase, TunnelPhase::Connecting);
  }

  #[test]
  fn start_pending_is_connecting() {
    let phase = classify_tunnel_phase(&evidence(TunnelServiceState::Starting, None, None, Some(2)));
    assert_eq!(phase, TunnelPhase::Connecting);
  }

  #[test]
  fn running_with_no_evidence_is_connecting() {
    let phase = classify_tunnel_phase(&evidence(TunnelServiceState::Running, None, None, Some(2)));
    assert_eq!(phase, TunnelPhase::Connecting);
  }

  #[test]
  fn running_with_zero_inbound_bytes_is_connecting() {
    // The dead window the user reported: adapter up, routes captured, peer has not answered.
    let phase =
      classify_tunnel_phase(&evidence(TunnelServiceState::Running, None, Some(0), Some(2)));
    assert_eq!(phase, TunnelPhase::Connecting);
  }

  #[test]
  fn running_with_inbound_bytes_is_connected_without_elevation() {
    // The load-bearing unelevated verdict.
    let phase =
      classify_tunnel_phase(&evidence(TunnelServiceState::Running, None, Some(1), Some(2)));
    assert_eq!(phase, TunnelPhase::Connected);
  }

  #[test]
  fn running_with_zero_inbound_bytes_past_threshold_is_stalled() {
    let phase = classify_tunnel_phase(&evidence(
      TunnelServiceState::Running,
      None,
      Some(0),
      Some(STALL_AFTER_SECONDS),
    ));
    assert_eq!(phase, TunnelPhase::Stalled);
  }

  #[test]
  fn running_with_unreadable_counters_past_threshold_is_stalled() {
    let phase = classify_tunnel_phase(&evidence(
      TunnelServiceState::Running,
      None,
      None,
      Some(STALL_AFTER_SECONDS + 5),
    ));
    assert_eq!(phase, TunnelPhase::Stalled);
  }

  #[test]
  fn stall_threshold_boundary_is_inclusive() {
    let just_under = classify_tunnel_phase(&evidence(
      TunnelServiceState::Running,
      None,
      Some(0),
      Some(STALL_AFTER_SECONDS - 1),
    ));
    let exactly = classify_tunnel_phase(&evidence(
      TunnelServiceState::Running,
      None,
      Some(0),
      Some(STALL_AFTER_SECONDS),
    ));

    assert_eq!(just_under, TunnelPhase::Connecting);
    assert_eq!(exactly, TunnelPhase::Stalled);
  }

  #[test]
  fn fresh_handshake_outranks_zero_inbound_bytes() {
    // Elevated run: the protocol says the session is live even though no payload has arrived.
    let phase = classify_tunnel_phase(&evidence(
      TunnelServiceState::Running,
      Some(2),
      Some(0),
      Some(2),
    ));
    assert_eq!(phase, TunnelPhase::Connected);
  }

  #[test]
  fn fresh_handshake_outranks_the_stall_threshold() {
    let phase = classify_tunnel_phase(&evidence(
      TunnelServiceState::Running,
      Some(2),
      Some(0),
      Some(STALL_AFTER_SECONDS + 60),
    ));
    assert_eq!(phase, TunnelPhase::Connected);
  }

  #[test]
  fn stale_handshake_is_degraded() {
    let phase = classify_tunnel_phase(&evidence(
      TunnelServiceState::Running,
      Some(HANDSHAKE_STALE_AFTER_SECONDS + 1),
      Some(9_999),
      Some(600),
    ));
    assert_eq!(phase, TunnelPhase::Degraded);
  }

  #[test]
  fn handshake_staleness_boundary_is_inclusive() {
    let fresh = classify_tunnel_phase(&evidence(
      TunnelServiceState::Running,
      Some(HANDSHAKE_STALE_AFTER_SECONDS),
      None,
      None,
    ));
    let stale = classify_tunnel_phase(&evidence(
      TunnelServiceState::Running,
      Some(HANDSHAKE_STALE_AFTER_SECONDS + 1),
      None,
      None,
    ));

    assert_eq!(fresh, TunnelPhase::Connected);
    assert_eq!(stale, TunnelPhase::Degraded);
  }

  #[test]
  fn running_with_inbound_bytes_is_connected_without_a_clock() {
    // A tunnel already up when the app started: no attempt time, but proof of traffic.
    let phase = classify_tunnel_phase(&evidence(TunnelServiceState::Running, None, Some(4_096), None));
    assert_eq!(phase, TunnelPhase::Connected);
  }

  #[test]
  fn running_without_a_clock_is_connecting_never_stalled() {
    // Asserting a stall from an unknown baseline would be a lie, however long ago it really was.
    let phase = classify_tunnel_phase(&evidence(TunnelServiceState::Running, None, None, None));
    assert_eq!(phase, TunnelPhase::Connecting);
  }

  #[test]
  fn zero_inbound_without_a_clock_is_connecting_never_stalled() {
    let phase = classify_tunnel_phase(&evidence(TunnelServiceState::Running, None, Some(0), None));
    assert_eq!(phase, TunnelPhase::Connecting);
  }

  #[test]
  fn phase_serializes_lowercase_for_the_ipc_boundary() {
    // The TypeScript union is a flat lowercase string; a PascalCase leak here breaks it silently.
    let json = serde_json::to_string(&TunnelPhase::Connecting).expect("serialize phase");
    assert_eq!(json, "\"connecting\"");
  }

  #[test]
  fn only_connected_and_degraded_count_as_usable() {
    assert!(TunnelPhase::Connected.is_usable());
    assert!(TunnelPhase::Degraded.is_usable());
    assert!(!TunnelPhase::Connecting.is_usable());
    assert!(!TunnelPhase::Stalled.is_usable());
    assert!(!TunnelPhase::Disconnected.is_usable());
  }

}
