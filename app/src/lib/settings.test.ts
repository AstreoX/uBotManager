import { describe, expect, it } from "vitest";
import type { Settings } from "../api/types";
import { parsePort, rebaseSettings, sameJson } from "./settings";

const settings = (): Settings => ({
  language: "en",
  theme: "dark",
  index: {
    sources: [{ id: "ubot", name: "uBotCore", location: "https://example.com/index.json", enabled: true, builtin: true }],
    syncOnStart: true,
    showPlanned: true
  },
  install: { directory: "Plugins/uBot", git: "git", usePrebuilt: false },
  build: { engineId: null, configuration: "Development", waitForEditor: true, reopenEditor: true, runTests: false },
  ros: { host: "127.0.0.1", port: 8268, checkOnRefresh: true },
  recentProjects: ["E:/A/A.uproject"],
  lastProject: "E:/A/A.uproject"
});

describe("parsePort", () => {
  it("accepts 1-65535 and nothing else", () => {
    expect(parsePort("8268")).toBe(8268);
    expect(parsePort(" 1 ")).toBe(1);
    expect(parsePort("65535")).toBe(65535);
    for (const bad of ["", "0", "65536", "99999", "123456", "abc", "80a", "8080abc", "-1", "1.5", "8 0"]) {
      expect(parsePort(bad), bad).toBeNull();
    }
  });
});

describe("rebaseSettings", () => {
  it("follows the backend for everything the user has not edited", () => {
    const base = settings();
    const draft = settings();
    const fresh = settings();
    // A second launch switched to project B, whose ini has other ROS values.
    fresh.ros = { host: "10.0.0.7", port: 9090, checkOnRefresh: true };
    fresh.recentProjects = ["E:/B/B.uproject", "E:/A/A.uproject"];
    fresh.lastProject = "E:/B/B.uproject";
    expect(rebaseSettings(draft, base, fresh)).toEqual(fresh);
  });

  it("keeps what the user edited, key by key", () => {
    const base = settings();
    const draft = settings();
    draft.theme = "light";
    draft.ros.checkOnRefresh = false;
    const fresh = settings();
    fresh.ros = { host: "10.0.0.7", port: 9090, checkOnRefresh: true };
    fresh.lastProject = "E:/B/B.uproject";
    const merged = rebaseSettings(draft, base, fresh);
    expect(merged.theme).toBe("light");
    // The untouched host and port follow the new project; the toggle the user flipped stays.
    expect(merged.ros).toEqual({ host: "10.0.0.7", port: 9090, checkOnRefresh: false });
    expect(merged.lastProject).toBe("E:/B/B.uproject");
  });

  it("replaces lists as a whole", () => {
    const base = settings();
    const draft = settings();
    draft.index.sources.push({ id: "user-1", name: "New source", location: "", enabled: true, builtin: false });
    const fresh = settings();
    fresh.index.syncOnStart = false;
    const merged = rebaseSettings(draft, base, fresh);
    expect(merged.index.sources).toHaveLength(2);
    expect(merged.index.syncOnStart).toBe(false);
  });

  it("does not modify its arguments", () => {
    const base = settings();
    const draft = settings();
    draft.theme = "light";
    const fresh = settings();
    fresh.ros.port = 1;
    rebaseSettings(draft, base, fresh);
    expect(sameJson(base, settings())).toBe(true);
    expect(draft.ros.port).toBe(8268);
  });
});
