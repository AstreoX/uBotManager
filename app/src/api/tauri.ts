import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getVersion } from "@tauri-apps/api/app";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { open } from "@tauri-apps/plugin-dialog";
import { openPath, openUrl, revealItemInDir } from "@tauri-apps/plugin-opener";
import type { Backend, GitTestResult, Platform, RosTestResult, TaskLogEvent, Unlisten } from "./backend";
import type {
  ApplyOptions, Change, EngineInfo, EnvCheck, Msg, Plan, Settings, Snapshot, Task, UpdateInfo
} from "./types";

function subscribe<T>(event: string, handler: (payload: T) => void): Unlisten {
  let disposed = false;
  let unlisten: UnlistenFn | null = null;
  listen<T>(event, (e) => handler(e.payload)).then((fn) => {
    if (disposed) fn();
    else unlisten = fn;
  }, (error: unknown) => console.error(error));
  return () => {
    disposed = true;
    unlisten?.();
  };
}

export const tauriBackend: Backend = {
  getSnapshot: () => invoke<Snapshot>("get_snapshot"),
  refresh: (sync: boolean) => invoke<Snapshot>("refresh", { sync }),
  openProject: (path: string) => invoke<Snapshot>("open_project", { path }),
  plan: (changes: Change[], options: ApplyOptions) => invoke<Plan>("plan", { changes, options }),
  apply: (changes: Change[], options: ApplyOptions, title: Msg | null, saveRecipe: string | null) =>
    invoke<string>("apply", { changes, options, title, saveRecipe }),
  cancelTask: (taskId: string) => invoke<void>("cancel_task", { taskId }),
  listTasks: () => invoke<Task[]>("list_tasks"),
  getTaskLog: (taskId: string, stepId: string | null) => invoke<string[]>("get_task_log", { taskId, stepId }),
  closeEditor: () => invoke<void>("close_editor"),
  ignoreProblem: (id: string) => invoke<Snapshot>("ignore_problem", { id }),
  getSettings: () => invoke<Settings>("get_settings"),
  saveSettings: (settings: Settings) => invoke<Snapshot>("save_settings", { settings }),
  testGit: (path: string) => invoke<GitTestResult>("test_git", { path }),
  testRos: (host: string, port: number) => invoke<RosTestResult>("test_ros", { host, port }),
  checkForUpdate: () => invoke<UpdateInfo>("check_for_update"),
  listEngines: () => invoke<EngineInfo[]>("list_engines"),
  checkEnvironment: (checks: string[]) => invoke<EnvCheck[]>("check_environment", { checks }),
  recipeChanges: (recipeId: string | null, packages: string[]) =>
    invoke<Change[]>("recipe_changes", { recipeId, packages }),

  onSnapshot: (handler) => subscribe<Snapshot>("ubot://snapshot", handler),
  onTask: (handler) => subscribe<Task>("ubot://task", handler),
  onTaskLog: (handler) => subscribe<TaskLogEvent>("ubot://task-log", handler)
};

/** Window and shell calls fail only when a capability is missing; log instead of rejecting unhandled. */
function run(action: Promise<unknown>): void {
  action.catch((error: unknown) => console.error(error));
}

export const tauriPlatform: Platform = {
  isTauri: true,
  minimizeWindow: () => run(getCurrentWindow().minimize()),
  toggleMaximizeWindow: () => run(getCurrentWindow().toggleMaximize()),
  closeWindow: () => run(getCurrentWindow().close()),
  onMaximizedChange(handler) {
    const win = getCurrentWindow();
    const update = () => run(win.isMaximized().then(handler));
    update();
    let disposed = false;
    let unlisten: UnlistenFn | null = null;
    run(win.onResized(update).then((fn) => {
      if (disposed) fn();
      else unlisten = fn;
    }));
    return () => {
      disposed = true;
      unlisten?.();
    };
  },
  async pickProjectFile(title, filterName) {
    const picked = await open({ title, multiple: false, directory: false, filters: [{ name: filterName, extensions: ["uproject"] }] });
    return typeof picked === "string" ? picked : null;
  },
  appVersion: () => getVersion(),
  openUrl: (url) => run(openUrl(url)),
  openPath: (path) => run(openPath(path)),
  revealPath: (path) => run(revealItemInDir(path))
};
