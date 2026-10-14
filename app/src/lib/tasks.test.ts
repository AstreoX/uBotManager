import { afterEach, describe, expect, it } from "vitest";
import type { Change, StepStatus, Task } from "../api/types";
import { setLanguage } from "../i18n";
import {
  MAX_LOG_LINES, activeTask, appendLog, canRerun, canSwitchProject, capLines, defaultSelection, isDownloadStep, isTaskOfProject,
  mergeLog, mergeTasks, stoppedStep, taskProgress
} from "./tasks";

afterEach(() => setLanguage("en"));

/** The core stores both the project's name and its .uproject path. */
function task(id: string, status: StepStatus, steps: StepStatus[], startedAt = "2026-10-10T08:00:00Z", error: string | null = null): Task {
  return {
    id, title: { code: "task.apply", params: { project: "P" } }, project: "P", projectPath: "E:\\Projects\\P\\P.uproject",
    status, startedAt, durationMs: null,
    steps: steps.map((s, i) => ({
      id: `s${i + 1}`, title: { code: "step.build", params: { target: "T" } }, status: s, startedAt: null, durationMs: null,
      progress: null, logLines: 0
    })),
    error, changes: []
  };
}

describe("defaultSelection", () => {
  it("is nothing without tasks", () => {
    expect(defaultSelection([], null)).toBeNull();
    expect(defaultSelection([], "task-1")).toBeNull();
  });

  it("follows the running step of the active task", () => {
    const tasks = [task("b", "running", ["done", "running", "pending"]), task("a", "done", ["done"])];
    expect(defaultSelection(tasks, null)).toEqual({ taskId: "b", stepId: "s2" });
  });

  it("takes the focused task, and falls back to the newest one while it is not in the list yet", () => {
    const old = task("a", "done", ["done", "done"]);
    const fresh = task("b", "running", ["running", "pending"]);
    expect(defaultSelection([fresh, old], "a")).toEqual({ taskId: "a", stepId: null });
    // Rerun from the Tasks view: the focused id is not known yet, so the previous task is selected ...
    expect(defaultSelection([old], "b")).toEqual({ taskId: "a", stepId: null });
    // ... and once the task arrives the selection is made again and lands on its running step.
    expect(defaultSelection([fresh, old], "b")).toEqual({ taskId: "b", stepId: "s1" });
  });

  it("selects the step a failed task stopped on, so the failure is explained", () => {
    const failed = task("a", "failed", ["done", "failed", "skipped"], undefined, "Unreal Editor is running this project.");
    expect(stoppedStep(failed)?.id).toBe("s2");
    expect(defaultSelection([failed], null)).toEqual({ taskId: "a", stepId: "s2" });
    const canceled = task("c", "canceled", ["done", "canceled", "skipped"]);
    expect(defaultSelection([canceled], null)).toEqual({ taskId: "c", stepId: "s2" });
  });
});

describe("activeTask", () => {
  it("is the task that has not finished", () => {
    expect(activeTask([task("a", "done", ["done"])])).toBeUndefined();
    expect(activeTask([task("b", "waiting", ["waiting"]), task("a", "done", ["done"])])?.id).toBe("b");
  });
});

describe("canSwitchProject", () => {
  it("is false while a task runs, waits for the editor or is about to start", () => {
    expect(canSwitchProject([])).toBe(true);
    expect(canSwitchProject([task("a", "done", ["done"]), task("b", "failed", ["failed"])])).toBe(true);
    expect(canSwitchProject([task("a", "running", ["running"])])).toBe(false);
    expect(canSwitchProject([task("a", "waiting", ["waiting"])])).toBe(false);
    expect(canSwitchProject([task("a", "pending", ["pending"])])).toBe(false);
  });
});

describe("canRerun", () => {
  const changes: Change[] = [{ kind: "install", package: "UBotSensor", version: "0.1.0" }];
  const finished = { ...task("a", "done", ["done"]), changes };
  const open = { name: "P" };

  it("is offered for a finished task of the open project", () => {
    expect(canRerun(finished, undefined, open)).toBe(true);
    expect(canRerun({ ...finished, status: "failed" }, undefined, open)).toBe(true);
    expect(canRerun({ ...finished, status: "canceled" }, undefined, open)).toBe(true);
  });

  it("is not offered for a task of another project, whose changes it would apply to the open one", () => {
    expect(canRerun({ ...finished, project: "Other" }, undefined, open)).toBe(false);
    expect(canRerun(finished, undefined, { name: "Other" })).toBe(false);
    expect(isTaskOfProject(finished, { name: "Other" })).toBe(false);
  });

  it("compares project names like package names, case-insensitively", () => {
    expect(isTaskOfProject({ ...finished, project: "myrobotsim" }, { name: "MyRobotSim" })).toBe(true);
    expect(canRerun({ ...finished, project: "myrobotsim" }, undefined, { name: "MyRobotSim" })).toBe(true);
  });

  it("rejects same-named projects in different directories", () => {
    const other = { name: "P", path: "E:\\Other\\P\\P.uproject" };
    expect(isTaskOfProject(finished, other)).toBe(false);
    expect(canRerun(finished, undefined, other)).toBe(false);
  });

  it.each([
    "e:\\projects\\p\\p.uproject",
    "E:/Projects/P/P.uproject",
    "e:/PROJECTS/P/P.UPROJECT///",
    "E:\\Projects\\P\\P.uproject\\\\"
  ])("matches project paths regardless of case, separators or trailing slashes: %s", (path) => {
    const openProject = { name: "P", path };
    expect(isTaskOfProject(finished, openProject)).toBe(true);
    expect(canRerun(finished, undefined, openProject)).toBe(true);
  });

  it("uses path identity even when project names differ", () => {
    const openProject = { name: "Other", path: finished.projectPath };
    expect(isTaskOfProject(finished, openProject)).toBe(true);
    expect(canRerun(finished, undefined, openProject)).toBe(true);
  });

  it("falls back to case-insensitive names for old history without a path", () => {
    const legacy = { ...finished, project: "p", projectPath: "" };
    const sameName = { name: "P", path: "E:/Other/P/P.uproject" };
    const otherName = { name: "Other", path: finished.projectPath };
    expect(isTaskOfProject(legacy, sameName)).toBe(true);
    expect(canRerun(legacy, undefined, sameName)).toBe(true);
    expect(isTaskOfProject(legacy, otherName)).toBe(false);
    expect(canRerun(legacy, undefined, otherName)).toBe(false);
    expect(canRerun(legacy, undefined, null)).toBe(false);
  });

  it.each([undefined, ""])("falls back to names when the caller has no project path: %s", (path) => {
    expect(canRerun(finished, undefined, { name: "p", path })).toBe(true);
    expect(canRerun(finished, undefined, { name: "Other", path })).toBe(false);
  });

  it("is not offered without an open project, without a task or without changes", () => {
    expect(canRerun(finished, undefined, null)).toBe(false);
    expect(isTaskOfProject(finished, null)).toBe(false);
    expect(canRerun(undefined, undefined, open)).toBe(false);
    expect(canRerun({ ...finished, changes: [] }, undefined, open)).toBe(false);
  });

  it("is not offered for an unfinished task or while another task runs", () => {
    expect(canRerun({ ...finished, status: "running" }, undefined, open)).toBe(false);
    expect(canRerun(finished, task("b", "running", ["running"]), open)).toBe(false);
  });
});

describe("mergeTasks", () => {
  it("adds the tasks the list does not have, newest first", () => {
    const current = [task("b", "running", ["running"], "2026-10-10T08:05:00Z")];
    const listed = [task("c", "running", ["running"], "2026-10-10T08:06:00Z"), task("a", "done", ["done"], "2026-10-09T08:00:00Z")];
    expect(mergeTasks(current, listed).map((x) => x.id)).toEqual(["c", "b", "a"]);
  });

  it("keeps the tasks it has: they come from events, which are not older than the answer", () => {
    const newer = task("a", "done", ["done"]);
    const older = task("a", "running", ["running"]);
    const merged = mergeTasks([newer], [older]);
    expect(merged).toHaveLength(1);
    expect(merged[0]).toBe(newer);
  });

  it("returns the same list when there is nothing to add", () => {
    const current = [task("a", "done", ["done"])];
    expect(mergeTasks(current, [])).toBe(current);
  });
});

describe("task log", () => {
  const lines = (n: number, from = 0) => Array.from({ length: n }, (_, i) => `line ${from + i}`);

  it("keeps the newest lines, as many as the core keeps", () => {
    expect(MAX_LOG_LINES).toBe(5000);
    expect(capLines(lines(10))).toHaveLength(10);
    const capped = capLines(lines(MAX_LOG_LINES + 3));
    expect(capped).toHaveLength(MAX_LOG_LINES);
    expect(capped[0]).toBe("line 3");
    expect(capped[capped.length - 1]).toBe(`line ${MAX_LOG_LINES + 2}`);
  });

  it("caps a streamed log batch by batch", () => {
    let log: string[] = [];
    for (let i = 0; i < 70; i++) log = appendLog(log, lines(100, i * 100));
    expect(log).toHaveLength(MAX_LOG_LINES);
    expect(log[log.length - 1]).toBe("line 6999");
    expect(log[0]).toBe("line 2000");
  });

  it("merges what streamed while the log loaded, without doubling lines", () => {
    expect(mergeLog(["a", "b"], ["b", "c"])).toEqual(["a", "b", "c"]);
    expect(mergeLog(["a", "b"], ["c"])).toEqual(["a", "b", "c"]);
    expect(mergeLog(["a", "b"], ["a", "b"])).toEqual(["a", "b"]);
    expect(mergeLog([], ["x"])).toEqual(["x"]);
    expect(mergeLog(lines(MAX_LOG_LINES), lines(5, MAX_LOG_LINES))).toHaveLength(MAX_LOG_LINES);
  });
});

describe("taskProgress", () => {
  const MB = 1024 * 1024;
  const download = (status: StepStatus, progress: { done: number; total: number } | null): Task => {
    const t = task("d", "running", [status, "pending", "pending"]);
    const first = t.steps[0];
    if (first) {
      first.title = { code: "step.download", params: { package: "UBotVehicles", version: "0.1.0" } };
      first.progress = progress;
    }
    return t;
  };

  it("counts the bytes of a download, in MB with one decimal", () => {
    const p = taskProgress(download("running", { done: Math.round(12.3 * MB), total: Math.round(45.6 * MB) }));
    expect(p.text).toBe("Downloading UBotVehicles 0.1.0");
    expect(p.count).toBe("12.3 / 45.6 MB");
    expect(p.done / p.total).toBeCloseTo(12.3 / 45.6, 3);
    expect(p.indeterminate).toBe(false);
    setLanguage("zh-CN");
    const zh = taskProgress(download("running", { done: 0, total: 47_815_066 }));
    expect(zh.text).toBe("正在下载 UBotVehicles 0.1.0");
    expect(zh.count).toBe("0.0 / 45.6 MB");
  });

  it("runs the bar without a fill while the size of a download is unknown", () => {
    const p = taskProgress(download("running", { done: 5 * MB, total: 0 }));
    expect(p.count).toBe("5.0 MB");
    expect(p.indeterminate).toBe(true);
  });

  it("counts build actions and steps like before", () => {
    const build = task("b", "running", ["done", "running"]);
    const step = build.steps[1];
    if (step) step.progress = { done: 15, total: 24 };
    expect(taskProgress(build)).toMatchObject({ text: "Building T", done: 15, total: 24, count: "15/24", indeterminate: false });
    // A download that has not reported bytes yet counts steps.
    expect(taskProgress(download("running", null))).toMatchObject({ done: 1, total: 3, count: "1/3" });
    expect(taskProgress(download("waiting", null)).text).toBe("Waiting for Unreal Editor to close");
  });

  it("knows download steps by their title", () => {
    expect(isDownloadStep({ title: { code: "step.download", params: {} } })).toBe(true);
    expect(isDownloadStep({ title: { code: "step.clone", params: {} } })).toBe(false);
  });
});
