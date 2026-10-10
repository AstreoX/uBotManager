// API contract between the web frontend and the Tauri backend (docs/SPEC.md §7). Keep verbatim.

export type Lang = 'en' | 'zh-CN';
export type Layer = 'Foundation' | 'Capability' | 'Adapter' | 'Content' | 'Unknown';
export interface LocalizedText { en: string; 'zh-CN': string | null }
/** Message for the UI: `code` is a key under `msg.` in the locale files; params are plain strings. */
export interface Msg { code: string; params: Record<string, string> }

export interface EngineInfo { id: string; version: string; root: string; kind: 'launcher' | 'source' | 'custom' }
export interface ProjectInfo {
  path: string; name: string; dir: string; engineAssociation: string;
  engine: EngineInfo | null; hasCode: boolean; buildTarget: string;
  editor: { running: boolean; pid: number | null };
}

export type PackageState = 'enabled' | 'disabled' | 'notInstalled' | 'planned';
export interface Requirement {
  name: string; constraint: string; optional: boolean; kind: 'package' | 'enginePlugin';
  status: 'satisfied' | 'versionMismatch' | 'notInstalled' | 'disabled' | 'missing';
  installedVersion: string | null;
}
export interface GitState { isCheckout: boolean; ref: string | null; dirty: boolean }
export interface PackageInfo {
  name: string; friendlyName: string; description: LocalizedText; layer: Layer; state: PackageState;
  installedVersion: string | null; latestVersion: string | null;
  versions: string[];               // non-planned index versions, newest first
  plannedVersion: string | null;
  updateAvailable: boolean;
  location: string | null;          // project-relative, '/' separators ("Plugins/uBot/uBotSensor", "../Plugins/uBot/uBotCore")
  git: GitState | null;             // null when not installed
  repository: string | null; webUrl: string | null; docsUrl: string | null;
  indexSource: string | null;       // id of the source that defined it
  requires: Requirement[]; provides: string[]; tags: string[];
  problemIds: string[];
}
export interface PluginInfo { name: string; enabled: boolean; location: string; version: string | null; isUBot: boolean }

export interface IndexSourceStatus {
  id: string; name: string; location: string; builtin: boolean; enabled: boolean;
  ok: boolean; fromCache: boolean; error: string | null; packages: number; fetchedAt: string | null; // RFC 3339
}
export interface Recipe {
  id: string; name: LocalizedText; description: LocalizedText; packages: string[]; checks: string[];
  available: boolean; user: boolean;
}

export type Severity = 'error' | 'warning' | 'info';
export type SettingsSection = 'appearance' | 'index' | 'install' | 'build' | 'ros';
export interface ProblemFact { label: Msg; value: string; valueMsg: Msg | null }
export type FixAction =
  | { kind: 'changes'; changes: Change[] }
  | { kind: 'ignore' }
  | { kind: 'openSettings'; section: SettingsSection };
export interface Fix { label: Msg; primary: boolean; action: FixAction }
export interface Problem {
  id: string; scope: 'packages' | 'session'; group: string; severity: Severity;
  message: Msg; facts: ProblemFact[]; fixes: Fix[];
}

export interface SessionStatus {        // parsed Saved/uBot/status.json, see §9.2
  kind: 'PIE' | 'Editor' | 'Game'; startedAt: string; endedAt: string | null; engineVersion: string;
}

export interface Snapshot {
  project: ProjectInfo | null;
  packages: PackageInfo[];              // index + installed, sorted: layer order, then name
  plugins: PluginInfo[];                // every discovered project plugin
  problems: Problem[];                  // ignored ones removed
  index: IndexSourceStatus[];
  recipes: Recipe[];                    // index recipes then user recipes
  session: SessionStatus | null;
  syncing: boolean;
}

export type ChangeKind = 'install' | 'update' | 'enable' | 'disable' | 'uninstall';
export interface Change { kind: ChangeKind; package: string; version: string | null }
export type StepGroup = 'download' | 'project' | 'build' | 'after';
export type StepKind = 'clone' | 'checkout' | 'enablePackage' | 'disablePackage' | 'enableEnginePlugin'
  | 'removePackage' | 'build' | 'runTests' | 'openEditor';
export interface PlanStep { id: string; n: number; group: StepGroup; kind: StepKind; title: Msg; detail: Msg; needsEditorClosed: boolean }
export interface ApplyOptions { build: boolean; reopenEditor: boolean; runTests: boolean }
export interface Plan {
  changes: Change[]; steps: PlanStep[]; needsBuild: boolean; editorRunning: boolean;
  errors: Msg[]; warnings: Msg[];
}

export type StepStatus = 'pending' | 'waiting' | 'running' | 'done' | 'failed' | 'skipped' | 'canceled';
export interface TaskStep {
  id: string; title: Msg; status: StepStatus; startedAt: string | null; durationMs: number | null;
  progress: { done: number; total: number } | null; logLines: number;
}
export interface Task {
  id: string; title: Msg; project: string; status: StepStatus; startedAt: string;
  durationMs: number | null; steps: TaskStep[]; error: string | null; changes: Change[];
}

export interface Settings {
  language: Lang; theme: 'dark' | 'light' | 'system';
  index: { sources: IndexSourceConfig[]; syncOnStart: boolean; showPlanned: boolean };
  install: { directory: string; git: string; usePrebuilt: boolean };
  build: { engineId: string | null; configuration: 'Development' | 'DebugGame'; waitForEditor: boolean; reopenEditor: boolean; runTests: boolean };
  ros: { host: string; port: number; checkOnRefresh: boolean };
  recentProjects: string[]; lastProject: string | null;
}
export interface IndexSourceConfig { id: string; name: string; location: string; enabled: boolean; builtin: boolean }
export interface EnvCheck { id: 'git' | 'vs2022' | 'engine' | 'rosBridge'; ok: boolean | null; detail: string | null }
