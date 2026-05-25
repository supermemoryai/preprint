#!/usr/bin/env node

import { spawn, execSync } from "node:child_process";
import { accessSync, chmodSync, constants, existsSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { arch, platform } from "node:os";

const here = dirname(fileURLToPath(import.meta.url));

function isMusl() {
  if (platform() !== "linux") return false;
  try {
    const out = execSync("ldd --version 2>&1 || true", { encoding: "utf8" });
    return out.toLowerCase().includes("musl");
  } catch {
    return (
      existsSync("/lib/ld-musl-x86_64.so.1") ||
      existsSync("/lib/ld-musl-aarch64.so.1")
    );
  }
}

function platformKey() {
  const os = platform();
  const cpu = arch();
  let osKey;
  if (os === "darwin") osKey = "darwin";
  else if (os === "linux") osKey = isMusl() ? "linux-musl" : "linux";
  else if (os === "win32") osKey = "win32";
  else return null;

  let archKey;
  if (cpu === "arm64") archKey = "arm64";
  else if (cpu === "x64") archKey = "x64";
  else return null;

  return `${osKey}-${archKey}`;
}

function binaryFor(name) {
  const key = platformKey();
  if (!key) {
    console.error(
      `preprint: unsupported platform ${platform()}-${arch()}. ` +
        "Supported: darwin-arm64, darwin-x64, linux-x64, linux-arm64, linux-musl-x64, linux-musl-arm64, win32-x64.",
    );
    process.exit(1);
  }
  const ext = platform() === "win32" ? ".exe" : "";
  return join(here, `${name}-${key}${ext}`);
}

function ensureExecutable(path) {
  if (platform() === "win32") return;
  try {
    accessSync(path, constants.X_OK);
  } catch {
    try {
      chmodSync(path, 0o755);
    } catch {}
  }
}

const preprint = binaryFor("preprint");
if (!existsSync(preprint)) {
  console.error(
    `preprint: native binary not found at ${preprint}. ` +
      "Reinstall the package or report a bug at https://github.com/supermemoryai/preprint/issues.",
  );
  process.exit(1);
}
ensureExecutable(preprint);
ensureExecutable(binaryFor("agent-browser"));

const child = spawn(preprint, process.argv.slice(2), { stdio: "inherit" });

child.on("exit", (code, signal) => {
  if (signal) process.kill(process.pid, signal);
  else process.exit(code ?? 0);
});

for (const sig of ["SIGINT", "SIGTERM", "SIGHUP"]) {
  process.on(sig, () => {
    if (!child.killed) child.kill(sig);
  });
}
