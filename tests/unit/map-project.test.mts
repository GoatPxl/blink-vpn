import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { LAND_MASK, MASK_COLUMNS, MASK_ROWS } from "../../src/lib/map/land-mask.ts";
import { cellOf, distanceKm, projectToMap } from "../../src/lib/map/project.ts";

function isLand(latitude: number, longitude: number): boolean {
  const point = projectToMap(latitude, longitude);
  assert.ok(point, `(${latitude}, ${longitude}) should be on the map`);
  const { column, row } = cellOf(point);
  return LAND_MASK[row][column] === "#";
}

describe("land mask", () => {
  it("is 72 by 36 and only land or sea", () => {
    assert.equal(LAND_MASK.length, MASK_ROWS);
    for (const row of LAND_MASK) {
      assert.equal(row.length, MASK_COLUMNS);
      assert.match(row, /^[#.]+$/);
    }
  });
});

describe("projectToMap", () => {
  it("puts datacentre cities on land and open ocean on sea", () => {
    // If the projection and the mask ever disagree, pins drift into the ocean. These catch that.
    assert.ok(isLand(50.11, 8.68), "Frankfurt");
    assert.ok(isLand(39.04, -77.49), "Ashburn");
    assert.ok(isLand(-23.55, -46.63), "Sao Paulo");
    assert.ok(isLand(28.7, 77.1), "Delhi");
    assert.ok(!isLand(0, -150), "mid-Pacific");
    assert.ok(!isLand(30, -40), "mid-Atlantic");
  });

  it("returns null for what it cannot show", () => {
    assert.equal(projectToMap(null, 10), null);
    assert.equal(projectToMap(-75, 0), null, "Antarctica is cropped");
  });

  it("maps the corners of the box", () => {
    assert.deepEqual(projectToMap(84, -180), { xPercent: 0, yPercent: 0 });
    assert.deepEqual(projectToMap(-60, 180), { xPercent: 100, yPercent: 100 });
  });
});

describe("distanceKm", () => {
  it("is roughly right for a known pair", () => {
    const london = { latitude: 51.51, longitude: -0.13 };
    const frankfurt = { latitude: 50.11, longitude: 8.68 };
    const km = distanceKm(london, frankfurt);
    assert.ok(km > 600 && km < 700, `London-Frankfurt is ~640 km, got ${km}`);
  });
});
