// Draws the app icon from the pixel goat, so the installer, taskbar, window and favicon all match
// the logo in the rail.
//
// The goat's grid is read from src/components/ui/pixel-icon.tsx — the one place it is drawn — so a
// redraw there reaches every icon on the next `npm run icons`. No dependencies: PNG is written with
// Node's own zlib.
//
// The goat sits on a 16×16 cell grid, 64 px per cell at 1024 px. Every size `tauri icon` derives
// from it (16, 32, 128, 256…) is then a whole number of pixels per cell, so the pixels stay pixels.

import { readFileSync, writeFileSync, mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { deflateSync } from "node:zlib";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const SOURCE = join(root, "src/components/ui/pixel-icon.tsx");
const OUT_ICON = join(root, "design/app-icon.png");
const OUT_FAVICON = join(root, "src/app/icon.png");

const INK = [0x0b, 0x0b, 0x0a];
const LIME = [0xd4, 0xff, 0x3a];
const GRID = 16;

function readGoat() {
  const source = readFileSync(SOURCE, "utf8");
  const block = source.match(/const GOAT = \[([\s\S]*?)\] as const/);
  if (!block) throw new Error(`GOAT grid not found in ${SOURCE}`);
  const rows = [...block[1].matchAll(/"([.#]+)"/g)].map((match) => match[1]);
  if (rows.length === 0 || rows.some((row) => row.length !== rows[0].length)) {
    throw new Error("GOAT grid is empty or ragged");
  }
  return rows;
}

const CRC_TABLE = Array.from({ length: 256 }, (_, n) => {
  let c = n;
  for (let k = 0; k < 8; k += 1) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
  return c >>> 0;
});

function crc32(buffer) {
  let c = 0xffffffff;
  for (const byte of buffer) c = CRC_TABLE[(c ^ byte) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}

function chunk(type, data) {
  const length = Buffer.alloc(4);
  length.writeUInt32BE(data.length);
  const body = Buffer.concat([Buffer.from(type, "ascii"), data]);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(body));
  return Buffer.concat([length, body, crc]);
}

/** An RGBA PNG of the goat, `size` px square. */
function renderPng(goat, size) {
  if (size % GRID !== 0) throw new Error(`size ${size} is not a multiple of ${GRID}`);
  const cell = size / GRID;
  // Centred: 12 wide leaves 2 cells either side; the goat's last row is blank, so 2 cells on top
  // centres what is drawn.
  const left = Math.floor((GRID - goat[0].length) / 2);
  const top = 2;

  const raw = Buffer.alloc(size * (size * 4 + 1));
  for (let y = 0; y < size; y += 1) {
    const rowStart = y * (size * 4 + 1);
    raw[rowStart] = 0; // filter: none
    for (let x = 0; x < size; x += 1) {
      const gx = Math.floor(x / cell) - left;
      const gy = Math.floor(y / cell) - top;
      const lit = gy >= 0 && gy < goat.length && gx >= 0 && gx < goat[0].length && goat[gy][gx] === "#";
      const [r, g, b] = lit ? LIME : INK;
      raw.set([r, g, b, 0xff], rowStart + 1 + x * 4);
    }
  }

  const header = Buffer.alloc(13);
  header.writeUInt32BE(size, 0);
  header.writeUInt32BE(size, 4);
  header[8] = 8; // bit depth
  header[9] = 6; // RGBA
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk("IHDR", header),
    chunk("IDAT", deflateSync(raw, { level: 9 })),
    chunk("IEND", Buffer.alloc(0)),
  ]);
}

const goat = readGoat();
mkdirSync(dirname(OUT_ICON), { recursive: true });
writeFileSync(OUT_ICON, renderPng(goat, 1024));
// Next serves src/app/icon.png as the page icon; 64 px keeps it crisp in a tab.
writeFileSync(OUT_FAVICON, renderPng(goat, 64));
console.log(`wrote ${OUT_ICON} and ${OUT_FAVICON}`);
