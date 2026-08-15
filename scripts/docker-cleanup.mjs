// Remove containers created by the Docker dev provider.
//
// Scoped strictly to the `goatstare.provider=docker` label that the adapter sets on every
// container it creates. Nothing else on the host is touched — filtering by image or name would
// risk removing unrelated containers that happen to share a base image.

import { execFileSync } from "node:child_process";

const LABEL = "goatstare.provider=docker";

function docker(args) {
  return execFileSync("docker", args, { encoding: "utf8" });
}

let ids;
try {
  ids = docker(["ps", "-aq", "--filter", `label=${LABEL}`])
    .split(/\s+/)
    .filter(Boolean);
} catch (error) {
  console.error(`Could not query Docker. Is the daemon running?\n${error.message}`);
  process.exit(1);
}

if (ids.length === 0) {
  console.log("No GoatStare containers to remove.");
  process.exit(0);
}

// Report what is being removed before removing it, so an unexpected match is visible.
try {
  const listing = docker([
    "ps",
    "-a",
    "--filter",
    `label=${LABEL}`,
    "--format",
    "{{.Names}}\t{{.Status}}",
  ]).trim();
  console.log(`Removing ${ids.length} GoatStare container(s):\n${listing}`);
} catch {
  console.log(`Removing ${ids.length} GoatStare container(s).`);
}

try {
  docker(["rm", "-f", ...ids]);
  console.log("Done.");
} catch (error) {
  console.error(`Failed to remove containers:\n${error.message}`);
  process.exit(1);
}
