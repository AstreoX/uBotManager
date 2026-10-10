import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { api } from "../api";
import type { StepStatus, Task, TaskStep } from "../api/types";
import { useProjectMenu } from "../components/Chrome";
import { CheckIcon, ClockIcon, CloseIcon, PendingIcon, RerunIcon, SkipIcon, SpinnerIcon, StopIcon } from "../components/Icons";
import { OptionsMenu } from "../components/Menu";
import { t, tm, tn } from "../i18n";
import { errorText } from "../lib/errors";
import { formatClock, formatDuration } from "../lib/format";
import {
  activeTask, appendLog, canRerun, defaultSelection, isFinished, isTaskOfProject, mergeLog, runningStep, stoppedStep,
  type Selection
} from "../lib/tasks";
import { useApp } from "../state";

const appStartedAt = Date.now();

function StatusIcon({ status, old }: { status: StepStatus; old?: boolean }) {
  switch (status) {
    case "done": return <CheckIcon size={14} className={old ? "c-muted" : "c-success"} />;
    case "running": return <SpinnerIcon size={14} className="c-info spin" />;
    case "waiting": return <ClockIcon size={14} className="c-warning" />;
    case "failed": return <CloseIcon size={14} strokeWidth={1.6} className="c-error" />;
    case "canceled": return <CloseIcon size={14} strokeWidth={1.6} className="c-muted" />;
    case "skipped": return <SkipIcon size={14} className="c-muted" />;
    case "pending": return <PendingIcon size={14} className="c-muted" />;
  }
}

export function TasksView() {
  const { state, actions } = useApp();
  const menu = useProjectMenu();
  const { tasks } = state;
  const [selection, setSelection] = useState<Selection | null>(() => defaultSelection(tasks, state.focusTaskId));
  const [follow, setFollow] = useState(true);
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const [now, setNow] = useState(Date.now());
  const running = activeTask(tasks);

  // A newly started task takes the selection and the log follows its running step. The task may reach the
  // list after its id was focused (the event races the apply answer), so the selection is made again once
  // the focused task is there.
  const focusLoaded = state.focusTaskId !== null && tasks.some((x) => x.id === state.focusTaskId);
  useEffect(() => {
    if (!state.focusTaskId) return;
    setFollow(true);
    setSelection(defaultSelection(tasks, state.focusTaskId));
  }, [state.focusTaskId, focusLoaded]);

  const followedTask = follow ? tasks.find((x) => x.id === selection?.taskId) : undefined;
  // While it runs the log follows the running step; when it stops, the step it stopped on, which explains why.
  const followedStep = followedTask
    ? (followedTask === running ? runningStep(followedTask) : stoppedStep(followedTask))?.id ?? null
    : null;
  useEffect(() => {
    if (followedStep && selection && selection.stepId !== followedStep) setSelection({ taskId: selection.taskId, stepId: followedStep });
  }, [followedStep, selection]);

  useEffect(() => {
    if (!running) return;
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, [running]);

  const selectedTask = tasks.find((x) => x.id === selection?.taskId) ?? tasks[0];
  const newest = tasks[0];

  const rows = useMemo(() => {
    const list: { key: string; task: Task; step: TaskStep | null; indent: number }[] = [];
    for (const task of tasks) {
      list.push({ key: task.id, task, step: null, indent: 0 });
      if (task === newest || expanded.has(task.id) || task.status === "running" || task.status === "waiting") {
        for (const step of task.steps) list.push({ key: `${task.id}/${step.id}`, task, step, indent: 1 });
      }
    }
    return list;
  }, [tasks, newest, expanded]);

  const isOld = (task: Task) => isFinished(task.status) && Date.parse(task.startedAt) < appStartedAt;

  const timeOf = (task: Task, step: TaskStep | null): string => {
    if (step) {
      if (step.status === "running" && step.startedAt) return formatDuration(now - Date.parse(step.startedAt));
      return step.durationMs !== null ? formatDuration(step.durationMs) : "";
    }
    if (isOld(task)) return t("tasks.earlier");
    if (!isFinished(task.status)) return formatDuration(now - Date.parse(task.startedAt));
    return task.durationMs !== null ? formatDuration(task.durationMs) : "";
  };

  // Rerun plans the task's changes against the open project, so it is offered for that project's tasks only.
  const project = state.snapshot.project;
  const rerunnable = canRerun(selectedTask, running, project);
  const otherProject = !!selectedTask && !isTaskOfProject(selectedTask, project);
  const rerunTitle = otherProject && selectedTask ? t("tasks.rerunOtherProject", { project: selectedTask.project }) : t("tasks.rerun");

  return (
    <main className="tool-window">
      <div className="tw-header" style={{ gap: 4 }}>
        <span className="tw-title" style={{ marginRight: 8 }}>{t("view.tasks")}</span>
        <button
          type="button"
          className="tool-icon-button stop"
          aria-label={t("tasks.stop")}
          title={t("tasks.stop")}
          disabled={!running}
          onClick={() => running && actions.cancelTask(running.id)}
        >
          <StopIcon size={14} />
        </button>
        <button
          type="button"
          className="tool-icon-button"
          aria-label={t("tasks.rerun")}
          title={rerunTitle}
          disabled={!rerunnable}
          onClick={() => selectedTask && actions.rerun(selectedTask)}
        >
          <RerunIcon size={14} />
        </button>
        <span className="spacer" />
        <OptionsMenu entries={menu} />
      </div>
      <div className="split">
        <section className="task-list" aria-label={t("tasks.steps")}>
          {tasks.length === 0 && <div className="list-placeholder">{t("tasks.none")}</div>}
          {rows.map(({ key, task, step, indent }) => {
            const selected = selection?.taskId === task.id && (selection.stepId ?? null) === (step?.id ?? null);
            const old = !step && isOld(task);
            return (
              <button
                key={key}
                type="button"
                className={`task-row${selected ? " selected" : ""}${old ? " old" : ""}`}
                style={{ paddingLeft: 12 + indent * 20 }}
                onClick={() => {
                  setFollow(false);
                  setSelection({ taskId: task.id, stepId: step?.id ?? null });
                  if (!step) setExpanded((set) => new Set(set).add(task.id));
                }}
              >
                <StatusIcon status={step ? step.status : task.status} old={old} />
                <span className="title">{tm(step ? step.title : task.title)}</span>
                <span className="time">{timeOf(task, step)}</span>
              </button>
            );
          })}
        </section>
        <section className="task-log" aria-label={t("tasks.log")}>
          {selectedTask && <TaskLog task={selectedTask} stepId={selection?.taskId === selectedTask.id ? selection.stepId : null} />}
        </section>
      </div>
    </main>
  );
}

function statusText(status: StepStatus): string {
  return t(`stepStatus.${status}`);
}

function TaskLog({ task, stepId }: { task: Task; stepId: string | null }) {
  const [lines, setLines] = useState<string[]>([]);
  const [loaded, setLoaded] = useState(false);
  const preRef = useRef<HTMLPreElement>(null);
  const stick = useRef(true);
  const taskId = task.id;

  useEffect(() => {
    let disposed = false;
    setLines([]);
    setLoaded(false);
    stick.current = true;
    const buffered: string[] = [];
    let ready = false;
    const off = stepId === null ? () => undefined : api.onTaskLog((e) => {
      if (e.taskId !== taskId || e.stepId !== stepId) return;
      if (ready) setLines((current) => appendLog(current, e.lines));
      else buffered.push(...e.lines);
    });
    api.getTaskLog(taskId, stepId).then((initial) => {
      if (disposed) return;
      ready = true;
      setLines(mergeLog(initial, buffered));
      setLoaded(true);
    }, () => undefined);
    return () => {
      disposed = true;
      off();
    };
  }, [taskId, stepId]);

  useLayoutEffect(() => {
    const pre = preRef.current;
    if (pre && stick.current) pre.scrollTop = pre.scrollHeight;
  }, [lines]);

  const step = stepId === null ? undefined : task.steps.find((s) => s.id === stepId);
  const stepStatus = step?.status;
  const text = useMemo(() => {
    if (stepId !== null) {
      // A step that stopped before it wrote anything (the editor was running) has no log: say why.
      if (loaded && lines.length === 0 && task.error && (stepStatus === "failed" || stepStatus === "canceled")) {
        return errorText(task.error);
      }
      return lines.join("\n");
    }
    const summary = [
      `${tn("tasks.steps", task.steps.length)} · ${statusText(task.status)} · ${t("tasks.startedAt", { time: formatClock(task.startedAt) })}`,
      ...task.steps.map((s) => `${tm(s.title)} · ${statusText(s.status)}${s.durationMs !== null ? ` · ${formatDuration(s.durationMs)}` : ""}`)
    ];
    if (task.error) summary.push("", errorText(task.error));
    if (lines.length > 0) summary.push("", ...lines);
    return summary.join("\n");
  }, [stepId, stepStatus, loaded, lines, task]);

  return (
    <pre
      ref={preRef}
      onScroll={(e) => {
        const el = e.currentTarget;
        stick.current = el.scrollHeight - el.scrollTop - el.clientHeight < 24;
      }}
    >
      {text}
    </pre>
  );
}
