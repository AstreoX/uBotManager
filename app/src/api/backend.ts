import type {
  ApplyOptions, Change, EngineInfo, EnvCheck, Msg, Plan, Settings, Snapshot, Task, UpdateInfo
} from "./types";

export interface TaskLogEvent { taskId: string; stepId: string; lines: string[] }
export interface GitTestResult { ok: boolean; version: string | null; error: string | null }
export interface RosTestResult { ok: boolean; error: string | null }

export type Unlisten = () => void;

/** The commands and events of SPEC §7, implemented by the Tauri backend and by the in-memory mock. */
export interface Backend {
  getSnapshot(): Promise<Snapshot>;
  refresh(sync: boolean): Promise<Snapshot>;
  openProject(path: string): Promise<Snapshot>;
  plan(changes: Change[], options: ApplyOptions): Promise<Plan>;
  apply(changes: Change[], options: ApplyOptions, title: Msg | null, saveRecipe: string | null): Promise<string>;
  cancelTask(taskId: string): Promise<void>;
  listTasks(): Promise<Task[]>;
  getTaskLog(taskId: string, stepId: string | null): Promise<string[]>;
  closeEditor(): Promise<void>;
  ignoreProblem(id: string): Promise<Snapshot>;
  getSettings(): Promise<Settings>;
  saveSettings(settings: Settings): Promise<Snapshot>;
  testGit(path: string): Promise<GitTestResult>;
  testRos(host: string, port: number): Promise<RosTestResult>;
  /** One request to GitHub for the latest uBot Manager release; never downloads or installs anything. */
  checkForUpdate(): Promise<UpdateInfo>;
  listEngines(): Promise<EngineInfo[]>;
  checkEnvironment(checks: string[]): Promise<EnvCheck[]>;
  recipeChanges(recipeId: string | null, packages: string[]): Promise<Change[]>;

  onSnapshot(handler: (snapshot: Snapshot) => void): Unlisten;
  onTask(handler: (task: Task) => void): Unlisten;
  onTaskLog(handler: (event: TaskLogEvent) => void): Unlisten;
}

/** Desktop integration that is not part of the backend contract (window chrome, native dialogs, shell). */
export interface Platform {
  readonly isTauri: boolean;
  minimizeWindow(): void;
  toggleMaximizeWindow(): void;
  closeWindow(): void;
  onMaximizedChange(handler: (maximized: boolean) => void): Unlisten;
  /** Asks for a `.uproject` file; resolves to null when the user cancels. */
  pickProjectFile(title: string, filterName: string): Promise<string | null>;
  /** The version of the running app. */
  appVersion(): Promise<string>;
  openUrl(url: string): void;
  openPath(path: string): void;
  revealPath(path: string): void;
}
