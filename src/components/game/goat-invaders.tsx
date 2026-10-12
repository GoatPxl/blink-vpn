"use client";

import { useCallback, useEffect, useRef, useState, type KeyboardEvent } from "react";
import {
  FIELD_HEIGHT,
  FIELD_WIDTH,
  NO_INPUT,
  START_LIVES,
  newGame,
  renderCells,
  step,
  type CellKind,
  type GameState,
} from "@/lib/game/invaders";
import { getSession, saveGame } from "@/lib/game/session";

/** 20 ticks a second: smooth enough for blocks, cheap enough to run beside a provision. */
const TICK_MS = 50;
const CELL_GAP = 2;

/** Token colours, read once from CSS so the canvas matches the theme without duplicating hex. */
function readColors(element: HTMLElement): Record<CellKind | "empty", string> {
  const style = getComputedStyle(element);
  const token = (name: string) => style.getPropertyValue(name).trim();
  return {
    empty: token("--panel-2"),
    goat: token("--stone"),
    "goat-front": token("--bone"),
    "goat-pawing": token("--ember"),
    "goat-charging": token("--alert"),
    shot: token("--lime"),
    spit: token("--bone"),
    player: token("--lime"),
  };
}

type Hud = { score: number; lives: number; status: GameState["status"] };

/**
 * The game shown while a server builds and its tunnel comes up. Optional, local, and disposable.
 *
 * Plays only while it has keyboard focus, and pauses when the window loses focus or is hidden — the
 * UAC prompt does both — so it never eats keystrokes meant for anything else. Its state lives in the
 * session store, so the same game carries on from the build into HOLD ON and survives a trip to
 * another view. The waiting screen renders it inside its own error boundary, so a fault here cannot
 * hide the build or the handshake.
 */
export function GoatInvaders({
  highScore,
  onGameOver,
  onStatusChange,
  onHide,
}: {
  highScore: number;
  /** Called once per finished game with its score, so the caller can keep the best. */
  onGameOver: (score: number) => void;
  /** Called when the game ends or restarts, so the screen around it knows. */
  onStatusChange?: (status: GameState["status"]) => void;
  onHide: () => void;
}) {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const frameRef = useRef<HTMLDivElement | null>(null);
  // The session's game if one is running, else a new one. Seeded lazily: a render must not read
  // the clock.
  const [firstGame] = useState(() => getSession().game ?? newGame(Date.now() | 0));
  const gameRef = useRef<GameState>(firstGame);
  const keysRef = useRef({ left: false, right: false, fire: false });
  const reportedRef = useRef(firstGame.status === "over");
  const [focused, setFocused] = useState(false);
  const [hud, setHud] = useState<Hud>({ score: firstGame.score, lives: firstGame.lives, status: firstGame.status });

  useEffect(() => {
    saveGame(gameRef.current);
  }, []);

  useEffect(() => {
    onStatusChange?.(hud.status);
  }, [hud.status, onStatusChange]);

  const draw = useCallback(() => {
    const canvas = canvasRef.current;
    const context = canvas?.getContext("2d");
    if (!canvas || !context) return;

    const width = canvas.clientWidth;
    const cell = Math.max(4, Math.floor((width + CELL_GAP) / FIELD_WIDTH) - CELL_GAP);
    const pitch = cell + CELL_GAP;
    const pixelWidth = FIELD_WIDTH * pitch - CELL_GAP;
    const pixelHeight = FIELD_HEIGHT * pitch - CELL_GAP;
    if (canvas.width !== pixelWidth || canvas.height !== pixelHeight) {
      canvas.width = pixelWidth;
      canvas.height = pixelHeight;
    }

    const colors = readColors(canvas);
    const cells = renderCells(gameRef.current);
    context.clearRect(0, 0, pixelWidth, pixelHeight);
    for (let y = 0; y < FIELD_HEIGHT; y += 1) {
      for (let x = 0; x < FIELD_WIDTH; x += 1) {
        const kind = cells.get(y * FIELD_WIDTH + x);
        context.fillStyle = kind ? colors[kind] : colors.empty;
        context.globalAlpha = kind ? 1 : 0.35;
        context.fillRect(x * pitch, y * pitch, cell, cell);
      }
    }
    context.globalAlpha = 1;

    // "+100" over a goat stopped mid-charge, drifting up as it fades.
    const font = getComputedStyle(canvas).getPropertyValue("--font-display").trim() || "monospace";
    for (const pop of gameRef.current.pops) {
      context.globalAlpha = Math.min(1, pop.ticksLeft / 6);
      context.fillStyle = colors.shot;
      context.font = `${pitch * 2}px ${font}`;
      context.fillText(`+${pop.points}`, pop.x * pitch, (pop.y - (12 - pop.ticksLeft) / 3) * pitch);
    }
    context.globalAlpha = 1;
  }, []);

  // The loop runs only while focused; everything else is a paused frame.
  useEffect(() => {
    draw();
    if (!focused) return;

    const timer = window.setInterval(() => {
      const keys = keysRef.current;
      const next = step(gameRef.current, { ...NO_INPUT, left: keys.left, right: keys.right, fire: keys.fire });
      keys.fire = false;
      gameRef.current = next;
      saveGame(next);
      draw();
      setHud((current) =>
        current.score === next.score && current.lives === next.lives && current.status === next.status
          ? current
          : { score: next.score, lives: next.lives, status: next.status },
      );
      if (next.status === "over" && !reportedRef.current) {
        reportedRef.current = true;
        onGameOver(next.score);
      }
    }, TICK_MS);

    return () => window.clearInterval(timer);
  }, [draw, focused, onGameOver]);

  // A hidden window or a lost focus pauses: the user is not looking.
  useEffect(() => {
    const pause = () => frameRef.current?.blur();
    const onVisibility = () => {
      if (document.hidden) pause();
    };
    window.addEventListener("blur", pause);
    document.addEventListener("visibilitychange", onVisibility);
    window.addEventListener("resize", draw);
    return () => {
      window.removeEventListener("blur", pause);
      document.removeEventListener("visibilitychange", onVisibility);
      window.removeEventListener("resize", draw);
    };
  }, [draw]);

  function onKey(event: KeyboardEvent<HTMLDivElement>, down: boolean) {
    const keys = keysRef.current;
    switch (event.key) {
      case "ArrowLeft":
        keys.left = down;
        break;
      case "ArrowRight":
        keys.right = down;
        break;
      case " ":
        if (down && !event.repeat) {
          if (gameRef.current.status === "over") {
            gameRef.current = newGame(Date.now() | 0);
            saveGame(gameRef.current);
            reportedRef.current = false;
            setHud({ score: 0, lives: START_LIVES, status: "playing" });
          } else {
            keys.fire = true;
          }
        }
        break;
      case "Escape":
        if (down) onHide();
        break;
      default:
        return;
    }
    // Arrow keys and space would otherwise scroll the page underneath.
    event.preventDefault();
  }

  const best = Math.max(highScore, hud.score);

  return (
    <div className="flex flex-col">
      <div className="flex flex-wrap items-center gap-x-10 gap-y-2 border-b-2 border-line px-7 py-4">
        <span className="font-display text-[18px] uppercase text-lime">Goat invaders</span>
        <span className="flex-1" />
        <span className="font-mono text-[14px] text-stone">SCORE <span className="text-bone">{String(hud.score).padStart(5, "0")}</span></span>
        <span className="font-mono text-[14px] text-stone">HI <span className="text-bone">{String(best).padStart(5, "0")}</span></span>
        <span className="flex items-center gap-2 font-mono text-[14px] text-stone">
          LIVES
          <span aria-label={`${hud.lives} lives`} className="flex gap-1">
            {Array.from({ length: START_LIVES }, (_, index) => (
              <span key={index} className={`h-2.5 w-2.5 ${index < hud.lives ? "bg-lime" : "bg-line-3"}`} />
            ))}
          </span>
        </span>
        <span className="flex gap-2 font-mono text-[12px] text-stone">
          <kbd className="border-2 border-line-3 px-2 py-0.5">← →</kbd>
          <kbd className="border-2 border-line-3 px-2 py-0.5">SPACE</kbd>
          <kbd className="border-2 border-line-3 px-2 py-0.5">ESC hides</kbd>
        </span>
      </div>

      <div
        ref={frameRef}
        tabIndex={0}
        role="application"
        aria-label="Goat Invaders. Arrow keys move, space fires, Escape hides the game."
        onFocus={() => setFocused(true)}
        onBlur={() => {
          setFocused(false);
          keysRef.current = { left: false, right: false, fire: false };
        }}
        onKeyDown={(event) => onKey(event, true)}
        onKeyUp={(event) => onKey(event, false)}
        className="relative cursor-pointer px-7 py-5 outline-none focus-visible:outline-none"
      >
        <canvas ref={canvasRef} className="pixelated mx-auto block w-full max-w-[720px]" />
        {!focused || hud.status === "over" ? (
          <div className="absolute inset-0 flex items-center justify-center">
            <span className="border-2 border-bone bg-ink px-5 py-3 font-display text-[14px] uppercase text-bone">
              {hud.status === "over" ? "Game over · space to play again" : "Paused · click to play"}
            </span>
          </div>
        ) : null}
      </div>
    </div>
  );
}
