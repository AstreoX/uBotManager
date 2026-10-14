#!/usr/bin/env node
// Copies uBotCore's package index (Index/index.json) to crates/ubot-core/assets/index.json, the
// copy embedded in the manager as the last-resort fallback (SPEC §1.1).
//
// Usage: node scripts/sync-index.mjs [--from <index.json>] [--check]
//   --from   source file (default: $UBOTCORE_INDEX, else ../../Plugins/uBot/uBotCore/Index/index.json
//            relative to this repository, the layout of the uBot workspace)
//   --check  only compare; exit 1 when the embedded copy is out of date
//            if no source is specified and the workspace file is unreadable, compare with
//            https://raw.githubusercontent.com/AstreoX/uBotCore/main/Index/index.json (20s timeout)
//            set UBOT_SKIP_INDEX_CHECK=1 to warn and skip the check for offline builds
//   Write mode always requires a local source; source read/download/validation errors exit 2.

import { readFileSync, writeFileSync, mkdirSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const target = resolve(repoRoot, "crates", "ubot-core", "assets", "index.json");
const remoteIndex = "https://raw.githubusercontent.com/AstreoX/uBotCore/main/Index/index.json";

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
  options.useDefaultSource = !options.from;
  options.from = resolve(options.from || resolve(repoRoot, "..", "..", "Plugins", "uBot", "uBotCore", "Index", "index.json"));
  return options;
}

// Ends the run through the exit code instead of process.exit(): after a fetch, process.exit aborts Node on
// Windows (an assertion in libuv) and the exit code is lost.
class Exit extends Error {
  constructor(code) {
    super(`exit ${code}`);
    this.code = code;
  }
}

function fail(message) {
  console.error(`sync-index: ${message}`);
  throw new Exit(2);
}

// Line endings depend on the git settings of the checkout; only the content decides.
function sameText(a, b) {
  return a !== null && a.replace(/\r\n/g, "\n") === b.replace(/\r\n/g, "\n");
}

function readText(path) {
  try {
    return readFileSync(path, "utf8");
  } catch (error) {
    return null;
  }
}

async function main() {
  const options = parseArgs(process.argv.slice(2));
  if (options.check && process.env.UBOT_SKIP_INDEX_CHECK === "1") {
    console.warn("sync-index: warning: index check skipped (UBOT_SKIP_INDEX_CHECK=1)");
    throw new Exit(0);
  }

  let sourcePath = options.from;
  let source = readText(sourcePath);
  if (source === null && options.check && options.useDefaultSource) {
    sourcePath = remoteIndex;
    console.log(`sync-index: cannot read ${options.from}; checking ${sourcePath}`);
    let problem = null;
    try {
      const response = await fetch(sourcePath, { signal: AbortSignal.timeout(20_000) });
      if (response.status === 200) {
        source = await response.text();
      } else {
        problem = `HTTP ${response.status} ${response.statusText}`;
      }
    } catch (error) {
      const cause = error.cause?.message;
      problem = `${error.message}${cause ? ` (${cause})` : ""}`;
    }
    if (problem !== null) {
      fail(`cannot download ${sourcePath}: ${problem}`);
    }
  }
  if (source === null) {
    fail(`cannot read ${sourcePath}`);
  }

  let parsed;
  try {
    parsed = JSON.parse(source.replace(/^﻿/, ""));
  } catch (error) {
    fail(`${sourcePath} is not valid JSON: ${error.message}`);
  }
  if (parsed?.formatVersion !== 2 || !Array.isArray(parsed.packages)) {
    fail(`${sourcePath} is not a format 2 package index`);
  }

  const current = readText(target);
  if (options.check) {
    if (sameText(current, source)) {
      console.log(`sync-index: ${target} is up to date`);
      throw new Exit(0);
    }
    console.error(`sync-index: ${target} differs from ${sourcePath}; run node scripts/sync-index.mjs`);
    throw new Exit(1);
  }

  if (current === source) {
    console.log(`sync-index: already up to date (${parsed.packages.length} packages)`);
  } else {
    mkdirSync(dirname(target), { recursive: true });
    writeFileSync(target, source, "utf8");
    console.log(`sync-index: copied ${sourcePath} -> ${target} (${parsed.packages.length} packages)`);
  }
}

try {
  await main();
} catch (error) {
  if (!(error instanceof Exit)) {
    throw error;
  }
  process.exitCode = error.code;
}
