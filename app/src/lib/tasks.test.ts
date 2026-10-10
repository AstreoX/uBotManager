import { describe, expect, it } from "vitest";
import type { Change, StepStatus, Task } from "../api/types";
import {
  MAX_LOG_LINES, activeTask, appendLog, canRerun, canSwitchProject, capLines, defaultSelection, isTaskOfProject, mergeLog,
  mergeTasks, stoppedStep
} from "./tasks";

/** `Task.project` is the project's name, like the core stores it. */
function task(id: string, status: StepStatus, steps: StepStatus[], startedAt = "2026-10-10T08:00:00Z", error: string | null = null): Task {
  return {
    id, title: { code: "task.apply", params: { project: "P" } }, project: "P", status, startedAt, durationMs: null,
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
