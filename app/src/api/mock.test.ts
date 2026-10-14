import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { afterEach, describe, expect, it } from "vitest";
import { localized, setLanguage } from "../i18n";
import { errorText } from "../lib/errors";
import { canRerun } from "../lib/tasks";
import { createMockBackend, downloadSource } from "./mock";
import mockIndex from "./mockIndex.json";
import type { MockParams } from "./mockParams";
import type { Backend } from "./backend";
import type { Snapshot } from "./types";

const params: MockParams = {
  lang: null, theme: null, view: null, dialog: null, section: null, select: null, noProject: false, prebuilt: true
};
const SCENARIO = "D:\\Projects\\MyRobotSim\\MyRobotSim.uproject";
const WAREHOUSE = "D:\\Projects\\WarehouseBot\\WarehouseBot.uproject";

afterEach(() => setLanguage("en"));

describe("mock backend settings", () => {
  it("stores what the backend would: blank sources dropped, values trimmed", async () => {
    const { backend } = createMockBackend(params);
    const settings = await backend.getSettings();
    settings.index.sources.push(
      { id: "user-a", name: "New source", location: "", enabled: true, builtin: false },
      { id: "user-b", name: "  Team  ", location: " D:\\Shared\\index.json ", enabled: true, builtin: false }
    );
    settings.install.git = "  ";
    await backend.saveSettings(settings);
    const stored = await backend.getSettings();
    expect(stored.index.sources.map((s) => s.id)).toEqual(["ubot", "local", "team", "user-b"]);
    expect(stored.index.sources[3]).toMatchObject({ name: "Team", location: "D:\\Shared\\index.json" });
    expect(stored.install.git).toBe("git");
  });

  it("keeps the recent projects and the last project of the backend when a stale copy is saved", async () => {
    const { backend } = createMockBackend(params);
    const stale = await backend.getSettings();
    // A second launch opens another project behind the settings dialog.
    await backend.openProject(WAREHOUSE);
    stale.theme = "light";
    await backend.saveSettings(stale);
    const stored = await backend.getSettings();
    expect(stored.theme).toBe("light");
    expect(stored.lastProject).toBe(WAREHOUSE);
    expect(stored.recentProjects[0]).toBe(WAREHOUSE);
  });

  it("reads and writes ROS host and port per project, only writing a changed one", async () => {
    const { backend } = createMockBackend(params);
    const a = await backend.getSettings();
    expect(a.ros.host).toBe("127.0.0.1");
    await backend.openProject(WAREHOUSE);
    const stale = a; // the draft the dialog opened with, for project A
    const b = await backend.getSettings();
    expect(b.ros).toMatchObject({ host: "10.0.0.7", port: 9090 });
    // Saving the stale draft with untouched ROS values must not write A's host and port into B's ini ...
    stale.theme = "light";
    await backend.saveSettings({ ...stale, ros: b.ros });
    expect((await backend.getSettings()).ros).toMatchObject({ host: "10.0.0.7", port: 9090 });
    // ... while a deliberate change is written, for that project only.
    await backend.saveSettings({ ...b, ros: { ...b.ros, port: 9191 } });
    expect((await backend.getSettings()).ros.port).toBe(9191);
    await backend.openProject(SCENARIO);
    expect((await backend.getSettings()).ros).toMatchObject({ host: "127.0.0.1", port: 8268 });
  });

  it("answers open_project with the snapshot only, like the shell (no event)", async () => {
    const { backend } = createMockBackend(params);
    const events: Snapshot[] = [];
    backend.onSnapshot((s) => events.push(s));
    const snapshot = await backend.openProject(WAREHOUSE);
    expect(snapshot.project?.path).toBe(WAREHOUSE);
    expect(events).toEqual([]);
  });
});

describe("mock backend tasks", () => {
  it("rejects a second apply with the English text the core uses, which the UI translates", async () => {
    const { backend } = createMockBackend(params);
    const options = { build: false, reopenEditor: false, runTests: false };
    const id = await backend.apply([{ kind: "install", package: "UBotMCP", version: null }], options, null, null);
    const second = backend.apply([{ kind: "install", package: "UBotVehicles", version: null }], options, null, null);
    await expect(second).rejects.toThrow("Another task is running.");
    setLanguage("zh-CN");
    const error: unknown = await second.catch((e: unknown) => e);
    expect(errorText(error)).toBe("另一个任务正在运行。");
    await backend.cancelTask(id);
  });

  it("rejects an apply with one English message per line, which the UI translates line by line", async () => {
    const { backend } = createMockBackend(params);
    const options = { build: false, reopenEditor: false, runTests: false };
    const rejected: unknown = await backend.apply([
      { kind: "uninstall", package: "UBotVehicles", version: null },
      { kind: "uninstall", package: "UBotMCP", version: null }
    ], options, null, null).catch((e: unknown) => e);
    expect(rejected).toBeInstanceOf(Error);
    expect((rejected as Error).message).toBe("UBotVehicles is not installed.\nUBotMCP is not installed.");
    setLanguage("zh-CN");
    expect(errorText(rejected)).toBe("UBotVehicles 尚未安装。\nUBotMCP 尚未安装。");
  });

  it("refuses to open another project while a task runs, like the core, and opens it afterwards", async () => {
    const { backend } = createMockBackend(params);
    const options = { build: false, reopenEditor: false, runTests: false };
    const id = await backend.apply([{ kind: "install", package: "UBotMCP", version: null }], options, null, null);
    const refused = backend.openProject(WAREHOUSE);
    await expect(refused).rejects.toThrow("Another task is running.");
    setLanguage("zh-CN");
    expect(errorText(await refused.catch((e: unknown) => e))).toBe("另一个任务正在运行。");
    await backend.cancelTask(id);
    await untilIdle(backend);
    expect((await backend.openProject(WAREHOUSE)).project?.path).toBe(WAREHOUSE);
  });

  it("records the project's name and path in history and new tasks", async () => {
    const { backend } = createMockBackend(params);
    const history = (await backend.listTasks()).find((x) => x.id === "task-0");
    expect(history?.project).toBe("MyRobotSim");
    expect(history?.projectPath).toBe(SCENARIO);
    const scenario = await backend.getSnapshot();
    expect(canRerun(history, undefined, scenario.project)).toBe(true);

    const options = { build: false, reopenEditor: false, runTests: false };
    const id = await backend.apply([{ kind: "install", package: "UBotMCP", version: null }], options, null, null);
    expect((await backend.listTasks()).find((x) => x.id === id)).toMatchObject({ project: "MyRobotSim", projectPath: SCENARIO });
    await backend.cancelTask(id);
    await untilIdle(backend);

    // Another project is open now: the history of the first one cannot be rerun in it.
    const other = await backend.openProject(WAREHOUSE);
    expect(other.project?.name).toBe("WarehouseBot");
    expect(canRerun(history, undefined, other.project)).toBe(false);
  });

  it("keeps task paths when a same-named project in another directory is opened", async () => {
    const { backend } = createMockBackend(params);
    const history = (await backend.listTasks()).find((x) => x.id === "task-0");
    const path = "E:\\Other\\MyRobotSim\\MyRobotSim.uproject";
    const other = await backend.openProject(path);
    expect(other.project?.name).toBe(history?.project);
    expect(canRerun(history, undefined, other.project)).toBe(false);

    const options = { build: false, reopenEditor: false, runTests: false };
    const id = await backend.apply([{ kind: "install", package: "UBotMCP", version: null }], options, null, null);
    expect((await backend.listTasks()).find((x) => x.id === id)).toMatchObject({ project: "MyRobotSim", projectPath: path });
    expect((await backend.listTasks()).find((x) => x.id === "task-0")?.projectPath).toBe(SCENARIO);
    await backend.cancelTask(id);
    await untilIdle(backend);
  });

  it("reruns history in the same project with different path case and separators", async () => {
    const { backend } = createMockBackend(params);
    const history = (await backend.listTasks()).find((x) => x.id === "task-0");
    const same = await backend.openProject("d:/projects/myrobotsim/myrobotsim.uproject");
    expect(canRerun(history, undefined, same.project)).toBe(true);
  });
});

describe("mock backend prebuilt binaries", () => {
  const options = { build: true, reopenEditor: false, runTests: false };
  const pkg = (s: Snapshot, name: string) => s.packages.find((p) => p.name === name);

  it("reports how each package was installed and whether it can come prebuilt", async () => {
    const { backend } = createMockBackend(params);
    const s = await backend.getSnapshot();
    for (const name of ["UBotCore", "UBotSensor", "UBotROS"]) {
      expect(pkg(s, name)).toMatchObject({ installKind: "git", prebuiltAvailable: false, git: { isCheckout: true } });
    }
    expect(pkg(s, "UBotCombat")).toMatchObject({
      installKind: "prebuilt", prebuilt: { engine: "5.5", platform: "Win64" }, git: { isCheckout: false, ref: null }
    });
    expect(pkg(s, "UBotCore")?.prebuilt).toBeNull();
    expect(pkg(s, "UBotVehicles")).toMatchObject({ installKind: null, git: null, prebuiltAvailable: true });
    expect(pkg(s, "UBotMCP")).toMatchObject({ installKind: null, prebuiltAvailable: false });
    expect((await backend.getSettings()).install.usePrebuilt).toBe(true);
  });

  it("downloads a package that has binaries for the engine, and builds nothing", async () => {
    const { backend } = createMockBackend(params);
    const plan = await backend.plan([{ kind: "install", package: "UBotVehicles", version: null }], options);
    expect(plan.errors).toEqual([]);
    expect(plan.steps.map((s) => s.kind)).toEqual(["download", "enableEnginePlugin", "enablePackage"]);
    expect(plan.needsBuild).toBe(false);
    const download = plan.steps[0];
    expect(download).toMatchObject({ group: "download", needsEditorClosed: false, title: { code: "step.download", params: { package: "UBotVehicles", version: "0.1.0" } } });
    expect(download?.detail.params).toEqual({ source: "github.com/…/UBotVehicles-0.1.0-UE5.5-Win64.zip", dest: "Plugins/uBot/uBotVehicles" });
  });

  it("shows the source of a download like the core: host and file name, never credentials or the query", () => {
    expect(downloadSource("https://github.com/AstreoX/uBotSensor/releases/download/v0.1.0/UBotSensor-0.1.0-UE5.5-Win64.zip"))
      .toBe("github.com/…/UBotSensor-0.1.0-UE5.5-Win64.zip");
    expect(downloadSource("https://user:SECRET@h.example/x.zip?token=SECRET#f")).toBe("h.example/x.zip");
    expect(downloadSource("https://h.example")).toBe("h.example");
    expect(downloadSource("https://h.example/")).toBe("h.example");
    expect(downloadSource("HTTPS://h.example/a/b.zip")).toBe("h.example/…/b.zip");
    expect(downloadSource("http://127.0.0.1:8080/a/b.zip")).toBe("127.0.0.1:8080/…/b.zip");
    expect(downloadSource("file:///C:/mirror/x.zip")).toBe("C:/mirror/x.zip");
    expect(downloadSource("file://server/share/x.zip")).toBe("//server/share/x.zip");
    expect(downloadSource("D:\\mirror\\x.zip")).toBe("D:\\mirror\\x.zip");
    expect(downloadSource("https://仓/😀.zip")).toBe("仓/😀.zip");
  });

  it("clones and builds a package without binaries, with a warning", async () => {
    const { backend } = createMockBackend(params);
    const plan = await backend.plan([{ kind: "install", package: "UBotMCP", version: null }], options);
    expect(plan.warnings).toContainEqual({ code: "plan.prebuiltUnavailable", params: { package: "UBotMCP", version: "0.1.0", engine: "5.5" } });
    expect(plan.steps.map((s) => s.kind)).toEqual(["clone", "enablePackage", "build"]);
  });

  it("clones and builds everything when prebuilt binaries are turned off", async () => {
    const { backend } = createMockBackend({ ...params, prebuilt: false });
    const plan = await backend.plan([{ kind: "install", package: "UBotVehicles", version: null }], options);
    expect(plan.warnings).toEqual([]);
    expect(plan.steps.map((s) => s.kind)).toEqual(["clone", "enableEnginePlugin", "enablePackage", "build"]);
  });

  it("raises an engine mismatch for an enabled prebuilt package, which a reinstall fixes when binaries exist", async () => {
    const { backend } = createMockBackend(params);
    await backend.closeEditor();
    // Built for 5.5; the project moves to 5.4, for which no binaries exist.
    const settings = await backend.getSettings();
    await backend.saveSettings({ ...settings, build: { ...settings.build, engineId: "5.4" } });
    const id = await backend.apply([{ kind: "enable", package: "UBotCombat", version: null }], { ...options, build: false }, null, null);
    expect(id).toBeTruthy();
    await untilIdle(backend);
    const s = await backend.getSnapshot();
    const problem = s.problems.find((p) => p.message.code === "problem.prebuiltEngineMismatch");
    expect(problem?.message.params).toEqual({ package: "UBotCombat", built: "5.5", engine: "5.4" });
    expect(pkg(s, "UBotCombat")?.problemIds).toContain(problem?.id);
    const reinstall = problem?.fixes[0];
    expect(reinstall).toMatchObject({ primary: true, label: { code: "fix.reinstall", params: { package: "UBotCombat" } } });
    expect(problem?.fixes[1]?.label.code).toBe("fix.disable");
    const changes = reinstall?.action.kind === "changes" ? reinstall.action.changes : [];
    expect(changes).toEqual([{ kind: "update", package: "UBotCombat", version: "0.1.0" }]);
    const refused = await backend.plan(changes, options);
    expect(refused.errors).toEqual([{ code: "plan.noPrebuilt", params: { package: "UBotCombat", version: "0.1.0", engine: "5.4" } }]);
    // Back on 5.5 the same version is downloaded again, replacing the folder (so the editor must be closed).
    await backend.saveSettings({ ...settings, build: { ...settings.build, engineId: "5.5" } });
    const again = await backend.plan(changes, options);
    expect(again.errors).toEqual([]);
    expect(again.steps.map((s) => [s.kind, s.needsEditorClosed])).toEqual([["download", true]]);
  }, 20_000);

  it("streams a download with byte progress and installs the package prebuilt", async () => {
    const { backend } = createMockBackend(params);
    await backend.closeEditor();
    const progress: number[] = [];
    backend.onTask((t) => {
      const step = t.steps[0];
      if (step?.status === "running" && step.progress) progress.push(step.progress.done);
    });
    await backend.apply([{ kind: "install", package: "UBotVehicles", version: null }], { ...options, build: false }, null, null);
    await untilIdle(backend, 400);
    const task = (await backend.listTasks())[0];
    expect(task?.status).toBe("done");
    expect(task?.steps[0]?.progress?.total).toBe(47_815_066);
    expect(progress.length).toBeGreaterThan(2);
    expect(progress[progress.length - 1]).toBe(47_815_066);
    const log = await backend.getTaskLog(task?.id ?? "", "s1");
    expect(log[0]).toBe("GET https://github.com/AstreoX/uBotVehicles/releases/download/v0.1.0/UBotVehicles-0.1.0-UE5.5-Win64.zip");
    expect(log.some((line) => line.includes(".ubot-install.json"))).toBe(true);
    expect(pkg(await backend.getSnapshot(), "UBotVehicles")).toMatchObject({ state: "enabled", installKind: "prebuilt" });
  }, 20_000);
});

describe("mock index", () => {
  it("has the recipes and Packages of the index the manager embeds (uBotCore's Index/index.json)", () => {
    const file = fileURLToPath(new URL("../../../crates/ubot-core/assets/index.json", import.meta.url));
    const embedded = JSON.parse(readFileSync(file, "utf8").replace(/^\uFEFF/, "")) as typeof mockIndex;
    expect(mockIndex.recipes).toEqual(embedded.recipes);
    expect(mockIndex.packages.map((p) => p.name)).toEqual(embedded.packages.map((p) => p.name));
  });

  it("shows the recipes by their Title Case names", async () => {
    const { backend } = createMockBackend(params);
    const names = (await backend.getSnapshot()).recipes.map((r) => localized(r.name));
    for (const name of ["Sensors Only", "ROS 2 Sensors", "ROS Ground Robot", "LLM Drone", "Smoke Combat Demo"]) {
      expect(names).toContain(name);
    }
  });
});

/** Waits until the task that was canceled has ended, so the mock accepts the next project or task. */
async function untilIdle(backend: Backend, tries = 100): Promise<void> {
  for (let i = 0; i < tries; i++) {
    const tasks = await backend.listTasks();
    if (!tasks.some((x) => x.status === "running" || x.status === "waiting" || x.status === "pending")) return;
    await new Promise((resolve) => setTimeout(resolve, 50));
  }
  throw new Error("The task did not end.");
}
