import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { UBOT_MARK_PATH } from "./Icons";

const brandFile = (name: string) => readFileSync(fileURLToPath(new URL(`../../../docs/brand/${name}`, import.meta.url)), "utf8");

describe("brand", () => {
  it("draws the title-bar logo with the mark from docs/brand", () => {
    const d = /\sd="([^"]+)"/.exec(brandFile("ubot-mark.svg"))?.[1];
    expect(UBOT_MARK_PATH).toBe(d);
  });

  it("ships docs/brand/ubot-icon.svg as the favicon", () => {
    const favicon = readFileSync(fileURLToPath(new URL("../../public/favicon.svg", import.meta.url)), "utf8");
    expect(favicon).toBe(brandFile("ubot-icon.svg"));
  });
});
