import { useEffect, useState, type ReactNode } from "react";
import { platform } from "../api";
import type { IndexSourceStatus, Task } from "../api/types";
import { hasKey, t, tm, tn } from "../i18n";
import { changeText, problemCounts, sameName } from "../lib/format";
import { activeTask, canSwitchProject } from "../lib/tasks";
import { packageProblems, useApp } from "../state";
import {
  CloseGlyph, CloseIcon, ErrorIcon, GearIcon, GraphIcon, MaximizeGlyph, MinimizeGlyph, PackageIcon, RecipeIcon,
  RefreshIcon, RestoreGlyph, SpinnerIcon, TasksIcon, WarningIcon, ArrowUpIcon
} from "./Icons";
import type { MenuEntry } from "./Menu";

export function sourceName(source: { id: string; name: string; builtin: boolean }): string {
  return source.builtin || source.id === "embedded" ? t(`source.${source.id}`) : source.name;
}

function indexTooltip(sources: IndexSourceStatus[], syncing: boolean): string {
  const lines = [syncing ? t("header.syncing") : t("header.refresh")];
  for (const s of sources) {
    if (!s.enabled) continue;
    const name = sourceName(s);
    // A source served from its cache after a failed fetch reports ok=false with fromCache=true.
    if (s.fromCache) lines.push(tn("header.indexCached", s.packages, { name }));
    else if (s.ok) lines.push(tn("header.indexOk", s.packages, { name }));
    else lines.push(t("header.indexFailed", { name, error: s.error ?? "" }));
  }
  return lines.join("\n");
}

export function TitleBar() {
  const { state, actions } = useApp();
  const [maximized, setMaximized] = useState(false);
  useEffect(() => platform.onMaximizedChange(setMaximized), []);
  const spinning = state.refreshing || state.snapshot.syncing;
  return (
    <header className="titlebar" data-tauri-drag-region="">
      <span className="logo" data-tauri-drag-region="">uB</span>
      <span className="app-name" data-tauri-drag-region="">{t("app.name")}</span>
      <span className="spacer" data-tauri-drag-region="" />
      <button type="button" className="header-button" onClick={actions.openRecipes}>
        <RecipeIcon />
        {t("header.recipes")}
      </button>
      <button
        type="button"
        className="icon-button"
        aria-label={t("header.refresh")}
        title={indexTooltip(state.snapshot.index, spinning)}
        onClick={actions.refresh}
        disabled={spinning}
      >
        <RefreshIcon className={spinning ? "spin" : undefined} />
      </button>
      <div className="window-controls">
        <button type="button" className="window-button" aria-label={t("header.minimize")} title={t("header.minimize")} onClick={platform.minimizeWindow}>
          <MinimizeGlyph />
        </button>
        <button
          type="button"
          className="window-button"
          aria-label={t(maximized ? "header.restore" : "header.maximize")}
          title={t(maximized ? "header.restore" : "header.maximize")}
          onClick={platform.toggleMaximizeWindow}
        >
          {maximized ? <RestoreGlyph /> : <MaximizeGlyph />}
        </button>
        <button type="button" className="window-button close" aria-label={t("header.close")} title={t("header.close")} onClick={platform.closeWindow}>
          <CloseGlyph />
        </button>
      </div>
    </header>
  );
}

export function ToolStrip() {
  const { state, actions } = useApp();
  const item = (view: "packages" | "graph" | "problems", icon: ReactNode) => (
    <button
      type="button"
      className={`strip-button${state.view === view ? " active" : ""}`}
      aria-label={t(`view.${view}`)}
      aria-current={state.view === view ? "page" : undefined}
      title={t(`view.${view}`)}
      onClick={() => actions.setView(view)}
    >
      {icon}
    </button>
  );
  return (
    <nav className="strip" aria-label={t("view.toolWindows")}>
      {item("packages", <PackageIcon size={18} />)}
      {item("graph", <GraphIcon size={18} />)}
      <span className="spacer" />
      {item("problems", <WarningIcon size={18} />)}
      <button type="button" className="strip-button" aria-label={t("view.settings")} title={t("view.settings")} onClick={() => actions.openSettings()}>
        <GearIcon size={18} />
      </button>
    </nav>
  );
}

function progressOf(task: Task): { text: string; done: number; total: number } {
  const index = task.steps.findIndex((s) => s.status === "running" || s.status === "waiting");
  const step = index >= 0 ? task.steps[index] : undefined;
  if (!step) return { text: tm(task.title), done: 0, total: task.steps.length };
  let text: string;
  if (step.status === "waiting") text = t("progress.waiting");
  else if (hasKey(`progress.${step.title.code}`)) text = t(`progress.${step.title.code}`, step.title.params);
  else text = tm(step.title);
  if (step.progress) return { text, done: step.progress.done, total: step.progress.total };
  return { text, done: index + 1, total: task.steps.length };
}

export function StatusBar() {
  const { state, actions } = useApp();
  const { snapshot, view, pending, tasks } = state;
  const crumb = `${t("app.name")} › ${t(`view.${view}`)}`;
  const running = activeTask(tasks);

  if (view === "tasks" && running) {
    const p = progressOf(running);
    const ratio = p.total > 0 ? Math.min(1, p.done / p.total) : 0;
    return (
      <footer className="statusbar">
        <span>{crumb}</span>
        <span className="spacer" />
        <span className="sb-progress-text">{p.text}</span>
        <span className="progress" role="progressbar" aria-valuemin={0} aria-valuemax={p.total} aria-valuenow={p.done}>
          <span className="fill" style={{ width: `${ratio * 100}%` }} />
        </span>
        <span className="mono" style={{ fontSize: 12 }}>{p.done}/{p.total}</span>
        <button type="button" className="sb-cancel" aria-label={t("status.cancelTask")} title={t("status.cancelTask")} onClick={() => actions.cancelTask(running.id)}>
          <CloseIcon size={10} strokeWidth={1.8} />
        </button>
      </footer>
    );
  }

  const problems = packageProblems(snapshot);
  const { errors, warnings } = problemCounts(problems);
  const updates = snapshot.packages.filter((p) => p.updateAvailable);
  const pkgOf = (name: string) => snapshot.packages.find((p) => sameName(p.name, name));
  const first = pending[0];
  const pendingText = pending.length === 1 && first
    ? t("status.pending.one", { change: changeText(first, pkgOf(first.package)) })
    : t("status.pending.other", { n: pending.length });

  return (
    <footer className="statusbar">
      <span>{crumb}</span>
      <span className="spacer" />
      {errors > 0 && (
        <button type="button" className="sb-button error" title={tn("status.errors", errors)} onClick={() => actions.setView("problems")}>
          <ErrorIcon size={13} strokeWidth={1.4} />{errors}
        </button>
      )}
      {warnings > 0 && (
        <button type="button" className="sb-button warning" title={tn("status.warnings", warnings)} onClick={() => actions.setView("problems")}>
          <WarningIcon size={13} strokeWidth={1.4} />{warnings}
        </button>
      )}
      {updates.length > 0 && (
        <button
          type="button"
          className="sb-button update"
          title={tn("status.updates", updates.length)}
          onClick={() => {
            const firstUpdate = updates[0];
            if (firstUpdate) actions.select(firstUpdate.name);
            actions.setView("packages");
          }}
        >
          <ArrowUpIcon size={13} />{updates.length}
        </button>
      )}
      {pending.length > 0 && (
        <span className="sb-pending" title={pending.map((c) => changeText(c, pkgOf(c.package))).join("\n")}>
          <span className="text">{pendingText}</span>
          <button type="button" className="link-button" disabled={state.busy} onClick={actions.reviewPending}>{t("status.apply")}</button>
          <button type="button" className="link-button" onClick={actions.discard}>{t("status.discard")}</button>
        </span>
      )}
      <button type="button" className="sb-button" onClick={() => actions.setView("tasks")}>
        {running ? <SpinnerIcon size={13} className="spin" /> : <TasksIcon size={13} />}
        {t("view.tasks")}
      </button>
    </footer>
  );
}

export function useProjectMenu(): MenuEntry[] {
  const { state, actions } = useApp();
  const project = state.snapshot.project;
  const recent = state.settings.recentProjects;
  // The core refuses to open another project while a task runs.
  const busy = !canSwitchProject(state.tasks);
  const title = busy ? tm({ code: "task.busy", params: {} }) : undefined;
  const entries: MenuEntry[] = [{ label: t("menu.openProject"), disabled: busy, title, onSelect: () => void actions.openProject() }];
  if (recent.length > 0) {
    entries.push({ kind: "separator" }, { kind: "section", label: t("menu.recent") });
    for (const path of recent) {
      const name = path.split(/[\\/]/).pop()?.replace(/\.uproject$/i, "") ?? path;
      entries.push({ label: name, hint: path, disabled: busy, title, onSelect: () => void actions.openProject(path) });
    }
  }
  entries.push(
    { kind: "separator" },
    { label: t("menu.showInExplorer"), disabled: !project, onSelect: () => project && platform.revealPath(project.path) },
    { label: t("menu.openInEditor"), disabled: !project, onSelect: () => project && platform.openPath(project.path) }
  );
  return entries;
}

export function Notices() {
  const { state, actions } = useApp();
  const count = state.notices.length;
  useEffect(() => {
    if (count === 0) return;
    const timer = setTimeout(() => actions.dismissNotice(0), 10000);
    return () => clearTimeout(timer);
  }, [count, actions]);
  if (count === 0) return null;
  return (
    <div className="notices" role="status">
      {state.notices.map((text, i) => (
        <div key={`${i}-${text}`} className="notice">
          <ErrorIcon size={14} />
          <span className="text">{text}</span>
          <button type="button" className="sb-cancel" aria-label={t("common.dismiss")} onClick={() => actions.dismissNotice(i)}>
            <CloseIcon size={10} strokeWidth={1.8} />
          </button>
        </div>
      ))}
    </div>
  );
}
