import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { buildQrPath } from "../../src/lib/qr-display.ts";

/** `.` light, `#` dark — readable matrices for the cases below. */
function matrix(rows: string[]): { modules: boolean[]; size: number } {
  const size = rows.length;
  const modules: boolean[] = [];

  for (const row of rows) {
    assert.equal(row.length, size, "the fixture must be square");
    for (const cell of row) {
      modules.push(cell === "#");
    }
  }

  return { modules, size };
}

describe("buildQrPath", () => {
  it("draws one subpath per dark module when none are adjacent", () => {
    const { modules, size } = matrix([
      "#.",
      ".#",
    ]);

    assert.equal(buildQrPath(modules, size), "M0 0h1v1h-1zM1 1h1v1h-1z");
  });

  it("merges horizontally adjacent modules into a single run", () => {
    const { modules, size } = matrix([
      "###",
      "...",
      "...",
    ]);

    // One run of three, not three runs of one.
    assert.equal(buildQrPath(modules, size), "M0 0h3v1h-3z");
  });

  it("breaks a run where a light module interrupts it", () => {
    const { modules, size } = matrix([
      "##.##",
      ".....",
      ".....",
      ".....",
      ".....",
    ]);

    assert.equal(buildQrPath(modules, size), "M0 0h2v1h-2zM3 0h2v1h-2z");
  });

  it("does not merge vertically, so each row stands alone", () => {
    const { modules, size } = matrix([
      "#.",
      "#.",
    ]);

    assert.equal(buildQrPath(modules, size), "M0 0h1v1h-1zM0 1h1v1h-1z");
  });

  it("returns an empty path for an all-light matrix", () => {
    const { modules, size } = matrix([
      "..",
      "..",
    ]);

    assert.equal(buildQrPath(modules, size), "");
  });

  // A mismatched matrix is a programming error. Returning "" lets the component show an explicit
  // fallback rather than a blank square that reads as a rendering glitch.
  it("returns an empty path when the matrix is not a square of the stated size", () => {
    assert.equal(buildQrPath([true, false, true], 2), "");
    assert.equal(buildQrPath([], 0), "");
    assert.equal(buildQrPath([true], -1), "");
    assert.equal(buildQrPath([true], 1.5), "");
  });
});
