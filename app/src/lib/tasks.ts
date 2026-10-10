import type { StepStatus, Task, TaskStep } from "../api/types";
import { hasKey, t, tm } from "../i18n";
import { formatByteProgress, sameName } from "./format";

export interface Selection { taskId: string; stepId: string | null }

export function isFinished(status: StepStatus): boolean {
  return status === "done" || status === "failed" || status === "canceled" || status === "skipped";
}

/** The task that is running, waiting for the editor or about to start. */
export function activeTask(tasks: Task[]): Task | undefined {
  return tasks.find((x) => x.status === "running" || x.status === "waiting" || x.status === "pending");
}

/** The core refuses to open another project while a task runs: a task works on the project it was planned for. */
export function canSwitchProject(tasks: Task[]): boolean {
  return activeTask(tasks) === undefined;
}

/**
 * Whether a task ran in the open project. `Task.project` is the project's name (the .uproject file name without
 * its extension), and the task history holds the tasks of every project.
 */
export function isTaskOfProject(task: Task, project: { name: string } | null): boolean {
  return project !== null && sameName(task.project, project.name);
}

/**
 * Whether a task can be rerun. Rerun plans the changes of a finished task against the open project, so it is offered
 * for the tasks of that project only: for another one it would install, update or remove Packages in the wrong project.
 */
export function canRerun(task: Task | undefined, running: Task | undefined, project: { name: string } | null): boolean {
  return !!task && isFinished(task.status) && !running && task.changes.length > 0 && isTaskOfProject(task, project);
}

export function runningStep(task: Task): TaskStep | undefined {
  return task.steps.find((s) => s.status === "running" || s.status === "waiting");
}

/** The step a failed or canceled task stopped on. */
export function stoppedStep(task: Task): TaskStep | undefined {
  return task.steps.find((s) => s.status === "failed" || s.status === "canceled");
}

/** A step that downloads prebuilt binaries: its progress counts bytes, not build actions. */
export function isDownloadStep(step: Pick<TaskStep, "title">): boolean {
  return step.title.code === "step.download";
}

/** What the status bar shows while a task runs: the step, how far it is, and that as text. */
export interface TaskProgress {
  text: string;
  done: number;
  total: number;
  /** "15/24" (build actions or steps), "12.3 / 45.6 MB" (a download). */
  count: string;
  /** The total is not known yet (a download without a size): the bar runs without a fill. */
  indeterminate: boolean;
}

export function taskProgress(task: Task): TaskProgress {
  const index = task.steps.findIndex((s) => s.status === "running" || s.status === "waiting");
  const step = index >= 0 ? task.steps[index] : undefined;
  const steps = (done: number, text: string): TaskProgress =>
    ({ text, done, total: task.steps.length, count: `${done}/${task.steps.length}`, indeterminate: false });
  if (!step) return steps(0, tm(task.title));
  let text: string;
  if (step.status === "waiting") text = t("progress.waiting");
  else if (hasKey(`progress.${step.title.code}`)) text = t(`progress.${step.title.code}`, step.title.params);
  else text = tm(step.title);
  const progress = step.progress;
  if (!progress) return steps(index + 1, text);
  if (isDownloadStep(step)) {
    return { text, done: progress.done, total: progress.total, count: formatByteProgress(progress.done, progress.total), indeterminate: progress.total <= 0 };
  }
  return { text, done: progress.done, total: progress.total, count: `${progress.done}/${progress.total}`, indeterminate: false };
}

/**
 * What the Tasks view selects: the focused task (the one just applied), else the active or newest one; in it
 * the running step, or the step it stopped on so a failure is explained, or the task itself.
 */
export function defaultSelection(tasks: Task[], focus: string | null): Selection | null {
  const task = (focus ? tasks.find((x) => x.id === focus) : undefined) ?? activeTask(tasks) ?? tasks[0];
  if (!task) return null;
  return { taskId: task.id, stepId: runningStep(task)?.id ?? stoppedStep(task)?.id ?? null };
}

/**
 * Adds the tasks of a `list_tasks` answer that the list does not have yet. Tasks it already has stay as
 * they are: they come from `ubot://task` events, which are at least as new as the answer.
 */
export function mergeTasks(current: Task[], listed: Task[]): Task[] {
  const known = new Set(current.map((x) => x.id));
  const added = listed.filter((x) => !known.has(x.id));
  if (added.length === 0) return current;
  const started = (task: Task) => Date.parse(task.startedAt) || 0;
  return [...current, ...added].sort((a, b) => started(b) - started(a));
}

/** The core keeps this many log lines per step (ubot-core exec.rs MAX_STEP_LOG_LINES); the view shows no more. */
export const MAX_LOG_LINES = 5000;

/** The newest `MAX_LOG_LINES` lines. */
export function capLines(lines: string[]): string[] {
  return lines.length > MAX_LOG_LINES ? lines.slice(lines.length - MAX_LOG_LINES) : lines;
}

/** Appends a streamed batch to the live log. */
export function appendLog(current: string[], batch: string[]): string[] {
  return capLines(current.concat(batch));
}

/** Appends lines streamed while the initial log was loading, skipping the ones it already contains. */
export function mergeLog(initial: string[], streamed: string[]): string[] {
  for (let k = Math.min(initial.length, streamed.length); k > 0; k--) {
    const tail = initial.slice(initial.length - k);
    if (tail.every((line, i) => line === streamed[i])) return capLines([...initial, ...streamed.slice(k)]);
  }
  return capLines([...initial, ...streamed]);
}
