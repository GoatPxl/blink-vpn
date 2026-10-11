import assert from "node:assert/strict";
import { describe, it } from "node:test";

import {
  FIELD_WIDTH,
  NO_INPUT,
  PLAYER_Y,
  marchInterval,
  newGame,
  renderCells,
  step,
  type GameState,
} from "../../src/lib/game/invaders.ts";

function run(state: GameState, ticks: number, input = NO_INPUT): GameState {
  let current = state;
  for (let index = 0; index < ticks; index += 1) current = step(current, input);
  return current;
}

describe("goat invaders", () => {
  it("starts a full wave with three lives", () => {
    const game = newGame(7);
    assert.equal(game.invaders.length, 24);
    assert.equal(game.lives, 3);
    assert.equal(game.status, "playing");
  });

  it("is deterministic for a seed and an input sequence", () => {
    const a = run(newGame(42), 300, { left: false, right: true, fire: true });
    const b = run(newGame(42), 300, { left: false, right: true, fire: true });
    assert.deepEqual(a, b);
  });

  it("keeps the player inside the field", () => {
    const left = run(newGame(), 200, { left: true, right: false, fire: false });
    const right = run(newGame(), 200, { left: false, right: true, fire: false });
    assert.equal(left.playerX, 0);
    assert.equal(right.playerX, FIELD_WIDTH - 3);
  });

  it("scores when a shot reaches an invader", () => {
    // Park under the leftmost column and keep firing; a hit must land within the field's height.
    let game = newGame(3);
    const target = game.invaders.find((invader) => invader.row === 2) ?? game.invaders[0];
    game = { ...game, playerX: target.x, bombs: [] };
    let scored = false;
    for (let tick = 0; tick < 60 && !scored; tick += 1) {
      game = step({ ...game, bombs: [] }, { left: false, right: false, fire: tick % 3 === 0 });
      scored = game.score > 0;
    }
    assert.ok(scored, "a shot fired straight up under an invader must hit something");
  });

  it("loses a life to a bomb, once, then blinks invulnerable", () => {
    const game = newGame(1);
    const hit = step({ ...game, bombs: [{ x: game.playerX + 1, y: PLAYER_Y }] }, NO_INPUT);
    assert.equal(hit.lives, 2);
    assert.ok(hit.invulnerableTicks > 0);
    const again = step({ ...hit, bombs: [{ x: hit.playerX + 1, y: PLAYER_Y }] }, NO_INPUT);
    assert.equal(again.lives, 2, "invulnerable ticks absorb an immediate second hit");
  });

  it("ends when the last life goes, and then stops changing", () => {
    const game = { ...newGame(1), lives: 1 };
    const over = step({ ...game, bombs: [{ x: game.playerX + 1, y: PLAYER_Y }] }, NO_INPUT);
    assert.equal(over.status, "over");
    assert.equal(step(over, { left: true, right: false, fire: true }), over);
  });

  it("brings a faster wave when one is cleared", () => {
    const game = newGame(1);
    const cleared = { ...game, invaders: game.invaders.map((invader, index) => ({ ...invader, alive: index === 0 })) };
    const lastTarget = cleared.invaders[0];
    const next = step({ ...cleared, shots: [{ x: lastTarget.x + 1, y: lastTarget.y + 2 }] }, NO_INPUT);
    assert.equal(next.wave, 2);
    assert.equal(next.invaders.filter((invader) => invader.alive).length, 24);
    assert.ok(marchInterval(24, 2) < marchInterval(24, 1));
  });

  it("marches faster as the wave thins", () => {
    assert.ok(marchInterval(1, 1) < marchInterval(24, 1));
  });

  it("draws the player on the bottom rows", () => {
    const cells = renderCells(newGame(1));
    const game = newGame(1);
    assert.equal(cells.get((PLAYER_Y + 1) * FIELD_WIDTH + game.playerX), "player");
  });
});
