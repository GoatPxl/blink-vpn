/**
 * Turn a QR module matrix into a single SVG path.
 *
 * The backend returns the matrix rather than an image so the code can be drawn at whatever size the
 * layout wants without resampling. Drawing it as one `<path>` instead of one `<rect>` per module
 * matters: a config-sized symbol is around 45x45, and the dark half of that is roughly a thousand
 * elements React would otherwise have to reconcile on every render.
 *
 * This follows the same shape as `buildLinePath` in the overview chart — pure data-to-path logic
 * here, `<svg>` in the component.
 */

/**
 * Build the `d` attribute for a QR symbol on a 1-unit-per-module grid.
 *
 * Pair with `viewBox="0 0 {size} {size}"`. Horizontally adjacent dark modules are merged into one
 * run, which roughly halves the path length for no change in what is drawn.
 *
 * Returns `""` when the matrix does not describe a square of `size` — that is a programming error
 * rather than a user-facing one, and the caller renders a fallback instead of a blank square.
 */
export function buildQrPath(modules: readonly boolean[], size: number): string {
  if (!Number.isInteger(size) || size <= 0) {
    return "";
  }

  if (modules.length !== size * size) {
    return "";
  }

  const runs: string[] = [];

  for (let row = 0; row < size; row += 1) {
    let column = 0;

    while (column < size) {
      if (!modules[row * size + column]) {
        column += 1;
        continue;
      }

      let run = 1;
      while (column + run < size && modules[row * size + column + run]) {
        run += 1;
      }

      runs.push(`M${column} ${row}h${run}v1h-${run}z`);
      column += run;
    }
  }

  return runs.join("");
}
