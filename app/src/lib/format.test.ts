import { afterEach, describe, expect, it } from "vitest";
import type { PackageInfo, Problem, Requirement, Snapshot } from "../api/types";
import { setLanguage } from "../i18n";
import {
  changeText, defaultVersion, detailsKey, displayUrl, formatDuration, isEditorRunning, problemCounts, problemGroupTitle,
  problemsFor, sameProjectPath, upsertChange, versionChoices
} from "./format";
import { layoutGraph } from "./graphLayout";
import { compareVersionStrings, satisfies } from "./semver";

afterEach(() => setLanguage("en"));

describe("formatDuration", () => {
  it("formats like the design in English", () => {
    setLanguage("en");
    expect(formatDuration(400)).toBe("< 1 s");
    expect(formatDuration(4_200)).toBe("4 s");
    expect(formatDuration(65_000)).toBe("1 min 05 s");
    expect(formatDuration(72_000)).toBe("1 min 12 s");
    expect(formatDuration(3_725_000)).toBe("1 h 02 min");
  });

  it("formats like the design in Chinese", () => {
    setLanguage("zh-CN");
    expect(formatDuration(400)).toBe("<1 秒");
    expect(formatDuration(4_200)).toBe("4 秒");
    expect(formatDuration(65_000)).toBe("1 分 5 秒");
  });
});

describe("changes", () => {
  const pkg = { name: "UBotVehicles", latestVersion: "0.1.0" } as PackageInfo;

  it("describes pending changes", () => {
    setLanguage("en");
    expect(changeText({ kind: "install", package: "UBotVehicles", version: null }, pkg)).toBe("Install UBotVehicles 0.1.0");
    expect(changeText({ kind: "update", package: "UBotSensor", version: "0.1.1" }, undefined)).toBe("Update UBotSensor to 0.1.1");
    setLanguage("zh-CN");
    expect(changeText({ kind: "install", package: "UBotVehicles", version: "0.1.0" }, pkg)).toBe("安装 UBotVehicles 0.1.0");
    expect(changeText({ kind: "update", package: "UBotSensor", version: "0.1.1" }, undefined)).toBe("将 UBotSensor 更新到 0.1.1");
  });

  it("keeps one change per package", () => {
    const list = upsertChange([{ kind: "install", package: "UBotMCP", version: null }], { kind: "uninstall", package: "ubotmcp", version: null });
    expect(list).toEqual([{ kind: "uninstall", package: "ubotmcp", version: null }]);
  });

  it("shortens repository URLs", () => {
    expect(displayUrl("https://github.com/AstreoX/uBotSensor.git")).toBe("github.com/AstreoX/uBotSensor");
    expect(displayUrl("https://user:token@example.com/x")).toBe("example.com/x");
  });
});

const emptyPackage = (patch: Partial<PackageInfo>): PackageInfo => ({
  name: "UBotSensor", friendlyName: "uBot Sensor", description: { en: "", "zh-CN": null }, layer: "Capability", state: "enabled",
  installedVersion: null, latestVersion: null, versions: [], plannedVersion: null, updateAvailable: false, location: null, git: null,
  repository: null, webUrl: null, docsUrl: null, indexSource: null, requires: [], provides: [], tags: [], problemIds: [], ...patch
});

const problem = (id: string, severity: Problem["severity"], group = "UBotSensor"): Problem => ({
  id, scope: "packages", group, severity, message: { code: "text", params: { text: id } }, facts: [], fixes: []
});

describe("problems", () => {
  it("count errors and warnings for the status bar, not info", () => {
    const list = [problem("a", "error"), problem("b", "warning"), problem("c", "warning"), problem("ros:bridge", "info")];
    expect(problemCounts(list)).toEqual({ errors: 1, warnings: 2 });
    expect(problemCounts([problem("ros:bridge", "info")])).toEqual({ errors: 0, warnings: 0 });
    expect(problemCounts([])).toEqual({ errors: 0, warnings: 0 });
  });

  it("translate the index group name and leave package and file names alone", () => {
    setLanguage("en");
    expect(problemGroupTitle("Package Index")).toBe("Package Index");
    expect(problemGroupTitle("UBotSensor")).toBe("UBotSensor");
    setLanguage("zh-CN");
    expect(problemGroupTitle("Package Index")).toBe("Package 索引");
    expect(problemGroupTitle("UBotSensor")).toBe("UBotSensor");
    expect(problemGroupTitle("MyRobotSim.uproject")).toBe("MyRobotSim.uproject");
  });

  it("belong to a package only when its problemIds list them", () => {
    const list = [problem("requires:UBotSensor:UBotCore", "warning"), problem("index:ubot", "warning", "Package Index")];
    const sensor = emptyPackage({ problemIds: ["requires:UBotSensor:UBotCore"] });
    const core = emptyPackage({ name: "UBotCore", problemIds: [] });
    expect(problemsFor(sensor, list).map((p) => p.id)).toEqual(["requires:UBotSensor:UBotCore"]);
    expect(problemsFor(core, list)).toEqual([]);
  });
});

describe("projects", () => {
  it("compare paths case-insensitively with either slash", () => {
    expect(sameProjectPath("E:\\Sims\\A\\A.uproject", "e:/sims/a/a.uproject")).toBe(true);
    expect(sameProjectPath("E:/Sims/A/A.uproject", "E:/Sims/B/B.uproject")).toBe(false);
    expect(sameProjectPath(null, null)).toBe(true);
    expect(sameProjectPath(null, "E:/Sims/A/A.uproject")).toBe(false);
  });

  it("know whether the editor runs from a snapshot", () => {
    const withProject = (running: boolean) => ({ project: { editor: { running, pid: running ? 1 : null } } }) as unknown as Snapshot;
    expect(isEditorRunning(withProject(true))).toBe(true);
    expect(isEditorRunning(withProject(false))).toBe(false);
    expect(isEditorRunning({ project: null } as unknown as Snapshot)).toBe(false);
  });
});

describe("version choice", () => {
  it("starts on the update when one is available, else on the installed version", () => {
    expect(defaultVersion(emptyPackage({ installedVersion: "0.1.0", latestVersion: "0.1.1", updateAvailable: true }))).toBe("0.1.1");
    expect(defaultVersion(emptyPackage({ installedVersion: "0.1.0", latestVersion: "0.1.1", updateAvailable: false }))).toBe("0.1.0");
    expect(defaultVersion(emptyPackage({ state: "notInstalled", latestVersion: "0.2.0" }))).toBe("0.2.0");
    expect(defaultVersion(emptyPackage({ state: "planned" }))).toBe("");
  });

  it("starts over when a sync changes what it is derived from", () => {
    // The cached index knew 0.1.0 only; the sync then finds 0.1.1: the pane must not stay on 0.1.0.
    const cached = emptyPackage({ installedVersion: "0.1.0", latestVersion: "0.1.0", versions: ["0.1.0"] });
    const synced = emptyPackage({ installedVersion: "0.1.0", latestVersion: "0.1.1", versions: ["0.1.1", "0.1.0"], updateAvailable: true });
    expect(detailsKey(cached)).not.toBe(detailsKey(synced));
    expect(defaultVersion(synced)).toBe("0.1.1");
    const uninstalled = emptyPackage({ state: "notInstalled", latestVersion: "0.1.0", versions: ["0.1.0"] });
    const newer = emptyPackage({ state: "notInstalled", latestVersion: "0.2.0", versions: ["0.2.0", "0.1.0"] });
    expect(detailsKey(uninstalled)).not.toBe(detailsKey(newer));
    expect(detailsKey(synced)).toBe(detailsKey({ ...synced, problemIds: ["x"] }));
  });

  it("offers the index versions, the installed one and the queued one, newest first", () => {
    const pkg = emptyPackage({ installedVersion: "0.1.0-dev", versions: ["0.2.0", "0.1.0"] });
    expect(versionChoices(pkg, null)).toEqual(["0.2.0", "0.1.0", "0.1.0-dev"]);
    expect(versionChoices(pkg, "0.3.0")).toEqual(["0.3.0", "0.2.0", "0.1.0", "0.1.0-dev"]);
    expect(versionChoices(pkg, "0.2.0")).toEqual(["0.2.0", "0.1.0", "0.1.0-dev"]);
  });
});

describe("semver (mock backend)", () => {
  it("matches caret, tilde and partial constraints", () => {
    expect(satisfies("0.1.0", "^0.1.0")).toBe(true);
    expect(satisfies("0.1.1", "^0.1.0")).toBe(true);
    expect(satisfies("0.2.0", "^0.1.0")).toBe(false);
    expect(satisfies("0.1.0", "^0.2.0")).toBe(false);
    expect(satisfies("5.5.4", "~5.5")).toBe(true);
    expect(satisfies("5.6.0", "~5.5")).toBe(false);
    expect(satisfies("1.4.0", "1")).toBe(true);
    expect(satisfies("2.0.0-rc.1", "<2.0.0")).toBe(true);
    expect(satisfies("2.0.0-rc.1", "^1.0.0")).toBe(false);
    expect(satisfies("1.2.3", ">=1.0.0 <2")).toBe(true);
    expect(satisfies("v1.2.3", "*")).toBe(true);
  });

  it("orders versions by precedence", () => {
    expect(["0.1.0", "0.10.0", "0.2.0", "0.2.0-rc.1"].sort(compareVersionStrings)).toEqual(["0.1.0", "0.2.0-rc.1", "0.2.0", "0.10.0"]);
  });
});

describe("layoutGraph", () => {
  const req = (name: string, status: Requirement["status"] = "satisfied"): Requirement =>
    ({ name, constraint: "^0.1.0", optional: false, kind: "package", status, installedVersion: null });
  const pkg = (name: string, layer: PackageInfo["layer"], state: PackageInfo["state"], requires: Requirement[]): PackageInfo => ({
    name, friendlyName: name, description: { en: "", "zh-CN": null }, layer, state, installedVersion: null, latestVersion: null,
    versions: [], plannedVersion: null, updateAvailable: false, location: null, git: null, repository: null, webUrl: null,
    docsUrl: null, indexSource: null, requires, provides: [], tags: [], problemIds: []
  });
  const packages = [
    pkg("UBotCore", "Foundation", "enabled", []),
    pkg("UBotSensor", "Capability", "enabled", [req("UBotCore")]),
    pkg("UBotEnvironment", "Capability", "enabled", [req("UBotCore", "versionMismatch")]),
    pkg("UBotROS", "Adapter", "enabled", [req("UBotCore"), req("UBotSensor")]),
    pkg("UBotMCP", "Adapter", "notInstalled", [req("UBotCore"), req("UBotSensor")]),
    pkg("UBotCombat", "Content", "disabled", [req("UBotCore"), req("UBotSensor"), req("UBotEnvironment")])
  ];
  const layout = layoutGraph({ packages, hasProblem: (p) => p.name === "UBotEnvironment" });

  it("collapses a foundation package everybody requires into one tall card", () => {
    expect(layout.collapsed).toBe("UBotCore");
    const core = layout.nodes.find((n) => n.pkg.name === "UBotCore");
    expect(core).toMatchObject({ x: 48, y: 48, w: 160, tall: true });
    expect(layout.edges.some((e) => e.from === "UBotCore")).toBe(false);
  });

  it("places layers in columns with the design's spacing", () => {
    expect(layout.columns.map((c) => c.x)).toEqual([48, 304, 600, 896]);
    expect(layout.width).toBe(1144);
    const env = layout.nodes.find((n) => n.pkg.name === "UBotEnvironment");
    expect(env?.row).toBe(1);
  });

  it("routes edges from dependency to dependent and flags problem sources", () => {
    const ids = layout.edges.map((e) => e.id).sort();
    expect(ids).toEqual(["UBotEnvironment->UBotCombat", "UBotSensor->UBotCombat", "UBotSensor->UBotMCP", "UBotSensor->UBotROS"]);
    expect(layout.edges.find((e) => e.id === "UBotEnvironment->UBotCombat")?.problem).toBe(true);
    expect(layout.edges.find((e) => e.id === "UBotSensor->UBotROS")?.path).toBe("M 504 72 L 595 72");
    expect(layout.edges.find((e) => e.id === "UBotSensor->UBotCombat")?.path).toMatch(/^M 404 48 L 404 24 L 996 24 L 996 43$/);
  });
});
