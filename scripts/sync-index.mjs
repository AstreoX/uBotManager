#!/usr/bin/env node
// Copies uBotCore's package index (Index/index.json) to crates/ubot-core/assets/index.json, the
// copy embedded in the manager as the last-resort fallback (SPEC §1.1).
//
// Usage: node scripts/sync-index.mjs [--from <index.json>] [--check]
//   --from   source file (default: $UBOTCORE_INDEX, else ../../Plugins/uBot/uBotCore/Index/index.json
//            relative to this repository, the layout of the uBot workspace)
//   --check  only compare; exit 1 when the embedded copy is out of date

import { readFileSync, writeFileSync, mkdirSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const target = resolve(repoRoot, "crates", "ubot-core", "assets", "index.json");

function parseArgs(argv) {
  const options = { from: process.env.UBOTCORE_INDEX || null, check: false };
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    if (arg === "--check") {
      options.check = true;
    } else if (arg === "--from") {
      options.from = argv[++i];
      if (!options.from) {
        fail("--from needs a path");
      }
    } else {
      fail(`unknown argument ${arg}`);
    }
  }
  options.from = resolve(options.from || resolve(repoRoot, "..", "..", "Plugins", "uBot", "uBotCore", "Index", "index.json"));
  return options;
}

function fail(message) {
  console.error(`sync-index: ${message}`);
  process.exit(2);
}

function readText(path) {
  try {
    return readFileSync(path, "utf8");
  } catch (error) {
    return null;
  }
}

const options = parseArgs(process.argv.slice(2));
const source = readText(options.from);
if (source === null) {
  fail(`cannot read ${options.from}`);
}

let parsed;
try {
  parsed = JSON.parse(source.replace(/^﻿/, ""));
} catch (error) {
  fail(`${options.from} is not valid JSON: ${error.message}`);
}
if (parsed.formatVersion !== 2 || !Array.isArray(parsed.packages)) {
  fail(`${options.from} is not a format 2 package index`);
}

const current = readText(target);
if (options.check) {
  if (current === source) {
    console.log(`sync-index: ${target} is up to date`);
    process.exit(0);
  }
  console.error(`sync-index: ${target} differs from ${options.from}; run node scripts/sync-index.mjs`);
  process.exit(1);
}

if (current === source) {
  console.log(`sync-index: already up to date (${parsed.packages.length} packages)`);
} else {
  mkdirSync(dirname(target), { recursive: true });
  writeFileSync(target, source, "utf8");
  console.log(`sync-index: copied ${options.from} -> ${target} (${parsed.packages.length} packages)`);
}
