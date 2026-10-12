/**
 * Goat Invaders: the game that fills the wait for a server and its first handshake.
 *
 * Pure and deterministic — a state, an input, a new state — so it can be tested without a DOM and
 * so the component that draws it holds no rules of its own. Randomness comes from a seed carried in
 * the state, never from `Math.random`, which keeps a replay of the same inputs identical.
 *
 * The goats are goats: they march as a herd, and every so often one paws the ground — a visible
 * warning — then charges down the field horns first. Charging is the main threat; spit is rare.
 *
 * Runs locally and touches nothing: no network, no backend, no storage. The component saves the
 * high score; this module never does.
 */

export const FIELD_WIDTH = 48;
export const FIELD_HEIGHT = 24;

/** Goat sprite, 5×3: horns up in the herd. */
export const GOAT_SPRITE = ["#...#", ".###.", ".#.#."] as const;
/** Horns down, for a goat on the charge. */
export const CHARGING_GOAT_SPRITE = [".#.#.", ".###.", "#...#"] as const;
const GOAT_WIDTH = 5;
const GOAT_HEIGHT = 3;
const HERD_COLUMNS = 7;
const HERD_ROWS = 3;
const SPACING_X = 6;
const SPACING_Y = 4;
const HERD_TOP = 2;

/** Player sprite, 3×2, on the bottom two rows. */
export const PLAYER_SPRITE = [".#.", "###"] as const;
const PLAYER_WIDTH = 3;
export const PLAYER_Y = FIELD_HEIGHT - 2;

/** Generous on purpose: this fills a wait, it is not meant to end quickly. */
export const START_LIVES = 5;
const MAX_PLAYER_SHOTS = 2;
const MAX_SPIT = 2;
/** Ticks between herd steps at the start of a wave. Speeds up as the herd thins. */
const SLOWEST_MARCH_TICKS = 10;
const FASTEST_MARCH_TICKS = 2;
/** Chance per tick, in thousandths, that some goat spits. A quarter of the old bomb rate. */
export const SPIT_CHANCE_PER_MILLE = 11;
/** Base chance per tick, in thousandths, that a goat starts pawing. Rises with wave and thinning. */
const PAW_CHANCE_PER_MILLE = 8;
/** How long a goat paws before it charges: the player's warning. */
export const PAW_TICKS = 12;
/** A charging goat moves down one row per this many ticks: about a second and a half to cross. */
export const CHARGE_EVERY_TICKS = 2;
/** It steers one column per this many ticks. Dodgeable, not a homing missile. */
export const STEER_EVERY_TICKS = 3;
/** A goat that missed, or rammed the player, is off the field this long before it rejoins. */
export const REGROUP_TICKS = 30;
/** It re-aims at the player this often, so it is not trivially dodged by standing still aside. */
const REAIM_EVERY_TICKS = 8;
export const CHARGE_BONUS = 100;
const POINTS_PER_ROW = [30, 20, 10] as const;
const POP_TICKS = 12;
const INVULNERABLE_TICKS = 40;

export type Point = { x: number; y: number };

/** `returning` is off the field, regrouping, before it rejoins the herd in its slot. */
export type GoatState = "herd" | "pawing" | "charging" | "returning";

export type Goat = {
  /** Slot in the herd. */
  column: number;
  row: number;
  alive: boolean;
  state: GoatState;
  /** Absolute position while charging; ignored in the herd. */
  x: number;
  y: number;
  /** Ticks left to paw, or where a charge is headed. */
  timer: number;
  targetX: number;
};

/** A "+100" over a goat shot mid-charge. */
export type Pop = Point & { points: number; ticksLeft: number };

export type GameStatus = "playing" | "over";

export type GameState = {
  tick: number;
  playerX: number;
  goats: Goat[];
  /** Where the herd's top-left slot is now. */
  herdX: number;
  herdY: number;
  direction: 1 | -1;
  shots: Point[];
  spit: Point[];
  pops: Pop[];
  score: number;
  lives: number;
  wave: number;
  status: GameStatus;
  /** Ticks the player is invulnerable for after being hit, so one hit cannot cost two lives. */
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

/** mulberry32: small, fast, and good enough for which goat loses its temper. */
function nextRandom(seed: number): { value: number; seed: number } {
  const next = (seed + 0x6d2b79f5) | 0;
  let t = next;
  t = Math.imul(t ^ (t >>> 15), t | 1);
  t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
  return { value: ((t ^ (t >>> 14)) >>> 0) / 4294967296, seed: next };
}

const HERD_WIDTH = (HERD_COLUMNS - 1) * SPACING_X + GOAT_WIDTH;

function spawnHerd(): Goat[] {
  const goats: Goat[] = [];
  for (let row = 0; row < HERD_ROWS; row += 1) {
    for (let column = 0; column < HERD_COLUMNS; column += 1) {
      goats.push({ column, row, alive: true, state: "herd", x: 0, y: 0, timer: 0, targetX: 0 });
    }
  }
  return goats;
}

export function newGame(seed = 1): GameState {
  return {
    tick: 0,
    playerX: Math.floor((FIELD_WIDTH - PLAYER_WIDTH) / 2),
    goats: spawnHerd(),
    herdX: Math.floor((FIELD_WIDTH - HERD_WIDTH) / 2),
    herdY: HERD_TOP,
    direction: 1,
    shots: [],
    spit: [],
    pops: [],
    score: 0,
    lives: START_LIVES,
    wave: 1,
    status: "playing",
    invulnerableTicks: 0,
    seed,
  };
}

/** Where a goat is drawn: its herd slot, or its own position once it has left the herd. */
export function goatPosition(state: Pick<GameState, "herdX" | "herdY">, goat: Goat): Point {
  if (goat.state === "charging") return { x: goat.x, y: goat.y };
  return { x: state.herdX + goat.column * SPACING_X, y: state.herdY + goat.row * SPACING_Y };
}

/** Ticks between herd steps: slower with a full herd, faster as it thins and in later waves. */
export function marchInterval(alive: number, wave: number): number {
  const total = HERD_COLUMNS * HERD_ROWS;
  const span = SLOWEST_MARCH_TICKS - FASTEST_MARCH_TICKS;
  const byCount = FASTEST_MARCH_TICKS + Math.round((span * alive) / total);
  return Math.max(FASTEST_MARCH_TICKS, byCount - (wave - 1));
}

/** How many goats may be pawing or charging at once. */
export function maxChargers(wave: number): number {
  return wave >= 3 ? 3 : 2;
}

function overlaps(point: Point, topLeft: Point, width: number, height: number): boolean {
  return point.x >= topLeft.x && point.x < topLeft.x + width && point.y >= topLeft.y && point.y < topLeft.y + height;
}

function boxesOverlap(a: Point, aw: number, ah: number, b: Point, bw: number, bh: number): boolean {
  return a.x < b.x + bw && a.x + aw > b.x && a.y < b.y + bh && a.y + ah > b.y;
}

/** The x a charging goat aims for, so its middle meets the player's middle. */
function aimAt(playerX: number): number {
  const target = playerX + Math.floor(PLAYER_WIDTH / 2) - Math.floor(GOAT_WIDTH / 2);
  return Math.min(FIELD_WIDTH - GOAT_WIDTH, Math.max(0, target));
}

/** A goat at the front of its column: nothing alive in the herd beneath it. */
function isFrontLine(goats: readonly Goat[], goat: Goat): boolean {
  return !goats.some((other) => other.alive && other.state === "herd" && other.column === goat.column && other.row > goat.row);
}

function marchHerd(state: GameState, goats: Goat[]): Pick<GameState, "herdX" | "herdY" | "direction"> {
  const inHerd = goats.filter((goat) => goat.alive && (goat.state === "herd" || goat.state === "pawing"));
  if (inHerd.length === 0) return { herdX: state.herdX, herdY: state.herdY, direction: state.direction };
  const minX = Math.min(...inHerd.map((goat) => state.herdX + goat.column * SPACING_X));
  const maxX = Math.max(...inHerd.map((goat) => state.herdX + goat.column * SPACING_X + GOAT_WIDTH - 1));
  const blocked = state.direction === 1 ? maxX >= FIELD_WIDTH - 1 : minX <= 0;
  if (blocked) return { herdX: state.herdX, herdY: state.herdY + 1, direction: state.direction === 1 ? -1 : 1 };
  return { herdX: state.herdX + state.direction, herdY: state.herdY, direction: state.direction };
}

/** One tick of the game. Returns the state unchanged once the game is over. */
export function step(state: GameState, input: GameInput): GameState {
  if (state.status === "over") return state;

  const tick = state.tick + 1;
  let seed = state.seed;
  const roll = () => {
    const result = nextRandom(seed);
    seed = result.seed;
    return result.value;
  };

  // Player.
  let playerX = state.playerX;
  if (input.left && !input.right) playerX = Math.max(0, playerX - 1);
  if (input.right && !input.left) playerX = Math.min(FIELD_WIDTH - PLAYER_WIDTH, playerX + 1);

  // Shots move every tick; a new one leaves the nose of the ship.
  let shots = state.shots.map((shot) => ({ x: shot.x, y: shot.y - 1 })).filter((shot) => shot.y >= 0);
  if (input.fire && shots.length < MAX_PLAYER_SHOTS) shots = [...shots, { x: playerX + 1, y: PLAYER_Y - 1 }];

  let goats = state.goats.map((goat) => ({ ...goat }));

  // The herd marches on its own clock.
  let { herdX, herdY, direction } = state;
  const aliveBefore = goats.filter((goat) => goat.alive).length;
  if (tick % marchInterval(aliveBefore, state.wave) === 0) {
    ({ herdX, herdY, direction } = marchHerd(state, goats));
  }
  const herd = { herdX, herdY };

  // Each goat out of the herd does its own thing.
  for (const goat of goats) {
    if (!goat.alive) continue;
    if (goat.state === "pawing") {
      goat.timer -= 1;
      if (goat.timer <= 0) {
        const from = goatPosition(herd, { ...goat, state: "herd" });
        Object.assign(goat, { state: "charging", x: from.x, y: from.y, targetX: aimAt(playerX), timer: 0 });
      }
    } else if (goat.state === "charging") {
      goat.timer += 1;
      if (goat.timer % CHARGE_EVERY_TICKS === 0) goat.y += 1;
      if (goat.timer % REAIM_EVERY_TICKS === 0) goat.targetX = aimAt(playerX);
      if (goat.timer % STEER_EVERY_TICKS === 0 && goat.x !== goat.targetX) goat.x += goat.x < goat.targetX ? 1 : -1;
      // Off the bottom: out of play for a moment, then back in its slot.
      if (goat.y >= FIELD_HEIGHT) Object.assign(goat, { state: "returning", timer: REGROUP_TICKS });
    } else if (goat.state === "returning") {
      // Off the field, so it cannot be shot or run into, and it never trots through the herd.
      goat.timer -= 1;
      if (goat.timer <= 0) Object.assign(goat, { state: "herd", timer: 0 });
    }
  }

  // Maybe a goat loses its temper. Front line only, and never too many at once.
  const restless = goats.filter((goat) => goat.alive && (goat.state === "pawing" || goat.state === "charging")).length;
  const aliveNow = goats.filter((goat) => goat.alive).length;
  const thinning = 1 + (1 - aliveNow / (HERD_COLUMNS * HERD_ROWS));
  const pawChance = (PAW_CHANCE_PER_MILLE / 1000) * (1 + (state.wave - 1) * 0.5) * thinning;
  if (restless < maxChargers(state.wave) && roll() < pawChance) {
    const candidates = goats.filter((goat) => goat.alive && goat.state === "herd" && isFrontLine(goats, goat));
    if (candidates.length > 0) {
      const chosen = candidates[Math.floor(roll() * candidates.length)];
      Object.assign(chosen, { state: "pawing", timer: PAW_TICKS });
    }
  }

  // Shots against goats. A goat hit mid-charge is worth far more.
  let score = state.score;
  let pops = state.pops.map((pop) => ({ ...pop, ticksLeft: pop.ticksLeft - 1 })).filter((pop) => pop.ticksLeft > 0);
  const survivingShots: Point[] = [];
  for (const shot of shots) {
    const target = goats.find((goat) => goat.alive && goat.state !== "returning" && overlaps(shot, goatPosition(herd, goat), GOAT_WIDTH, GOAT_HEIGHT));
    if (!target) {
      survivingShots.push(shot);
      continue;
    }
    target.alive = false;
    if (target.state === "charging") {
      score += CHARGE_BONUS;
      pops = [...pops, { x: target.x, y: target.y, points: CHARGE_BONUS, ticksLeft: POP_TICKS }];
    } else {
      score += POINTS_PER_ROW[target.row] ?? 10;
    }
  }

  // Spit falls every other tick, and is rare.
  let spit = tick % 2 === 0
    ? state.spit.map((drop) => ({ x: drop.x, y: drop.y + 1 })).filter((drop) => drop.y < FIELD_HEIGHT)
    : state.spit;
  const spitters = goats.filter((goat) => goat.alive && goat.state === "herd" && isFrontLine(goats, goat));
  if (spitters.length > 0 && spit.length < MAX_SPIT && roll() * 1000 < SPIT_CHANCE_PER_MILLE) {
    const spitter = goatPosition(herd, spitters[Math.floor(roll() * spitters.length)]);
    spit = [...spit, { x: spitter.x + 2, y: spitter.y + GOAT_HEIGHT }];
  }

  // What reaches the player.
  let lives = state.lives;
  let invulnerableTicks = Math.max(0, state.invulnerableTicks - 1);
  const player = { x: playerX, y: PLAYER_Y };
  const spitHit = spit.some((drop) => overlaps(drop, player, PLAYER_WIDTH, 2));
  const rammer = goats.find((goat) => goat.alive && goat.state === "charging" && boxesOverlap({ x: goat.x, y: goat.y }, GOAT_WIDTH, GOAT_HEIGHT, player, PLAYER_WIDTH, 2));
  if (invulnerableTicks === 0 && (spitHit || rammer)) {
    lives -= 1;
    invulnerableTicks = INVULNERABLE_TICKS;
    spit = spit.filter((drop) => !overlaps(drop, player, PLAYER_WIDTH, 2));
  }
  // A goat that rams the player bounces off and heads home either way.
  if (rammer) Object.assign(rammer, { state: "returning", timer: REGROUP_TICKS });

  // The herd reaching the player's rows ends the game; a charging goat is exempt — it is meant to.
  const landed = goats.some((goat) => goat.alive && goat.state === "herd" && goatPosition(herd, goat).y + GOAT_HEIGHT > PLAYER_Y);
  const base = { ...state, tick, playerX, goats, herdX, herdY, direction, shots: survivingShots, spit, pops, score, invulnerableTicks, seed };
  if (lives <= 0 || landed) return { ...base, lives: Math.max(0, lives), status: "over" };

  // A cleared herd brings the next, faster one.
  if (goats.every((goat) => !goat.alive)) {
    goats = spawnHerd();
    return { ...base, lives, goats, herdX: Math.floor((FIELD_WIDTH - HERD_WIDTH) / 2), herdY: HERD_TOP, direction: 1, shots: [], spit: [], wave: state.wave + 1 };
  }

  return { ...base, lives };
}

/** Every lit cell, for drawing. Later layers win: goats, then shots, spit, then the player. */
export type CellKind = "goat" | "goat-front" | "goat-pawing" | "goat-charging" | "shot" | "spit" | "player";

export function renderCells(state: GameState): Map<number, CellKind> {
  const cells = new Map<number, CellKind>();
  const put = (x: number, y: number, kind: CellKind) => {
    if (x >= 0 && x < FIELD_WIDTH && y >= 0 && y < FIELD_HEIGHT) cells.set(y * FIELD_WIDTH + x, kind);
  };

  for (const goat of state.goats) {
    if (!goat.alive || goat.state === "returning") continue;
    const position = goatPosition(state, goat);
    const y = position.y;
    let x = position.x;
    const charging = goat.state === "charging";
    const kind: CellKind = charging
      ? "goat-charging"
      : goat.state === "pawing"
        ? "goat-pawing"
        : goat.state === "herd" && isFrontLine(state.goats, goat)
          ? "goat-front"
          : "goat";
    // Pawing goats shuffle on the spot, a column either way: the warning is visible as motion.
    if (goat.state === "pawing") x += goat.timer % 4 < 2 ? -1 : 1;
    const sprite = charging ? CHARGING_GOAT_SPRITE : GOAT_SPRITE;
    sprite.forEach((line, dy) => {
      [...line].forEach((pixel, dx) => {
        if (pixel === "#") put(x + dx, y + dy, kind);
      });
    });
  }
  for (const shot of state.shots) put(shot.x, shot.y, "shot");
  for (const drop of state.spit) put(drop.x, drop.y, "spit");
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
