import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { api, initialChanges, mockParams, platform } from "./api";
import type { ViewId } from "./api/mockParams";
import type {
  ApplyOptions, Change, Fix, Msg, Plan, Problem, Settings, SettingsSection, Snapshot, Task
} from "./api/types";
import { t } from "./i18n";
import { errorText } from "./lib/errors";
import { bySeverity, findChange, sameName, sameProjectPath, upsertChange } from "./lib/format";
import { projectSwitch } from "./lib/project";
import { mergeTasks } from "./lib/tasks";

export type { ViewId };

export type DialogState =
  | { kind: "plan"; plan: Plan; options: ApplyOptions; title: Msg | null; fromPending: boolean }
  | { kind: "recipes" }
  | { kind: "settings"; section: SettingsSection };

export interface AppState {
  snapshot: Snapshot;
  settings: Settings;
  tasks: Task[];
  pending: Change[];
  view: ViewId;
  selected: string | null;
  dialog: DialogState | null;
  refreshing: boolean;
  busy: boolean;
  notices: string[];
  focusTaskId: string | null;
}

export interface AppActions {
  setView(view: ViewId): void;
  select(name: string | null): void;
  queue(change: Change): void;
  unqueue(name: string): void;
  discard(): void;
  reviewPending(): void;
  reviewChanges(changes: Change[], title: Msg | null): Promise<void>;
  replan(options: ApplyOptions): Promise<void>;
  apply(saveRecipe: string | null): Promise<void>;
  openRecipes(): void;
  openSettings(section?: SettingsSection): void;
  closeDialog(): void;
  refresh(): void;
  runFix(problem: Problem, fix: Fix): void;
  openProject(path?: string): Promise<boolean>;
  cancelTask(id: string): void;
  rerun(task: Task): void;
  closeEditor(): void;
  /** Saves the settings; resolves to what the backend stored (it normalizes them), or null on failure. */
  saveSettings(next: Settings): Promise<Settings | null>;
  /** Re-reads the settings from the backend (it changes them behind the UI); null on failure. */
  refreshSettings(): Promise<Settings | null>;
  setSnapshot(snapshot: Snapshot): void;
  report(error: unknown): void;
  dismissNotice(index: number): void;
}

interface AppContextValue { state: AppState; actions: AppActions }

const AppContext = createContext<AppContextValue | null>(null);

export function useApp(): AppContextValue {
  const ctx = useContext(AppContext);
  if (!ctx) throw new Error("useApp outside AppProvider");
  return ctx;
}

export function applyTheme(theme: Settings["theme"]): void {
  document.documentElement.dataset["theme"] = theme;
}

function defaultSelection(snapshot: Snapshot): string | null {
  const requested = mockParams.select;
  if (requested) {
    const match = snapshot.packages.find((p) => sameName(p.name, requested));
    if (match) return match.name;
  }
  const withProblem = snapshot.packages.find((p) =>
    snapshot.problems.some((x) => p.problemIds.includes(x.id) && x.severity !== "info"));
  return withProblem?.name ?? snapshot.packages[0]?.name ?? null;
}

export function defaultOptions(settings: Settings): ApplyOptions {
  return { build: true, reopenEditor: settings.build.reopenEditor, runTests: settings.build.runTests };
}

export function AppProvider(props: { snapshot: Snapshot; settings: Settings; tasks: Task[]; children: ReactNode }) {
  const [snapshot, setSnapshotState] = useState(props.snapshot);
  const [settings, setSettings] = useState(props.settings);
  const [tasks, setTasks] = useState(props.tasks);
  const [pending, setPending] = useState<Change[]>(initialChanges);
  const [view, setViewState] = useState<ViewId>(mockParams.view ?? "packages");
  const [selected, setSelected] = useState<string | null>(() => defaultSelection(props.snapshot));
  const [dialog, setDialog] = useState<DialogState | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  const [busy, setBusy] = useState(false);
  const [notices, setNotices] = useState<string[]>([]);
  const [focusTaskId, setFocusTaskId] = useState<string | null>(null);

  const stateRef = useRef({ pending, settings, dialog, snapshot });
  stateRef.current = { pending, settings, dialog, snapshot };
  // The project of the latest snapshot taken in. Updated on arrival, not on render, so two snapshots
  // arriving back to back are each compared with the one before.
  const projectPathRef = useRef<string | null>(props.snapshot.project?.path ?? null);

  const report = useCallback((error: unknown) => {
    const text = errorText(error);
    setNotices((list) => (list.includes(text) ? list : [...list, text]));
  }, []);

  const refreshSettings = useCallback(async (): Promise<Settings | null> => {
    try {
      const fresh = await api.getSettings();
      setSettings(fresh);
      return fresh;
    } catch (e) {
      report(e);
      return null;
    }
  }, [report]);

  /**
   * Takes a snapshot in. When it belongs to another project than the one before, everything tied to that
   * project is dropped: queued changes, the task focus, the selection and, for `external` switches (a second
   * launch from UE hands over a project and the backend only emits `ubot://snapshot`), a plan or recipe
   * dialog that was prepared for the old project. Returns whether the project changed.
   */
  const adoptSnapshot = useCallback((next: Snapshot, external: boolean): boolean => {
    const nextPath = next.project?.path ?? null;
    const { switched, dropsProjectState } = projectSwitch(projectPathRef.current, nextPath);
    projectPathRef.current = nextPath;
    setSnapshotState(next);
    if (switched) {
      setSelected(defaultSelection(next));
      if (dropsProjectState) {
        setPending([]);
        setFocusTaskId(null);
        // Settings stays open: it follows the new project's settings itself.
        if (external) setDialog((d) => (d && d.kind === "settings" ? d : null));
      }
    } else {
      setSelected((current) => current && next.packages.some((p) => sameName(p.name, current)) ? current : defaultSelection(next));
    }
    return switched;
  }, []);

  const setSnapshot = useCallback((next: Snapshot) => {
    // The per-project settings (ROS host and port, recent projects) belong to the new project too.
    if (adoptSnapshot(next, true)) void refreshSettings();
  }, [adoptSnapshot, refreshSettings]);

  useEffect(() => {
    const offSnapshot = api.onSnapshot(setSnapshot);
    const offTask = api.onTask((task) => {
      setTasks((list) => {
        const index = list.findIndex((x) => x.id === task.id);
        if (index < 0) return [task, ...list];
        const copy = list.slice();
        copy[index] = task;
        return copy;
      });
    });
    return () => {
      offSnapshot();
      offTask();
    };
  }, [setSnapshot]);

  const openPlanDialog = useCallback(async (changes: Change[], title: Msg | null, fromPending: boolean) => {
    const options = defaultOptions(stateRef.current.settings);
    const project = projectPathRef.current;
    setBusy(true);
    try {
      const plan = await api.plan(changes, options);
      // A second launch may have switched the project while the backend was planning: that plan is for the old one.
      if (!sameProjectPath(project, projectPathRef.current)) return;
      setDialog({ kind: "plan", plan, options, title, fromPending });
    } catch (e) {
      report(e);
    } finally {
      setBusy(false);
    }
  }, [report]);

  const openProject = useCallback(async (path?: string) => {
    let target = path ?? null;
    if (!target) target = await platform.pickProjectFile(t("dialog.projectPathPrompt"), t("dialog.uprojectFilter"));
    if (!target) return false;
    try {
      const next = await api.openProject(target);
      // Opened from inside the app (the recipe dialog does it mid-review): its dialogs stay. The queued changes
      // go only when the project is another one: opening the open project again keeps them.
      adoptSnapshot(next, false);
      await refreshSettings();
      return true;
    } catch (e) {
      report(e);
      return false;
    }
  }, [adoptSnapshot, refreshSettings, report]);

  const actions = useMemo<AppActions>(() => ({
    setView: (next) => setViewState(next),
    select: (name) => setSelected(name),
    queue: (change) => setPending((list) => upsertChange(list, change)),
    unqueue: (name) => setPending((list) => list.filter((c) => !sameName(c.package, name))),
    discard: () => setPending([]),
    reviewPending: () => void openPlanDialog(stateRef.current.pending, null, true),
    reviewChanges: (changes, title) => openPlanDialog(changes, title, false),
    async replan(options) {
      const current = stateRef.current.dialog;
      if (!current || current.kind !== "plan") return;
      const project = projectPathRef.current;
      try {
        const plan = await api.plan(current.plan.changes, options);
        if (!sameProjectPath(project, projectPathRef.current)) return;
        setDialog((d) => (d && d.kind === "plan" ? { ...d, plan, options } : d));
      } catch (e) {
        report(e);
      }
    },
    async apply(saveRecipe) {
      const current = stateRef.current.dialog;
      if (!current || current.kind !== "plan") return;
      setBusy(true);
      try {
        const id = await api.apply(current.plan.changes, current.options, current.title, saveRecipe);
        if (current.fromPending) setPending([]);
        else setPending((list) => list.filter((c) => !findChange(current.plan.changes, c.package)));
        setDialog(null);
        // The new task's first event may still be on its way. Have the task in the list before the Tasks
        // view looks for it; a failed listing is fine, the event adds the task.
        const listed = await api.listTasks().catch(() => [] as Task[]);
        setTasks((list) => mergeTasks(list, listed));
        setFocusTaskId(id);
        setViewState("tasks");
      } catch (e) {
        report(e);
      } finally {
        setBusy(false);
      }
    },
    openRecipes: () => setDialog({ kind: "recipes" }),
    openSettings: (section) => setDialog({ kind: "settings", section: section ?? "appearance" }),
    closeDialog: () => setDialog(null),
    refresh() {
      setRefreshing(true);
      api.refresh(true).then(setSnapshot, report).finally(() => setRefreshing(false));
    },
    runFix(problem, fix) {
      const action = fix.action;
      switch (action.kind) {
        case "changes":
          setPending((list) => action.changes.reduce(upsertChange, list));
          break;
        case "ignore":
          api.ignoreProblem(problem.id).then(setSnapshot, report);
          break;
        case "openSettings":
          setDialog({ kind: "settings", section: action.section });
          break;
      }
    },
    openProject,
    cancelTask: (id) => void api.cancelTask(id).catch(report),
    rerun(task) {
      void openPlanDialog(task.changes, task.title, false);
    },
    closeEditor() {
      // The editor may still be showing a save prompt; the Apply Changes dialog keeps polling for it.
      api.closeEditor().then(() => api.getSnapshot()).then(setSnapshot).catch(report);
    },
    async saveSettings(next) {
      try {
        const snap = await api.saveSettings(next);
        // The backend normalizes what it stores (blank sources dropped, values trimmed): show that, not the draft.
        const stored = await api.getSettings().catch(() => next);
        setSettings(stored);
        setSnapshot(snap);
        applyTheme(stored.theme);
        return stored;
      } catch (e) {
        report(e);
        return null;
      }
    },
    refreshSettings,
    setSnapshot,
    report,
    dismissNotice: (index) => setNotices((list) => list.filter((_, i) => i !== index))
  }), [openPlanDialog, openProject, refreshSettings, report, setSnapshot]);

  // Mock-only: open the dialog named in the URL once the app is up.
  const openedFromParams = useRef(false);
  useEffect(() => {
    if (openedFromParams.current) return;
    openedFromParams.current = true;
    if (mockParams.dialog === "plan" && pending.length > 0) actions.reviewPending();
    else if (mockParams.dialog === "recipes") actions.openRecipes();
    else if (mockParams.dialog === "settings") actions.openSettings(mockParams.section ?? "appearance");
  }, [actions, pending.length]);

  const state: AppState = {
    snapshot, settings, tasks, pending, view, selected, dialog, refreshing, busy, notices, focusTaskId
  };
  return <AppContext.Provider value={{ state, actions }}>{props.children}</AppContext.Provider>;
}

/** Problems of the packages scope, most severe first. */
export function packageProblems(snapshot: Snapshot): Problem[] {
  return snapshot.problems.filter((p) => p.scope === "packages").sort(bySeverity);
}
