/**
 * The game session that spans a whole wait: the build, the handshake, and as long after as the
 * user keeps playing.
 *
 * Held at module level rather than in a component so the same game — score, lives, the goat
 * mid-charge — survives Home swapping its build strip for HOLD ON, and survives a trip to the Map
 * and back. Nothing here is persisted; closing the app ends it. The best score goes to preferences
 * through the component, as before.
 */

import type { GameState } from "./invaders.ts";

type Session = { open: boolean; over: boolean; game: GameState | null };

let session: Session = { open: false, over: false, game: null };
const listeners = new Set<() => void>();

function emit() {
  for (const listener of listeners) listener();
}

export function subscribeSession(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

/** Snapshot for `useSyncExternalStore`; identity changes only when something changed. */
export function getSession(): Session {
  return session;
}

export function openSession(): void {
  if (session.open) return;
  session = { ...session, open: true };
  emit();
}

/** Ends the session and forgets the game: the next wait starts a fresh one. */
export function closeSession(): void {
  if (!session.open && session.game === null) return;
  session = { open: false, over: false, game: null };
  emit();
}

/** The live game, kept between ticks. Does not notify: the game draws itself every tick. */
export function saveGame(game: GameState): void {
  session.game = game;
}

/** Whether the current game has ended. Notifies, since it decides whether the screen stays. */
export function setGameOver(over: boolean): void {
  if (session.over === over) return;
  session = { ...session, over };
  emit();
}

/**
 * Whether Home should keep the waiting screen up.
 *
 * While anything is in flight, always. Once the wait is over, only while a game is still being
 * played: a finished game, or no game at all, hands Home back.
 */
export function keepWaitScreen(input: { waiting: boolean; sessionOpen: boolean; gameOver: boolean }): boolean {
  if (input.waiting) return true;
  return input.sessionOpen && !input.gameOver;
}
