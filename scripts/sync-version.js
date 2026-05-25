#!/usr/bin/env node

import { execSync } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, "..");

const pkg = JSON.parse(readFileSync(join(root, "package.json"), "utf8"));
const version = pkg.version;

const cargoPath = join(root, "Cargo.toml");
const cargo = readFileSync(cargoPath, "utf8");
const versionRe = /^version\s*=\s*"[^"]*"/m;
const current = cargo.match(versionRe)?.[0];
const next = `version = "${version}"`;

if (!current) {
  console.error("sync-version: no version field in Cargo.toml");
  process.exit(1);
}

if (current === next) {
  console.log(`Cargo.toml already at ${version}`);
  process.exit(0);
}

writeFileSync(cargoPath, cargo.replace(versionRe, next));
console.log(`Cargo.toml: ${current} -> ${next}`);

try {
  execSync("cargo update -p preprint --offline", { cwd: root, stdio: "pipe" });
} catch {
  try {
    execSync("cargo update -p preprint", { cwd: root, stdio: "pipe" });
  } catch (err) {
    console.warn(`sync-version: could not refresh Cargo.lock (${err.message})`);
  }
}
