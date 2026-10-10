import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { afterEach, describe, expect, it } from "vitest";
import { localized, setLanguage } from "../i18n";
import { errorText } from "../lib/errors";
import { canRerun } from "../lib/tasks";
import { createMockBackend } from "./mock";
import mockIndex from "./mockIndex.json";
import type { MockParams } from "./mockParams";
import type { Backend } from "./backend";
import type { Snapshot } from "./types";

const params: MockParams = { lang: null, theme: null, view: null, dialog: null, section: null, select: null, noProject: false };
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

  it("records the project's name in a task, which is what Rerun compares with the open project", async () => {
    const { backend } = createMockBackend(params);
    const history = (await backend.listTasks()).find((x) => x.id === "task-0");
    expect(history?.project).toBe("MyRobotSim");
    const scenario = await backend.getSnapshot();
    expect(canRerun(history, undefined, scenario.project)).toBe(true);

    const options = { build: false, reopenEditor: false, runTests: false };
    const id = await backend.apply([{ kind: "install", package: "UBotMCP", version: null }], options, null, null);
    expect((await backend.listTasks()).find((x) => x.id === id)?.project).toBe("MyRobotSim");
    await backend.cancelTask(id);
    await untilIdle(backend);

    // Another project is open now: the history of the first one cannot be rerun in it.
    const other = await backend.openProject(WAREHOUSE);
    expect(other.project?.name).toBe("WarehouseBot");
    expect(canRerun(history, undefined, other.project)).toBe(false);
  });
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
async function untilIdle(backend: Backend): Promise<void> {
  for (let i = 0; i < 100; i++) {
    const tasks = await backend.listTasks();
    if (!tasks.some((x) => x.status === "running" || x.status === "waiting" || x.status === "pending")) return;
    await new Promise((resolve) => setTimeout(resolve, 50));
  }
  throw new Error("The task did not end.");
}
