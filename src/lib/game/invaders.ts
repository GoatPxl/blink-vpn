/**
 * Goat Invaders: the game that fills the ~2 minute build.
 *
 * Pure and deterministic — a state, an input, a new state — so it can be tested without a DOM and
 * so the component that draws it holds no rules of its own. Randomness comes from a seed carried in
 * the state, never from `Math.random`, which keeps a replay of the same inputs identical.
 *
 * Runs locally and touches nothing: no network, no backend, no storage. The component saves the
 * high score; this module never does.
 */

export const FIELD_WIDTH = 48;
export const FIELD_HEIGHT = 24;

/** Invader sprite, 3×2. */
export const INVADER_SPRITE = ["#.#", "###"] as const;
const INVADER_WIDTH = 3;
const INVADER_HEIGHT = 2;
const INVADER_COLUMNS = 8;
const INVADER_ROWS = 3;
const INVADER_SPACING_X = 5;
const INVADER_SPACING_Y = 3;

/** Player sprite, 3×2, on the bottom two rows. */
export const PLAYER_SPRITE = [".#.", "###"] as const;
const PLAYER_WIDTH = 3;
export const PLAYER_Y = FIELD_HEIGHT - 2;

const START_LIVES = 3;
const MAX_PLAYER_SHOTS = 2;
const MAX_BOMBS = 3;
/** Ticks between a full invader step, at the start of a wave. Speeds up as the wave thins. */
const SLOWEST_MARCH_TICKS = 10;
const FASTEST_MARCH_TICKS = 2;
/** Chance per tick, in thousandths, that some invader drops a bomb. */
const BOMB_CHANCE_PER_MILLE = 45;
const POINTS_PER_ROW = [30, 20, 10] as const;

export type Point = { x: number; y: number };
export type Invader = Point & { row: number; alive: boolean };
export type GameStatus = "playing" | "over";

export type GameState = {
  tick: number;
  playerX: number;
  invaders: Invader[];
  direction: 1 | -1;
  shots: Point[];
  bombs: Point[];
  score: number;
  lives: number;
  wave: number;
  status: GameStatus;
  /** Ticks the player is invulnerable for after being hit, so one bomb cannot cost two lives. */
  invulnerableTicks: number;
  seed: number;
};

export type GameInput = {
  left: boolean;
  right: boolean;
  /** True on the tick the fire key went down. Held fire does not auto-repeat. */
  fire: boolean;
};

export const NO_INPUT: GameInput = { left: false, right: false, fire: false };

/** mulberry32: small, fast, and good enough for where a goat drops a bomb. */
function nextRandom(seed: number): { value: number; seed: number } {
  const next = (seed + 0x6d2b79f5) | 0;
  let t = next;
  t = Math.imul(t ^ (t >>> 15), t | 1);
  t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
  return { value: ((t ^ (t >>> 14)) >>> 0) / 4294967296, seed: next };
}

function spawnWave(): Invader[] {
  const invaders: Invader[] = [];
  const blockWidth = (INVADER_COLUMNS - 1) * INVADER_SPACING_X + INVADER_WIDTH;
  const left = Math.floor((FIELD_WIDTH - blockWidth) / 2);
  for (let row = 0; row < INVADER_ROWS; row += 1) {
    for (let column = 0; column < INVADER_COLUMNS; column += 1) {
      invaders.push({ x: left + column * INVADER_SPACING_X, y: 2 + row * INVADER_SPACING_Y, row, alive: true });
    }
  }
  return invaders;
}

export function newGame(seed = 1): GameState {
  return {
    tick: 0,
    playerX: Math.floor((FIELD_WIDTH - PLAYER_WIDTH) / 2),
    invaders: spawnWave(),
    direction: 1,
    shots: [],
    bombs: [],
    score: 0,
    lives: START_LIVES,
    wave: 1,
    status: "playing",
    invulnerableTicks: 0,
    seed,
  };
}

/** Ticks between invader steps: slower with a full wave, faster as it thins and in later waves. */
export function marchInterval(alive: number, wave: number): number {
  const total = INVADER_COLUMNS * INVADER_ROWS;
  const span = SLOWEST_MARCH_TICKS - FASTEST_MARCH_TICKS;
  const byCount = FASTEST_MARCH_TICKS + Math.round((span * alive) / total);
  return Math.max(FASTEST_MARCH_TICKS, byCount - (wave - 1));
}

function hitsInvader(point: Point, invader: Invader): boolean {
  return invader.alive
    && point.x >= invader.x && point.x < invader.x + INVADER_WIDTH
    && point.y >= invader.y && point.y < invader.y + INVADER_HEIGHT;
}

function hitsPlayer(point: Point, playerX: number): boolean {
  return point.x >= playerX && point.x < playerX + PLAYER_WIDTH && point.y >= PLAYER_Y && point.y < FIELD_HEIGHT;
}

function marchInvaders(state: GameState): Pick<GameState, "invaders" | "direction"> {
  const alive = state.invaders.filter((invader) => invader.alive);
  const minX = Math.min(...alive.map((invader) => invader.x));
  const maxX = Math.max(...alive.map((invader) => invader.x + INVADER_WIDTH - 1));
  const blocked = state.direction === 1 ? maxX >= FIELD_WIDTH - 1 : minX <= 0;

  if (blocked) {
    return {
      direction: state.direction === 1 ? -1 : 1,
      invaders: state.invaders.map((invader) => ({ ...invader, y: invader.y + 1 })),
    };
  }
  return {
    direction: state.direction,
    invaders: state.invaders.map((invader) => ({ ...invader, x: invader.x + state.direction })),
  };
}

/** One tick of the game. Returns the state unchanged once the game is over. */
export function step(state: GameState, input: GameInput): GameState {
  if (state.status === "over") return state;

  const tick = state.tick + 1;
  let seed = state.seed;

  // Player.
  let playerX = state.playerX;
  if (input.left && !input.right) playerX = Math.max(0, playerX - 1);
  if (input.right && !input.left) playerX = Math.min(FIELD_WIDTH - PLAYER_WIDTH, playerX + 1);

  // Shots move every tick; a new one leaves the nose of the ship.
  let shots = state.shots.map((shot) => ({ x: shot.x, y: shot.y - 1 })).filter((shot) => shot.y >= 0);
  if (input.fire && shots.length < MAX_PLAYER_SHOTS) {
    shots = [...shots, { x: playerX + 1, y: PLAYER_Y - 1 }];
  }

  // Invaders march on their own clock.
  let { invaders, direction } = state;
  const aliveBefore = invaders.filter((invader) => invader.alive).length;
  if (tick % marchInterval(aliveBefore, state.wave) === 0) {
    ({ invaders, direction } = marchInvaders({ ...state, invaders, direction }));
  }

  // Shots against invaders.
  let score = state.score;
  const survivingShots: Point[] = [];
  invaders = invaders.map((invader) => ({ ...invader }));
  for (const shot of shots) {
    const target = invaders.find((invader) => hitsInvader(shot, invader));
    if (target) {
      target.alive = false;
      score += POINTS_PER_ROW[target.row] ?? 10;
    } else {
      survivingShots.push(shot);
    }
  }

  // Bombs fall every other tick; one may drop from a random front-line invader.
  let bombs = tick % 2 === 0
    ? state.bombs.map((bomb) => ({ x: bomb.x, y: bomb.y + 1 })).filter((bomb) => bomb.y < FIELD_HEIGHT)
    : state.bombs;
  const roll = nextRandom(seed);
  seed = roll.seed;
  const living = invaders.filter((invader) => invader.alive);
  if (living.length > 0 && bombs.length < MAX_BOMBS && roll.value * 1000 < BOMB_CHANCE_PER_MILLE) {
    const pick = nextRandom(seed);
    seed = pick.seed;
    const shooter = living[Math.floor(pick.value * living.length)];
    bombs = [...bombs, { x: shooter.x + 1, y: shooter.y + INVADER_HEIGHT }];
  }

  // Bombs against the player.
  let lives = state.lives;
  let invulnerableTicks = Math.max(0, state.invulnerableTicks - 1);
  if (invulnerableTicks === 0 && bombs.some((bomb) => hitsPlayer(bomb, playerX))) {
    lives -= 1;
    invulnerableTicks = 20;
    bombs = bombs.filter((bomb) => !hitsPlayer(bomb, playerX));
  }

  // A goat that reaches the player's rows ends the game outright.
  const landed = invaders.some((invader) => invader.alive && invader.y + INVADER_HEIGHT > PLAYER_Y);
  if (lives <= 0 || landed) {
    return { ...state, tick, playerX, invaders, direction, shots: survivingShots, bombs, score, lives: Math.max(0, lives), invulnerableTicks, seed, status: "over" };
  }

  // A cleared wave brings the next, faster one.
  if (invaders.every((invader) => !invader.alive)) {
    return { ...state, tick, playerX, invaders: spawnWave(), direction: 1, shots: [], bombs: [], score, lives, invulnerableTicks, seed, wave: state.wave + 1 };
  }

  return { ...state, tick, playerX, invaders, direction, shots: survivingShots, bombs, score, lives, invulnerableTicks, seed };
}

/** Every lit cell, for drawing. Later layers win: invaders, then shots, bombs, then the player. */
export type CellKind = "invader" | "invader-front" | "shot" | "bomb" | "player";

export function renderCells(state: GameState): Map<number, CellKind> {
  const cells = new Map<number, CellKind>();
  const put = (x: number, y: number, kind: CellKind) => {
    if (x >= 0 && x < FIELD_WIDTH && y >= 0 && y < FIELD_HEIGHT) cells.set(y * FIELD_WIDTH + x, kind);
  };
  const frontRow = Math.max(-1, ...state.invaders.filter((invader) => invader.alive).map((invader) => invader.row));

  for (const invader of state.invaders) {
    if (!invader.alive) continue;
    INVADER_SPRITE.forEach((line, dy) => {
      [...line].forEach((pixel, dx) => {
        if (pixel === "#") put(invader.x + dx, invader.y + dy, invader.row === frontRow ? "invader-front" : "invader");
      });
    });
  }
  for (const shot of state.shots) put(shot.x, shot.y, "shot");
  for (const bomb of state.bombs) put(bomb.x, bomb.y, "bomb");
  // Blinks while invulnerable so a hit is visible.
  if (state.invulnerableTicks % 4 < 2) {
    PLAYER_SPRITE.forEach((line, dy) => {
      [...line].forEach((pixel, dx) => {
        if (pixel === "#") put(state.playerX + dx, PLAYER_Y + dy, "player");
      });
    });
  }
  return cells;
}
