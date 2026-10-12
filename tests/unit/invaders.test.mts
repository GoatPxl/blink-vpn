import assert from "node:assert/strict";
import { describe, it } from "node:test";

import {
  CHARGE_BONUS,
  CHARGE_EVERY_TICKS,
  FIELD_HEIGHT,
  FIELD_WIDTH,
  NO_INPUT,
  PAW_TICKS,
  PLAYER_Y,
  REGROUP_TICKS,
  SPIT_CHANCE_PER_MILLE,
  START_LIVES,
  STEER_EVERY_TICKS,
  goatPosition,
  marchInterval,
  maxChargers,
  newGame,
  renderCells,
  step,
  type GameInput,
  type GameState,
  type Goat,
} from "../../src/lib/game/invaders.ts";
import { keepWaitScreen } from "../../src/lib/game/session.ts";

function run(state: GameState, ticks: number, input: GameInput = NO_INPUT): GameState {
  let current = state;
  for (let index = 0; index < ticks; index += 1) current = step(current, input);
  return current;
}

/** A game where goat 0 is set up in a given state, everything else in the herd. */
function withGoat(game: GameState, patch: Partial<Goat>): GameState {
  return { ...game, goats: game.goats.map((goat, index) => (index === 0 ? { ...goat, ...patch } : goat)) };
}

describe("goat invaders", () => {
  it("starts a full herd with a generous set of lives", () => {
    const game = newGame(7);
    assert.equal(game.goats.length, 21);
    assert.equal(game.lives, START_LIVES);
    assert.ok(START_LIVES >= 5);
    assert.equal(game.status, "playing");
  });

  it("is deterministic for a seed and an input sequence", () => {
    const a = run(newGame(42), 400, { left: false, right: true, fire: true });
    const b = run(newGame(42), 400, { left: false, right: true, fire: true });
    assert.deepEqual(a, b);
  });

  it("keeps the player inside the field", () => {
    assert.equal(run(newGame(), 200, { left: true, right: false, fire: false }).playerX, 0);
    assert.equal(run(newGame(), 200, { left: false, right: true, fire: false }).playerX, FIELD_WIDTH - 3);
  });

  it("fits the herd inside the field", () => {
    const game = newGame();
    for (const goat of game.goats) {
      const { x } = goatPosition(game, goat);
      assert.ok(x >= 0 && x + 5 <= FIELD_WIDTH);
    }
  });

  it("never charges without pawing first, for the full warning", () => {
    // Watch every goat's state across a long seeded game; a charge must follow PAW_TICKS of pawing.
    let game = newGame(9);
    const pawedFor = new Map<number, number>();
    for (let tick = 0; tick < 3000 && game.status === "playing"; tick += 1) {
      const before = game.goats.map((goat) => goat.state);
      game = step({ ...game, lives: 3 }, { left: tick % 40 < 20, right: tick % 40 >= 20, fire: tick % 7 === 0 });
      game.goats.forEach((goat, index) => {
        if (goat.state === "pawing") pawedFor.set(index, (pawedFor.get(index) ?? 0) + 1);
        if (goat.state === "charging" && before[index] === "pawing") {
          assert.equal(pawedFor.get(index), PAW_TICKS, `goat ${index} charged after ${pawedFor.get(index)} ticks of warning`);
          pawedFor.delete(index);
        }
        if (goat.state === "charging") assert.notEqual(before[index], "herd", "a goat went straight from the herd to a charge");
        // Shot mid-paw, or a new wave reusing the slot: that paw never became a charge.
        if (goat.state !== "pawing" && !(goat.state === "charging" && before[index] === "pawing")) pawedFor.delete(index);
      });
    }
  });

  it("charges down the field at a dodgeable pace, steering at most one column per three ticks", () => {
    let game = withGoat(newGame(3), { state: "charging", x: 5, y: 2, targetX: 40, timer: 0 });
    game = { ...game, playerX: 42, spit: [] };
    for (let tick = 0; tick < 12; tick += 1) {
      const previous = game.goats[0];
      game = step({ ...game, spit: [] }, NO_INPUT);
      const goat = game.goats[0];
      assert.ok(goat.y >= previous.y && goat.y - previous.y <= 1);
      assert.ok(Math.abs(goat.x - previous.x) <= 1);
    }
    assert.equal(game.goats[0].y, 2 + 12 / CHARGE_EVERY_TICKS, "one row every two ticks");
    assert.ok(game.goats[0].x - 5 <= 12 / STEER_EVERY_TICKS, "twelve ticks steer at most four columns");
  });

  it("scores a charge bonus for a goat shot mid-charge", () => {
    const game = withGoat(newGame(3), { state: "charging", x: 20, y: 10, targetX: 20, timer: 1 });
    const hit = step({ ...game, shots: [{ x: 22, y: 12 }] }, NO_INPUT);
    assert.equal(hit.goats[0].alive, false);
    assert.equal(hit.score, CHARGE_BONUS);
    assert.equal(hit.pops.length, 1);
  });

  it("sends a goat that misses off the field, then back to its slot, still part of the wave", () => {
    // Timer 1 so the next tick moves it a row, off the bottom.
    let game = withGoat(newGame(3), { state: "charging", x: 0, y: FIELD_HEIGHT - 1, targetX: 0, timer: 1 });
    game = { ...game, playerX: 40 };
    game = step(game, NO_INPUT);
    assert.equal(game.goats[0].state, "returning");
    assert.equal(game.wave, 1);
    for (let tick = 0; tick < REGROUP_TICKS && game.goats[0].state === "returning"; tick += 1) game = step({ ...game, spit: [] }, NO_INPUT);
    assert.equal(game.goats[0].state, "herd");
    assert.equal(game.goats[0].alive, true);
  });

  it("loses a life when a charging goat rams the player, once", () => {
    const game = newGame(1);
    const rammed = step(withGoat(game, { state: "charging", x: game.playerX - 1, y: PLAYER_Y - 3, targetX: game.playerX - 1, timer: 1 }), NO_INPUT);
    assert.equal(rammed.lives, START_LIVES - 1);
    assert.equal(rammed.goats[0].state, "returning");
    assert.ok(rammed.invulnerableTicks > 0);
  });

  it("caps how many goats are restless at once", () => {
    let game = newGame(11);
    for (let tick = 0; tick < 4000 && game.status === "playing"; tick += 1) {
      game = step({ ...game, lives: 3 }, NO_INPUT);
      const restless = game.goats.filter((goat) => goat.alive && (goat.state === "pawing" || goat.state === "charging")).length;
      assert.ok(restless <= maxChargers(game.wave), `${restless} restless on wave ${game.wave}`);
    }
  });

  it("spits rarely: charging is the main threat", () => {
    assert.ok(SPIT_CHANCE_PER_MILLE < 45 / 3, "a quarter of the old bomb rate, roughly");
  });

  it("ends when the last life goes, and then stops changing", () => {
    const game = { ...newGame(1), lives: 1 };
    const over = step({ ...game, spit: [{ x: game.playerX + 1, y: PLAYER_Y }] }, NO_INPUT);
    assert.equal(over.status, "over");
    assert.equal(step(over, { left: true, right: false, fire: true }), over);
  });

  it("brings a faster herd when one is cleared", () => {
    const game = newGame(1);
    const lastOne = { ...game, goats: game.goats.map((goat, index) => ({ ...goat, alive: index === 0 })) };
    const position = goatPosition(lastOne, lastOne.goats[0]);
    const next = step({ ...lastOne, shots: [{ x: position.x + 1, y: position.y + 3 }] }, NO_INPUT);
    assert.equal(next.wave, 2);
    assert.equal(next.goats.filter((goat) => goat.alive).length, 21);
    assert.ok(marchInterval(21, 2) < marchInterval(21, 1));
  });

  it("draws pawing and charging goats as their own kinds", () => {
    const game = withGoat(newGame(1), { state: "charging", x: 10, y: 12, targetX: 10, timer: 0 });
    const kinds = new Set(renderCells(game).values());
    assert.ok(kinds.has("goat-charging"));
    assert.ok(kinds.has("player"));
  });
});

describe("keepWaitScreen", () => {
  it("stays while anything is in flight", () => {
    assert.equal(keepWaitScreen({ waiting: true, sessionOpen: false, gameOver: false }), true);
  });

  it("stays after the wait only while a game is being played", () => {
    assert.equal(keepWaitScreen({ waiting: false, sessionOpen: true, gameOver: false }), true);
    assert.equal(keepWaitScreen({ waiting: false, sessionOpen: true, gameOver: true }), false);
    assert.equal(keepWaitScreen({ waiting: false, sessionOpen: false, gameOver: false }), false);
  });
});
