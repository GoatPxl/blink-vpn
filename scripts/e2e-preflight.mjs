// Check the environment the real-Tauri E2E lane needs, and say what to do about anything missing.
//
// The connect spec installs and removes a WireGuard tunnel service, which is an administrator
// operation. Run from an elevated shell it completes silently; run unelevated it stops on a UAC
// dialog that Playwright cannot answer, and the test just times out with no explanation.

import { execFileSync, execSync } from "node:child_process";

const CONNECT_ONLY = process.argv.includes("--connect");

let failures = 0;
let warnings = 0;

function ok(label, detail) {
  console.log(`  PASS  ${label}${detail ? ` — ${detail}` : ""}`);
}

function fail(label, detail, fix) {
  failures += 1;
  console.log(`  FAIL  ${label}${detail ? ` — ${detail}` : ""}`);
  if (fix) console.log(`        fix: ${fix}`);
}

function warn(label, detail, fix) {
  warnings += 1;
  console.log(`  WARN  ${label}${detail ? ` — ${detail}` : ""}`);
  if (fix) console.log(`        fix: ${fix}`);
}

function tryRun(command, args) {
  try {
    return execFileSync(command, args, { encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] }).trim();
  } catch {
    return null;
  }
}

console.log("\nReal-Tauri E2E preflight\n");

// Docker: both specs provision a real container.
const dockerVersion = tryRun("docker", ["info", "--format", "{{.ServerVersion}}"]);
if (dockerVersion) {
  ok("Docker daemon reachable", `server ${dockerVersion}`);
} else {
  fail("Docker daemon unreachable", null, "start Docker, then re-run");
}

// WireGuard for Windows: the connect spec drives its tunnel service.
const wgPath = tryRun("where", ["wireguard.exe"]) ?? tryRun("where", ["wireguard"]);
if (wgPath) {
  ok("WireGuard for Windows installed", wgPath.split(/\r?\n/)[0]);
} else {
  fail("wireguard.exe not found on PATH", null, "install WireGuard for Windows");
}

// Elevation: the whole point of this script.
let elevated = false;
try {
  execSync("net session", { stdio: "ignore" });
  elevated = true;
} catch {
  elevated = false;
}

if (elevated) {
  ok("Shell is elevated", "connect will run with no UAC prompts");
} else if (CONNECT_ONLY) {
  fail(
    "Shell is NOT elevated",
    "the connect spec will stop on a UAC dialog Playwright cannot answer",
    'open a terminal with "Run as administrator" and re-run there',
  );
} else {
  warn(
    "Shell is NOT elevated",
    "fine for the provisioning spec; the connect spec needs elevation",
    'for connect: open a terminal with "Run as administrator"',
  );
}

// Leftovers from an interrupted run. Not fatal — the fixture resets nodes and kills stale app
// instances — but worth surfacing, because a stale config holds a private key.
const staleContainers = tryRun("docker", [
  "ps", "-aq", "--filter", "label=blink.provider=docker",
]);
if (staleContainers) {
  const count = staleContainers.split(/\s+/).filter(Boolean).length;
  if (count > 0) warn(`${count} leftover Blink container(s)`, null, "npm run docker:cleanup");
}

console.log(
  failures === 0
    ? `\nReady${warnings ? ` (${warnings} warning${warnings === 1 ? "" : "s"})` : ""}.\n`
    : `\n${failures} blocker${failures === 1 ? "" : "s"}. Resolve the above, then re-run.\n`,
);

process.exit(failures === 0 ? 0 : 1);
