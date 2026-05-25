#!/usr/bin/env node

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, "..");

const pkgVersion = JSON.parse(
  readFileSync(join(root, "package.json"), "utf8"),
).version;

const cargo = readFileSync(join(root, "Cargo.toml"), "utf8");
const cargoVersion = cargo.match(/^version\s*=\s*"([^"]*)"/m)?.[1];

if (!cargoVersion) {
  console.error("check-version-sync: no version in Cargo.toml");
  process.exit(1);
}

if (pkgVersion !== cargoVersion) {
  console.error(
    `version mismatch: package.json=${pkgVersion} Cargo.toml=${cargoVersion}. ` +
      "Run `npm run version:sync`.",
  );
  process.exit(1);
}

console.log(`versions in sync: ${pkgVersion}`);
