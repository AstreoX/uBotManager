/// <reference types="node" />
import { execFileSync } from "node:child_process";
import { createRequire } from "node:module";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import * as fixture from "./contract.fixture";

// contract.fixture.ts is JSON produced by the Rust model (src-tauri/src/contract.rs). Each value is
// assigned to its types.ts type, so the TypeScript compiler rejects a missing key, an extra key or
// an unknown literal on either side of the Tauri API.

const appDir = fileURLToPath(new URL("../..", import.meta.url));
const fixtureFile = fileURLToPath(new URL("./contract.fixture.ts", import.meta.url));

function typeCheck(file: string): string {
  const tsc = join(dirname(createRequire(import.meta.url).resolve("typescript/package.json")), "bin", "tsc");
  try {
    execFileSync(process.execPath, [
      tsc, "--ignoreConfig", "--noEmit", "--pretty", "false", "--strict", "--skipLibCheck",
      "--target", "es2022", "--module", "esnext", "--moduleResolution", "bundler",
      file
    ], { cwd: appDir, encoding: "utf8", stdio: "pipe" });
    return "";
  } catch (error) {
    const failed = error as { stdout?: string; stderr?: string; message: string };
    return `${failed.stdout ?? ""}${failed.stderr ?? ""}` || failed.message;
  }
}

describe("Rust model ↔ types.ts", () => {
  it("the Rust JSON type-checks against types.ts", () => {
    expect(typeCheck(fixtureFile)).toBe("");
  }, 60_000);

  it("covers commands and events", () => {
    expect(fixture.snapshots.length).toBeGreaterThan(0);
    expect(fixture.snapshots[0].project).not.toBeNull();
    expect(fixture.plans[0].steps.length).toBeGreaterThan(0);
    expect(fixture.tasks.map((task) => task.status)).toContain("canceled");
    expect(fixture.settings.map((settings) => settings.theme)).toEqual(["dark", "light", "system"]);
  });
});
