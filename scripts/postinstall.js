#!/usr/bin/env node

import { chmodSync, existsSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { arch, platform } from "node:os";

if (process.env.npm_config_ignore_scripts === "true") {
  process.exit(0);
}

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, "..");
const binDir = join(root, "bin");

function isMusl() {
  if (platform() !== "linux") return false;
  return (
    existsSync("/lib/ld-musl-x86_64.so.1") ||
    existsSync("/lib/ld-musl-aarch64.so.1")
  );
}

function platformKey() {
  const os = platform();
  const cpu = arch();
  let osKey;
  if (os === "darwin") osKey = "darwin";
  else if (os === "linux") osKey = isMusl() ? "linux-musl" : "linux";
  else if (os === "win32") osKey = "win32";
  else return null;
  const archKey = cpu === "arm64" ? "arm64" : cpu === "x64" ? "x64" : null;
  if (!archKey) return null;
  return `${osKey}-${archKey}`;
}

const key = platformKey();
if (!key) {
  console.warn(
    `preprint: unsupported platform ${platform()}-${arch()}; skipping native binary setup.`,
  );
  process.exit(0);
}

const ext = platform() === "win32" ? ".exe" : "";
const preprint = join(binDir, `preprint-${key}${ext}`);
const agentBrowser = join(binDir, `agent-browser-${key}${ext}`);

let missing = false;
for (const path of [preprint, agentBrowser]) {
  if (!existsSync(path)) {
    console.error(`preprint: missing binary ${path}`);
    missing = true;
    continue;
  }
  if (platform() !== "win32") {
    try {
      chmodSync(path, 0o755);
    } catch (err) {
      console.warn(`preprint: failed to chmod ${path}: ${err.message}`);
    }
  }
}

if (missing) {
  console.error(
    "preprint: one or more native binaries are missing from the package. " +
      "Reinstall or file a bug at https://github.com/supermemoryai/preprint/issues.",
  );
  process.exit(1);
}

const pkg = JSON.parse(readFileSync(join(root, "package.json"), "utf8"));
console.log(`preprint ${pkg.version} ready (${key}).`);
