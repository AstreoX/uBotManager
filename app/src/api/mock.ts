// In-memory backend used when the frontend runs outside Tauri (vite dev / preview, screenshots).
// It reproduces the design scenario: project MyRobotSim with UBotCore 0.1.0, UBotSensor 0.1.0 (0.1.1
// available), UBotEnvironment 0.2.0 needing UBotCore ^0.2.0, UBotROS 0.1.0, UBotCombat disabled, the
// legacy AgentSensorCore plugin and a ROS session problem. Package and recipe data come from a copy of
// uBotCore's Index/index.json (mockIndex.json) with scenario versions layered on top.

import { renderMsg } from "../i18n";
import { compareVersionStrings, satisfies } from "../lib/semver";
import type { Backend, GitTestResult, RosTestResult, TaskLogEvent } from "./backend";
import type { MockParams } from "./mockParams";
import indexFile from "./mockIndex.json";
import type {
  ApplyOptions, Change, EngineInfo, EnvCheck, Fix, IndexSourceStatus, Layer, Msg, PackageInfo, Plan,
  PlanStep, PluginInfo, Problem, Recipe, Requirement, Settings, Snapshot, StepGroup, StepKind, Task, TaskStep
} from "./types";

interface IndexRequire { name: string; version: string; optional?: boolean }
interface IndexVersion {
  version: string; ref?: string; planned?: boolean; engine?: string;
  requires?: IndexRequire[]; enginePlugins?: string[]; provides?: string[];
}
interface IndexPackage {
  name: string; friendlyName?: string; description: { en: string; "zh-CN"?: string };
  layer: string; repository?: string; folder?: string; docsUrl?: string; tags?: string[];
  versions: IndexVersion[];
}
interface IndexRecipe {
  id: string; name: { en: string; "zh-CN"?: string }; description: { en: string; "zh-CN"?: string };
  packages: string[]; checks?: string[];
}
interface IndexReplacement { legacy: string; replacement: string; redirects?: string }
interface IndexData { name: string; packages: IndexPackage[]; recipes?: IndexRecipe[]; replacements?: IndexReplacement[] }

interface Installed {
  name: string; version: string; enabled: boolean; ubot: boolean; location: string;
  requires: IndexRequire[]; enginePlugins: string[]; provides: string[];
}

type InstalledLike = Pick<Installed, "name" | "enabled" | "requires">;

interface ProjectState {
  path: string; name: string; dir: string; editorRunning: boolean; editorPid: number | null;
  installed: Map<string, Installed>;
  enabledEnginePlugins: Set<string>;
}

const REMOTE_INDEX = "https://raw.githubusercontent.com/AstreoX/uBotCore/main/Index/index.json";
const SCENARIO_PROJECT = "D:\\Projects\\MyRobotSim\\MyRobotSim.uproject";
const ENGINES: EngineInfo[] = [
  { id: "5.5", version: "5.5.4", root: "C:\\Program Files\\Epic Games\\UE_5.5", kind: "launcher" },
  { id: "5.4", version: "5.4.4", root: "C:\\Program Files\\Epic Games\\UE_5.4", kind: "launcher" }
];
const ENGINE_PLUGINS = ["ChaosVehiclesPlugin", "LearningAgents", "EnhancedInput", "ModelingToolsEditorMode", "PythonScriptPlugin"];
const KNOWN_CHECKS = ["git", "vs2022", "engine", "rosBridge"];
const LAYER_ORDER: Layer[] = ["Foundation", "Capability", "Adapter", "Content", "Unknown"];

const key = (name: string) => name.toLowerCase();
const clone = <T>(value: T): T => structuredClone(value);
const delay = (ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms));
const nowIso = () => new Date().toISOString();

function scenarioIndex(): IndexData {
  const index = clone(indexFile) as IndexData;
  const pkg = (name: string) => index.packages.find((p) => p.name === name);
  const release = (name: string, version = "0.1.0") => {
    const v = pkg(name)?.versions.find((x) => x.version === version);
    if (v) {
      v.planned = false;
      v.ref = `v${version}`;
      v.engine = "~5.5";
    }
  };
  const core = pkg("UBotCore");
  const core010 = core?.versions[0];
  if (core && core010) core.versions.push({ ...clone(core010), version: "0.2.0", ref: "v0.2.0" });
  const sensor = pkg("UBotSensor");
  const sensor010 = sensor?.versions[0];
  if (sensor && sensor010) sensor.versions.push({ ...clone(sensor010), version: "0.1.1", ref: "v0.1.1" });
  ["UBotVehicles", "UBotAerial", "UBotMCP", "UBotEnvironment", "UBotCombat"].forEach((name) => release(name));
  const env = pkg("UBotEnvironment");
  const env010 = env?.versions[0];
  if (env && env010) {
    env.versions.push({
      ...clone(env010), version: "0.2.0", ref: "v0.2.0",
      requires: [{ name: "UBotCore", version: "^0.2.0" }]
    });
  }
  return index;
}

const INDEX = scenarioIndex();

function indexPackage(name: string): IndexPackage | undefined {
  return INDEX.packages.find((p) => key(p.name) === key(name));
}

function releasedVersions(p: IndexPackage): IndexVersion[] {
  return p.versions.filter((v) => !v.planned).sort((a, b) => compareVersionStrings(b.version, a.version));
}

function plannedVersions(p: IndexPackage): IndexVersion[] {
  return p.versions.filter((v) => v.planned).sort((a, b) => compareVersionStrings(b.version, a.version));
}

function folderOf(p: IndexPackage): string {
  return p.folder ?? (p.repository ?? p.name).replace(/\.git$/, "").split("/").pop() ?? p.name;
}

function displayRepo(url: string): string {
  return url.replace(/^https?:\/\//, "").replace(/\.git$/, "");
}

function asLayer(layer: string): Layer {
  return (LAYER_ORDER as string[]).includes(layer) ? (layer as Layer) : "Unknown";
}

function slug(text: string): string {
  return text.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "") || "recipe";
}

function baseName(path: string): string {
  return path.split(/[\\/]/).pop()?.replace(/\.uproject$/i, "") ?? path;
}

function dirName(path: string): string {
  const parts = path.split(/[\\/]/);
  parts.pop();
  return parts.join("\\");
}

interface RunState {
  task: Task;
  plan: Plan;
  options: ApplyOptions;
  saveRecipe: string | null;
  canceled: boolean;
  buildLineMs: number;
}

export function createMockBackend(params: MockParams): { backend: Backend; initialChanges: Change[] } {
  const snapshotListeners = new Set<(s: Snapshot) => void>();
  const taskListeners = new Set<(t: Task) => void>();
  const logListeners = new Set<(e: TaskLogEvent) => void>();

  const settings: Settings = {
    language: params.lang ?? "en",
    theme: params.theme ?? "dark",
    index: {
      sources: [
        { id: "ubot", name: "uBotCore", location: REMOTE_INDEX, enabled: true, builtin: true },
        { id: "local", name: "Local copy", location: "Plugins/uBot/uBotCore/Index/index.json", enabled: true, builtin: true },
        { id: "team", name: params.lang === "zh-CN" ? "团队索引" : "Team index", location: "D:\\Shared\\ubot-index.json", enabled: true, builtin: false }
      ],
      syncOnStart: true,
      showPlanned: true
    },
    install: { directory: "Plugins/uBot", git: "git", usePrebuilt: false },
    build: { engineId: null, configuration: "Development", waitForEditor: true, reopenEditor: true, runTests: false },
    ros: { host: "127.0.0.1", port: 8268, checkOnRefresh: true },
    recentProjects: [SCENARIO_PROJECT, "D:\\Projects\\WarehouseBot\\WarehouseBot.uproject"],
    lastProject: params.noProject ? null : SCENARIO_PROJECT
  };
  const userRecipes: Recipe[] = [{
    id: "user-myrobotsim-baseline",
    name: { en: "MyRobotSim baseline", "zh-CN": null },
    description: { en: "UBotCore, UBotSensor, UBotROS", "zh-CN": null },
    packages: ["UBotCore", "UBotSensor", "UBotROS"],
    checks: ["rosBridge"],
    available: true,
    user: true
  }];
  const ignored = new Map<string, Set<string>>();
  const tasks: Task[] = [];
  const logs = new Map<string, string[]>();
  let project: ProjectState | null = params.noProject ? null : scenarioProject();
  let syncing = false;
  let fetchedAt = nowIso();
  let running: RunState | null = null;
  let taskCounter = 0;

  function scenarioProject(): ProjectState {
    const installed = new Map<string, Installed>();
    const add = (name: string, version: string, enabled: boolean, requires?: IndexRequire[]) => {
      const p = indexPackage(name);
      const v = p?.versions.find((x) => x.version === version);
      if (!p || !v) return;
      installed.set(key(name), {
        name: p.name, version, enabled, ubot: true,
        location: `Plugins/uBot/${folderOf(p)}`,
        requires: requires ?? clone(v.requires ?? []),
        enginePlugins: clone(v.enginePlugins ?? []),
        provides: clone(v.provides ?? [])
      });
    };
    add("UBotCore", "0.1.0", true);
    add("UBotSensor", "0.1.0", true);
    add("UBotEnvironment", "0.2.0", true);
    add("UBotROS", "0.1.0", true);
    add("UBotCombat", "0.1.0", false, [
      { name: "UBotCore", version: "^0.1.0" },
      { name: "UBotSensor", version: "^0.1.0" },
      { name: "UBotEnvironment", version: "^0.2.0" }
    ]);
    installed.set(key("AgentSensorCore"), {
      name: "AgentSensorCore", version: "0.5.0", enabled: true, ubot: false,
      location: "Plugins/AgentSensorCore", requires: [], enginePlugins: [], provides: []
    });
    return {
      path: SCENARIO_PROJECT, name: "MyRobotSim", dir: dirName(SCENARIO_PROJECT),
      editorRunning: true, editorPid: 18244, installed, enabledEnginePlugins: new Set()
    };
  }

  function emptyProject(path: string): ProjectState {
    return {
      path, name: baseName(path), dir: dirName(path), editorRunning: false, editorPid: null,
      installed: new Map(), enabledEnginePlugins: new Set()
    };
  }

  /** ROS host and port are per project (Config/DefaultUBotROS.ini); a project without its own uses the app's. */
  const projectRos = new Map<string, Settings["ros"]>([
    [key("D:\\Projects\\WarehouseBot\\WarehouseBot.uproject"), { host: "10.0.0.7", port: 9090, checkOnRefresh: true }]
  ]);
  function rosFor(p: ProjectState | null): Settings["ros"] {
    return clone((p && projectRos.get(key(p.path))) || settings.ros);
  }

  function rememberProject(path: string) {
    settings.recentProjects = [path, ...settings.recentProjects.filter((r) => key(r) !== key(path))].slice(0, 10);
    settings.lastProject = path;
  }

  /** What settings.rs normalize does to a saved object, as far as the dialog can produce it. */
  function normalizeSettings(next: Settings): Settings {
    const sources: Settings["index"]["sources"] = [];
    for (const source of next.index.sources) {
      if (source.builtin) {
        sources.push(source);
        continue;
      }
      const location = source.location.trim();
      if (!location) continue;
      let id = source.id.trim();
      if (!id || sources.some((x) => x.id === id)) {
        let counter = 1;
        while (sources.some((x) => x.id === `user-${counter}`)) counter++;
        id = `user-${counter}`;
      }
      sources.push({ ...source, id, name: source.name.trim() || location, location });
    }
    next.index.sources = sources;
    next.install.directory = next.install.directory.trim() || "Plugins/uBot";
    next.install.git = next.install.git.trim() || "git";
    next.ros.host = next.ros.host.trim() || "127.0.0.1";
    return next;
  }

  // ---------------------------------------------------------------- snapshot

  const engine = (): EngineInfo | null => {
    const id = settings.build.engineId ?? "5.5";
    return ENGINES.find((e) => e.id === id) ?? null;
  };

  function enginePluginEnabled(p: ProjectState, name: string): boolean {
    return p.enabledEnginePlugins.has(key(name));
  }

  /** A package's requirements and engine plugins, evaluated against the open project. */
  function requirementRows(p: ProjectState | null, requires: IndexRequire[], enginePlugins: string[]): Requirement[] {
    const rows: Requirement[] = requires.map((r) => {
      const inst = p?.installed.get(key(r.name));
      const status: Requirement["status"] = inst
        ? !satisfies(inst.version, r.version) ? "versionMismatch" : inst.enabled ? "satisfied" : "disabled"
        : indexPackage(r.name) ? "notInstalled" : "missing";
      return {
        name: indexPackage(r.name)?.name ?? r.name, constraint: r.version, optional: !!r.optional,
        kind: "package", status, installedVersion: inst?.version ?? null
      };
    });
    for (const plugin of enginePlugins) {
      const status: Requirement["status"] = !ENGINE_PLUGINS.some((e) => key(e) === key(plugin)) ? "missing"
        : p && enginePluginEnabled(p, plugin) ? "satisfied" : "disabled";
      rows.push({ name: plugin, constraint: "", optional: false, kind: "enginePlugin", status, installedVersion: null });
    }
    return rows;
  }

  /** Index versions of `name` that every enabled dependent (other than `except`) accepts, newest first. */
  function compatibleVersions(installed: Iterable<InstalledLike>, name: string, extraConstraint: string | null, except?: string): string[] {
    const pkg = indexPackage(name);
    if (!pkg) return [];
    const constraints: string[] = extraConstraint ? [extraConstraint] : [];
    for (const inst of installed) {
      if (!inst.enabled || (except && key(inst.name) === key(except))) continue;
      for (const r of inst.requires) if (key(r.name) === key(name) && !r.optional) constraints.push(r.version);
    }
    return releasedVersions(pkg).map((v) => v.version).filter((v) => constraints.every((c) => satisfies(v, c)));
  }

  function computeProblems(p: ProjectState | null): Problem[] {
    if (!p) return [];
    const problems: Problem[] = [];
    const msg = (code: string, params: Record<string, string> = {}): Msg => ({ code, params });
    const fact = (label: string, value: string): Problem["facts"][number] => ({ label: msg(`fact.${label}`), value, valueMsg: null });
    for (const inst of p.installed.values()) {
      if (!inst.ubot || !inst.enabled) continue;
      const disableFix: Fix = { label: msg("fix.disable", { package: inst.name }), primary: false, action: { kind: "changes", changes: [{ kind: "disable", package: inst.name, version: null }] } };
      for (const r of inst.requires) {
        if (r.optional) continue;
        const dep = p.installed.get(key(r.name));
        const depName = indexPackage(r.name)?.name ?? r.name;
        const id = `requires:${inst.name}:${depName}`;
        const facts = [fact("package", `${inst.name} ${inst.version}`), fact("requires", `${depName} ${r.version}`)];
        if (!dep) {
          const fixes: Fix[] = [];
          if (compatibleVersions(p.installed.values(), depName, r.version).length > 0) {
            fixes.push({ label: msg("fix.install", { package: depName }), primary: true, action: { kind: "changes", changes: [{ kind: "install", package: depName, version: null }] } });
          }
          fixes.push({ ...disableFix, primary: fixes.length === 0 });
          problems.push({ id, scope: "packages", group: inst.name, severity: "warning", message: msg("problem.requiresMissing", { package: inst.name, dep: depName, range: r.version }), facts, fixes });
        } else if (!satisfies(dep.version, r.version)) {
          const fixes: Fix[] = [];
          const depPkg = indexPackage(depName);
          const best = depPkg ? releasedVersions(depPkg).map((v) => v.version).find((v) => satisfies(v, r.version)) : undefined;
          if (best) {
            fixes.push({ label: msg("fix.updateTo", { package: depName, version: best }), primary: true, action: { kind: "changes", changes: [{ kind: "update", package: depName, version: best }] } });
          }
          fixes.push({ ...disableFix, primary: fixes.length === 0 });
          problems.push({
            id, scope: "packages", group: inst.name, severity: "warning",
            message: msg("problem.requiresVersion", { package: inst.name, dep: depName, range: r.version, installed: dep.version }),
            facts: [...facts, fact("installed", `${depName} ${dep.version}`)], fixes
          });
        } else if (!dep.enabled) {
          problems.push({
            id, scope: "packages", group: inst.name, severity: "warning",
            message: msg("problem.requiresDisabled", { package: inst.name, dep: depName }), facts,
            fixes: [{ label: msg("fix.enable", { package: depName }), primary: true, action: { kind: "changes", changes: [{ kind: "enable", package: depName, version: null }] } }, disableFix]
          });
        }
      }
      for (const plugin of inst.enginePlugins) {
        const id = `enginePlugin:${inst.name}:${plugin}`;
        if (!ENGINE_PLUGINS.some((e) => key(e) === key(plugin))) {
          problems.push({
            id, scope: "packages", group: inst.name, severity: "error",
            message: msg("problem.enginePluginMissing", { package: inst.name, plugin }),
            facts: [fact("package", `${inst.name} ${inst.version}`), fact("plugin", plugin), fact("engine", engine()?.version ?? "—")],
            fixes: [{ label: msg("fix.ignore"), primary: true, action: { kind: "ignore" } }]
          });
        } else if (!enginePluginEnabled(p, plugin)) {
          problems.push({
            id, scope: "packages", group: inst.name, severity: "warning",
            message: msg("problem.enginePluginDisabled", { package: inst.name, plugin }),
            facts: [fact("package", `${inst.name} ${inst.version}`), fact("plugin", plugin)],
            fixes: [{ label: msg("fix.enable", { package: plugin }), primary: true, action: { kind: "changes", changes: [{ kind: "enable", package: plugin, version: null }] } }]
          });
        }
      }
    }
    for (const r of INDEX.replacements ?? []) {
      const legacy = p.installed.get(key(r.legacy));
      const replacement = p.installed.get(key(r.replacement));
      if (legacy?.enabled && replacement?.enabled) {
        const facts = [fact("legacy", `${legacy.name} ${legacy.version}`), fact("replacement", `${replacement.name} ${replacement.version}`)];
        if (r.redirects) facts.push(fact("redirects", r.redirects));
        problems.push({
          id: `legacy:${legacy.name}`, scope: "packages", group: `${p.name}.uproject`, severity: "warning",
          message: msg("problem.legacyConflict", { legacy: legacy.name, replacement: replacement.name }), facts,
          fixes: [
            { label: msg("fix.disable", { package: legacy.name }), primary: true, action: { kind: "changes", changes: [{ kind: "disable", package: legacy.name, version: null }] } },
            { label: msg("fix.ignore"), primary: false, action: { kind: "ignore" } }
          ]
        });
      }
    }
    if (p.installed.has(key("UBotROS")) && p.path === SCENARIO_PROJECT) {
      const ros = rosFor(p);
      const address = `${ros.host}:${ros.port}`;
      problems.push({
        id: "session:ros.bridge", scope: "session", group: "UBotROS", severity: "info",
        message: msg("problem.rosBridgeUnreachable", { address }),
        facts: [
          fact("source", "Saved/uBot/status.json"),
          fact("address", address),
          { label: msg("fact.time"), value: "", valueMsg: msg("session.lastPie") }
        ],
        fixes: [{ label: msg("fix.openRosSettings"), primary: false, action: { kind: "openSettings", section: "ros" } }]
      });
    }
    const hidden = ignored.get(p.path) ?? new Set<string>();
    return problems.filter((x) => !hidden.has(x.id));
  }

  function problemsOf(problems: Problem[], name: string): string[] {
    const k = key(name);
    return problems.filter((x) => {
      const parts = x.id.split(":");
      if ((parts[0] === "requires" || parts[0] === "enginePlugin") && key(parts[1] ?? "") === k) return true;
      return x.scope === "session" && key(x.group) === k;
    }).map((x) => x.id);
  }

  function packageInfos(p: ProjectState | null, problems: Problem[]): PackageInfo[] {
    const infos: PackageInfo[] = INDEX.packages.map((pkg) => {
      const inst = p?.installed.get(key(pkg.name));
      const released = releasedVersions(pkg);
      const latest = released[0];
      const planned = plannedVersions(pkg)[0];
      const shown = latest ?? planned;
      const requires = inst ? inst.requires : shown?.requires ?? [];
      const enginePlugins = inst ? inst.enginePlugins : shown?.enginePlugins ?? [];
      const updateAvailable = !!(p && inst && released.some((v) =>
        compareVersionStrings(v.version, inst.version) > 0 && compatibleVersions(p.installed.values(), pkg.name, null).includes(v.version)));
      return {
        name: pkg.name,
        friendlyName: pkg.friendlyName ?? pkg.name,
        description: { en: pkg.description.en, "zh-CN": pkg.description["zh-CN"] ?? null },
        layer: asLayer(pkg.layer),
        state: inst ? (inst.enabled ? "enabled" : "disabled") : latest ? "notInstalled" : "planned",
        installedVersion: inst?.version ?? null,
        latestVersion: latest?.version ?? null,
        versions: released.map((v) => v.version),
        plannedVersion: planned?.version ?? null,
        updateAvailable,
        location: inst?.location ?? null,
        git: inst ? { isCheckout: true, ref: `v${inst.version}`, dirty: false } : null,
        repository: pkg.repository ?? null,
        webUrl: pkg.repository ? pkg.repository.replace(/\.git$/, "") : null,
        docsUrl: pkg.docsUrl ?? null,
        indexSource: "ubot",
        requires: requirementRows(p, requires, enginePlugins),
        provides: inst ? inst.provides : shown?.provides ?? [],
        tags: pkg.tags ?? [],
        problemIds: problemsOf(problems, pkg.name)
      };
    });
    return infos.sort((a, b) => LAYER_ORDER.indexOf(a.layer) - LAYER_ORDER.indexOf(b.layer) || a.name.localeCompare(b.name));
  }

  function indexStatus(p: ProjectState | null): IndexSourceStatus[] {
    return settings.index.sources.map((s) => {
      const localMissing = s.id === "local" && !p?.installed.has(key("UBotCore"));
      const ok = s.enabled && !localMissing;
      return {
        id: s.id, name: s.name, location: s.location, builtin: s.builtin, enabled: s.enabled, ok, fromCache: false,
        error: !s.enabled ? null : localMissing ? "UBotCore is not installed in this project" : null,
        packages: ok ? (s.builtin ? INDEX.packages.length : 0) : 0,
        fetchedAt: ok ? fetchedAt : null
      };
    });
  }

  function recipes(): Recipe[] {
    const fromIndex: Recipe[] = (INDEX.recipes ?? []).map((r) => ({
      id: r.id,
      name: { en: r.name.en, "zh-CN": r.name["zh-CN"] ?? null },
      description: { en: r.description.en, "zh-CN": r.description["zh-CN"] ?? null },
      packages: r.packages,
      checks: (r.checks ?? []).filter((c) => KNOWN_CHECKS.includes(c)),
      available: r.packages.every((name) => {
        const pkg = indexPackage(name);
        return !!pkg && releasedVersions(pkg).length > 0;
      }),
      user: false
    }));
    return [...fromIndex, ...clone(userRecipes)];
  }

  function snapshot(): Snapshot {
    const p = project;
    const problems = computeProblems(p);
    const plugins: PluginInfo[] = p ? [...p.installed.values()].map((i) => ({
      name: i.name, enabled: i.enabled, location: i.location, version: i.version, isUBot: i.ubot
    })) : [];
    return {
      project: p ? {
        path: p.path, name: p.name, dir: p.dir, engineAssociation: "5.5", engine: engine(), hasCode: false,
        buildTarget: `${p.name}Editor`, editor: { running: p.editorRunning, pid: p.editorRunning ? p.editorPid : null }
      } : null,
      packages: packageInfos(p, problems),
      plugins,
      problems,
      index: indexStatus(p),
      recipes: recipes(),
      session: p && p.path === SCENARIO_PROJECT
        ? { kind: "PIE", startedAt: "2026-10-10T07:41:12Z", endedAt: "2026-10-10T07:52:40Z", engineVersion: "5.5.4" }
        : null,
      syncing
    };
  }

  const emitSnapshot = () => {
    const s = snapshot();
    snapshotListeners.forEach((l) => l(clone(s)));
  };
  const emitTask = (task: Task) => taskListeners.forEach((l) => l(clone(task)));

  // ---------------------------------------------------------------- planning

  function plan(changes: Change[], options: ApplyOptions): Plan {
    const errors: Msg[] = [];
    const warnings: Msg[] = [];
    const msg = (code: string, params: Record<string, string> = {}): Msg => ({ code, params });
    if (!project) return { changes, steps: [], needsBuild: false, editorRunning: false, errors: [msg("plan.noProject")], warnings };
    const p = project;
    // Working copy of the project so that later changes see earlier ones.
    const work = new Map<string, { name: string; version: string; enabled: boolean; ubot: boolean; requires: IndexRequire[]; enginePlugins: string[]; location: string }>();
    p.installed.forEach((v, k) => work.set(k, { ...v, requires: [...v.requires], enginePlugins: [...v.enginePlugins] }));
    const engineEnabled = new Set(p.enabledEnginePlugins);
    type Draft = Omit<PlanStep, "id" | "n" | "needsEditorClosed">;
    const downloads: Draft[] = [];
    const enginePluginSteps: Draft[] = [];
    const enables: Draft[] = [];
    const disables: Draft[] = [];
    const removes: Draft[] = [];
    const touched: string[] = [];
    const file = `${p.name}.uproject`;
    const step = (group: StepGroup, kind: StepKind, title: Msg, detail: Msg): Draft => ({ group, kind, title, detail });
    const touch = (name: string) => { if (!touched.includes(name)) touched.push(name); };

    const dependentsOf = (name: string) =>
      [...work.values()].filter((w) => w.enabled && w.requires.some((r) => key(r.name) === key(name) && !r.optional));

    const enableEnginePlugins = (owner: string, plugins: string[]) => {
      for (const plugin of plugins) {
        if (!ENGINE_PLUGINS.some((e) => key(e) === key(plugin))) {
          errors.push(msg("plan.enginePluginMissing", { plugin }));
        } else if (!engineEnabled.has(key(plugin))) {
          engineEnabled.add(key(plugin));
          enginePluginSteps.push(step("project", "enableEnginePlugin", msg("step.enableEnginePlugin", { plugin }), msg("step.detail.requiredBy", { package: owner })));
        }
      }
    };

    const resolve = (owner: string, requires: IndexRequire[], enginePlugins: string[]) => {
      for (const r of requires) {
        if (r.optional) continue;
        const dep = work.get(key(r.name));
        if (dep) {
          if (satisfies(dep.version, r.version)) {
            if (!dep.enabled) enable(dep.name, owner);
          } else {
            const target = compatibleVersions(work.values(), r.name, r.version, owner)[0];
            if (target) update(dep.name, target, owner);
            else errors.push(msg("plan.noVersion", { package: dep.name, range: r.version }));
          }
        } else {
          const pkg = indexPackage(r.name);
          if (!pkg) errors.push(msg("plan.notInIndex", { package: r.name }));
          else if (releasedVersions(pkg).length === 0) errors.push(msg("plan.planned", { package: pkg.name }));
          else {
            const target = releasedVersions(pkg).map((v) => v.version).find((v) => satisfies(v, r.version));
            if (target) install(pkg.name, target, owner);
            else errors.push(msg("plan.noVersion", { package: pkg.name, range: r.version }));
          }
        }
      }
      enableEnginePlugins(owner, enginePlugins);
    };

    function install(name: string, version: string | null, requiredBy: string | null) {
      const pkg = indexPackage(name);
      if (!pkg) { errors.push(msg("plan.notInIndex", { package: name })); return; }
      if (work.has(key(name))) { if (!requiredBy) errors.push(msg("plan.alreadyInstalled", { package: pkg.name })); return; }
      const released = releasedVersions(pkg);
      if (released.length === 0) { errors.push(msg("plan.planned", { package: pkg.name })); return; }
      const v = version ? released.find((x) => x.version === version) : released[0];
      if (!v) { errors.push(msg("plan.noVersion", { package: pkg.name, range: version ?? "*" })); return; }
      const ue = engine();
      if (!ue) errors.push(msg("plan.noEngine"));
      else if (v.engine && !satisfies(ue.version, v.engine)) {
        errors.push(msg("plan.engineVersion", { package: pkg.name, version: v.version, range: v.engine, engine: ue.version }));
      }
      const location = `${settings.install.directory}/${folderOf(pkg)}`;
      work.set(key(name), { name: pkg.name, version: v.version, enabled: false, ubot: true, requires: v.requires ?? [], enginePlugins: v.enginePlugins ?? [], location });
      resolve(pkg.name, v.requires ?? [], v.enginePlugins ?? []);
      downloads.push(step("download", "clone", msg("step.clone", { package: pkg.name, version: v.version }),
        msg("step.clone.detail", { repo: displayRepo(pkg.repository ?? ""), dest: location })));
      const entry = work.get(key(name));
      if (entry) entry.enabled = true;
      enables.push(step("project", "enablePackage", msg("step.enablePackage", { package: pkg.name }),
        requiredBy ? msg("step.detail.requiredBy", { package: requiredBy }) : msg("step.detail.project", { file })));
      touch(pkg.name);
    }

    function update(name: string, version: string, requiredBy: string | null) {
      const inst = work.get(key(name));
      if (!inst) { errors.push(msg("plan.notInstalled", { package: name })); return; }
      const pkg = indexPackage(name);
      const v = pkg?.versions.find((x) => x.version === version && !x.planned);
      if (!pkg || !v) { errors.push(msg("plan.noVersion", { package: inst.name, range: version })); return; }
      if (inst.version === version) return;
      for (const dependent of dependentsOf(name)) {
        if (key(dependent.name) === key(requiredBy ?? "")) continue;
        const r = dependent.requires.find((x) => key(x.name) === key(name));
        if (r && !satisfies(version, r.version)) {
          errors.push(msg("plan.breaksDependent", { package: inst.name, dependent: dependent.name, range: r.version }));
        }
      }
      const from = inst.version;
      inst.version = version;
      inst.requires = v.requires ?? [];
      inst.enginePlugins = v.enginePlugins ?? [];
      resolve(inst.name, inst.requires, inst.enginePlugins);
      downloads.push(step("download", "checkout", msg("step.checkout", { package: inst.name, version }),
        msg("step.checkout.detail", { from: `v${from}`, to: v.ref ?? `v${version}` })));
      touch(inst.name);
    }

    function enable(name: string, requiredBy: string | null) {
      const inst = work.get(key(name));
      if (!inst) {
        if (ENGINE_PLUGINS.some((e) => key(e) === key(name))) {
          if (!engineEnabled.has(key(name))) {
            engineEnabled.add(key(name));
            enginePluginSteps.push(step("project", "enableEnginePlugin", msg("step.enableEnginePlugin", { plugin: name }), msg("step.detail.project", { file })));
          }
          return;
        }
        errors.push(msg("plan.notInstalled", { package: name }));
        return;
      }
      if (inst.enabled) return;
      inst.enabled = true;
      if (inst.ubot) resolve(inst.name, inst.requires, inst.enginePlugins);
      enables.push(step("project", "enablePackage", msg("step.enablePackage", { package: inst.name }),
        requiredBy ? msg("step.detail.requiredBy", { package: requiredBy }) : msg("step.detail.project", { file })));
      touch(inst.name);
    }

    function disableWithDependents(name: string, includeSelf: boolean) {
      const inst = work.get(key(name));
      if (!inst) { errors.push(msg("plan.notInstalled", { package: name })); return; }
      const order: string[] = [];
      const visit = (n: string) => {
        for (const d of dependentsOf(n)) {
          if (order.includes(d.name)) continue;
          visit(d.name);
          order.push(d.name);
        }
      };
      visit(inst.name);
      if (order.length > 0) warnings.push(msg("plan.alsoDisable", { package: inst.name, dependents: order.join(", ") }));
      const all = includeSelf && inst.enabled ? [...order, inst.name] : order;
      for (const n of all) {
        const w = work.get(key(n));
        if (!w || !w.enabled) continue;
        w.enabled = false;
        disables.push(step("project", "disablePackage", msg("step.disablePackage", { package: w.name }), msg("step.detail.project", { file })));
      }
    }

    for (const c of changes) {
      switch (c.kind) {
        case "install": install(c.package, c.version, null); break;
        case "update": {
          if (!work.has(key(c.package))) errors.push(msg("plan.notInstalled", { package: c.package }));
          else if (c.version) update(c.package, c.version, null);
          break;
        }
        case "enable": enable(c.package, null); break;
        case "disable": disableWithDependents(c.package, true); break;
        case "uninstall": {
          const inst = work.get(key(c.package));
          if (!inst) { errors.push(msg("plan.notInstalled", { package: c.package })); break; }
          disableWithDependents(c.package, false);
          work.delete(key(c.package));
          removes.push(step("project", "removePackage", msg("step.removePackage", { package: inst.name }), msg("step.removePackage.detail", { dest: inst.location })));
          break;
        }
      }
    }

    const drafts: Draft[] = [...downloads, ...enginePluginSteps, ...enables, ...disables, ...removes];
    const needsBuild = options.build && drafts.some((d) => d.kind === "clone" || d.kind === "checkout" || d.kind === "enablePackage" || d.kind === "enableEnginePlugin");
    if (needsBuild) {
      drafts.push(step("build", "build", msg("step.build", { target: `${p.name}Editor` }),
        msg("step.build.detail", { platform: "Win64", configuration: settings.build.configuration })));
    }
    if (options.runTests && needsBuild && touched.length > 0) {
      drafts.push(step("after", "runTests", msg("step.runTests"), msg("step.runTests.detail", { filter: touched.join("+") })));
    }
    if (options.reopenEditor && drafts.length > 0 && (p.editorRunning || needsBuild)) {
      drafts.push(step("after", "openEditor", msg("step.openEditor"), msg("step.openEditor.detail", { project: file })));
    }
    const steps: PlanStep[] = drafts.map((d, i) => ({ ...d, id: `s${i + 1}`, n: i + 1, needsEditorClosed: d.kind !== "clone" }));
    return { changes, steps, needsBuild, editorRunning: p.editorRunning, errors, warnings };
  }

  // ---------------------------------------------------------------- execution

  function pushLog(task: Task, stepId: string, lines: string[]) {
    const id = `${task.id}/${stepId}`;
    const log = logs.get(id) ?? [];
    log.push(...lines);
    logs.set(id, log);
    const s = task.steps.find((x) => x.id === stepId);
    if (s) s.logLines = log.length;
    logListeners.forEach((l) => l({ taskId: task.id, stepId, lines: [...lines] }));
  }

  function buildActions(packages: string[]): string[] {
    const actions: string[] = [];
    for (const name of packages) {
      const sources = name === "UBotVehicles"
        ? ["Module.UBotVehicles.cpp", "UBotWheeledVehicleComponent.cpp", "UBotDifferentialDriveController.cpp", "UBotAckermannController.cpp",
          "UBotVehicleMovementComponent.cpp", "UBotVehicleSettings.cpp", "UBotVehicleDescriptor.cpp", "UBotTireModel.cpp",
          "UBotSteeringModel.cpp", "UBotWheelOdometry.cpp", "UBotVehicleSensorMount.cpp", "UBotVehicleCommands.cpp",
          "UBotVehicleBlueprintLibrary.cpp", "UBotVehiclesTests.cpp"]
        : [`Module.${name}.cpp`, `${name}Module.cpp`, `${name}Subsystem.cpp`, `${name}Settings.cpp`, `${name}BlueprintLibrary.cpp`, `${name}Tests.cpp`];
      sources.forEach((s) => actions.push(`Compile [x64] ${s}`));
      actions.push(`Link [x64] UnrealEditor-${name}.dll`);
      if (name === "UBotVehicles") {
        ["Module.UBotVehiclesEditor.cpp", "UBotVehiclesEditorModule.cpp", "UBotVehicleDetailsCustomization.cpp", "UBotVehicleFactory.cpp",
          "UBotVehicleAssetTypeActions.cpp", "UBotVehicleThumbnailRenderer.cpp", "UBotVehiclesEditorStyle.cpp"]
          .forEach((s) => actions.push(`Compile [x64] ${s}`));
        actions.push("Link [x64] UnrealEditor-UBotVehiclesEditor.dll");
      }
    }
    actions.push(`WriteMetadata ${project?.name ?? "Project"}Editor.target`);
    return actions;
  }

  function buildHeader(): string[] {
    const p = project;
    return [
      `Build.bat ${p?.name}Editor Win64 ${settings.build.configuration} -Project="${p?.path}" -WaitMutex`,
      "Using bundled DotNet SDK",
      `Building ${p?.name}Editor...`,
      "Using Visual Studio 2022 toolchain",
      "Determining max actions to execute in parallel"
    ];
  }

  function stepLines(planStep: PlanStep): string[] {
    const p = project;
    const params = planStep.title.params;
    const name = params["package"] ?? params["plugin"] ?? "";
    switch (planStep.kind) {
      case "clone": {
        const pkg = indexPackage(name);
        const dest = `${p?.dir}\\${settings.install.directory.replace(/\//g, "\\")}\\${pkg ? folderOf(pkg) : name}`;
        return [
          `git clone --branch v${params["version"]} -- ${pkg?.repository ?? ""} ${dest}`,
          `Cloning into '${dest}'...`,
          "remote: Enumerating objects: 212, done.",
          "remote: Counting objects: 100% (212/212), done.",
          "Receiving objects: 100% (212/212), 1.21 MiB | 4.30 MiB/s, done.",
          "Resolving deltas: 100% (96/96), done.",
          `Note: switching to '${planStep.title.params["version"] ? `v${params["version"]}` : ""}'.`
        ];
      }
      case "checkout": {
        const inst = p?.installed.get(key(name));
        const dir = `${p?.dir}\\${(inst?.location ?? "").replace(/\//g, "\\")}`;
        return [
          `git -C ${dir} fetch --tags origin`,
          `From ${indexPackage(name)?.repository ?? ""}`,
          ` * [new tag]         v${params["version"]} -> v${params["version"]}`,
          `git -C ${dir} checkout --detach v${params["version"]}`,
          `HEAD is now at 4f2c1a9 Release v${params["version"]}`
        ];
      }
      case "enablePackage":
      case "enableEnginePlugin":
        return [`${p?.name}.uproject: Plugins += { "Name": "${name}", "Enabled": true }`];
      case "disablePackage":
        return [`${p?.name}.uproject: { "Name": "${name}", "Enabled": false }`];
      case "removePackage":
        return [`${p?.name}.uproject: Plugins -= "${name}"`, `Moved ${planStep.detail.params["dest"]} to the Recycle Bin`];
      case "runTests":
        return [
          `UnrealEditor-Cmd.exe "${p?.path}" -unattended -nop4 -nosplash -NullRHI -ExecCmds="Automation RunTests ${planStep.detail.params["filter"]}; Quit" -TestExit="Automation Test Queue Empty"`,
          "LogAutomationController: Display: Found 18 automation tests based on filter",
          "LogAutomationController: Display: Test Completed. Result={Success}",
          "LogAutomationController: Display: **** TEST COMPLETE. EXIT CODE: 0 ****"
        ];
      case "openEditor":
        return [`Started UnrealEditor.exe "${p?.path}" (PID 23512)`];
      case "build":
        return [];
    }
  }

  function mutate(planStep: PlanStep) {
    const p = project;
    if (!p) return;
    const params = planStep.title.params;
    const name = params["package"] ?? params["plugin"] ?? "";
    switch (planStep.kind) {
      case "clone": {
        const pkg = indexPackage(name);
        const v = pkg?.versions.find((x) => x.version === params["version"]);
        if (pkg && v) {
          p.installed.set(key(name), {
            name: pkg.name, version: v.version, enabled: false, ubot: true,
            location: `${settings.install.directory}/${folderOf(pkg)}`,
            requires: clone(v.requires ?? []), enginePlugins: clone(v.enginePlugins ?? []), provides: clone(v.provides ?? [])
          });
        }
        break;
      }
      case "checkout": {
        const inst = p.installed.get(key(name));
        const v = indexPackage(name)?.versions.find((x) => x.version === params["version"]);
        if (inst && v) {
          inst.version = v.version;
          inst.requires = clone(v.requires ?? []);
          inst.enginePlugins = clone(v.enginePlugins ?? []);
          inst.provides = clone(v.provides ?? []);
        }
        break;
      }
      case "enablePackage": {
        const inst = p.installed.get(key(name));
        if (inst) inst.enabled = true;
        break;
      }
      case "enableEnginePlugin":
        p.enabledEnginePlugins.add(key(name));
        break;
      case "disablePackage": {
        const inst = p.installed.get(key(name));
        if (inst) inst.enabled = false;
        break;
      }
      case "removePackage":
        p.installed.delete(key(name));
        break;
      case "openEditor":
        p.editorRunning = true;
        p.editorPid = 23512;
        break;
      default:
        break;
    }
  }

  async function runStep(run: RunState, planStep: PlanStep, taskStep: TaskStep): Promise<void> {
    const task = run.task;
    if (planStep.kind === "build") {
      const actions = buildActions(task.changes.length > 0 ? touchedPackages(run.plan) : []);
      const already = taskStep.progress?.done ?? 0;
      if (already === 0) {
        for (const line of buildHeader()) {
          if (run.canceled) return;
          pushLog(task, taskStep.id, [line]);
          await delay(120);
        }
      }
      taskStep.progress = { done: already, total: actions.length };
      emitTask(task);
      for (let i = already; i < actions.length; i++) {
        await delay(run.buildLineMs);
        if (run.canceled) return;
        pushLog(task, taskStep.id, [`[${i + 1}/${actions.length}] ${actions[i]}`]);
        taskStep.progress = { done: i + 1, total: actions.length };
        emitTask(task);
      }
      pushLog(task, taskStep.id, ["Total time in Unreal Build Accelerator local executor: 61.42 seconds", "Result: Succeeded"]);
      return;
    }
    const lines = stepLines(planStep);
    const pause = planStep.kind === "clone" || planStep.kind === "checkout" ? 260 : planStep.kind === "runTests" ? 600 : 120;
    for (const line of lines) {
      await delay(pause);
      if (run.canceled) return;
      pushLog(task, taskStep.id, [line]);
    }
  }

  function touchedPackages(p: Plan): string[] {
    return p.steps.filter((s) => s.kind === "clone" || s.kind === "checkout" || s.kind === "enablePackage")
      .map((s) => s.title.params["package"] ?? "").filter((n, i, all) => n && all.indexOf(n) === i && indexPackage(n));
  }

  async function runTask(run: RunState, startAt = 0): Promise<void> {
    const task = run.task;
    task.status = "running";
    emitTask(task);
    let failed = false;
    for (let i = startAt; i < run.plan.steps.length; i++) {
      const planStep = run.plan.steps[i];
      const taskStep = task.steps[i];
      if (!planStep || !taskStep || run.canceled) break;
      if (planStep.needsEditorClosed && project?.editorRunning) {
        if (!settings.build.waitForEditor) {
          taskStep.status = "failed";
          task.error = "Unreal Editor is running this project.";
          failed = true;
          emitTask(task);
          break;
        }
        taskStep.status = "waiting";
        task.status = "waiting";
        emitTask(task);
        pushLog(task, taskStep.id, [`Waiting for UnrealEditor.exe (PID ${project.editorPid}) to exit...`]);
        let waited = 0;
        while (project?.editorRunning && !run.canceled) {
          await delay(1000);
          // Simulates the user closing the editor when nobody presses "Close Editor".
          if (++waited === 3 && project) {
            project.editorRunning = false;
            emitSnapshot();
          }
        }
        if (run.canceled) break;
        task.status = "running";
      }
      taskStep.status = "running";
      taskStep.startedAt ??= nowIso();
      const started = Date.parse(taskStep.startedAt);
      emitTask(task);
      await runStep(run, planStep, taskStep);
      if (run.canceled) break;
      mutate(planStep);
      taskStep.status = "done";
      taskStep.durationMs = Date.now() - started;
      emitTask(task);
    }
    if (run.canceled) {
      for (const s of task.steps) {
        if (s.status === "running" || s.status === "waiting") s.status = "canceled";
        else if (s.status === "pending") s.status = "skipped";
      }
      task.status = "canceled";
    } else if (failed) {
      for (const s of task.steps) if (s.status === "pending") s.status = "skipped";
      task.status = "failed";
    } else {
      task.status = "done";
      if (run.saveRecipe && project) {
        const enabled = [...project.installed.values()].filter((i) => i.ubot && i.enabled).map((i) => i.name);
        userRecipes.push({
          id: `user-${slug(run.saveRecipe)}`, name: { en: run.saveRecipe, "zh-CN": null },
          description: { en: enabled.join(", "), "zh-CN": null }, packages: enabled, checks: [], available: true, user: true
        });
      }
    }
    task.durationMs = Date.now() - Date.parse(task.startedAt);
    running = null;
    emitTask(task);
    emitSnapshot();
  }

  function startTask(planned: Plan, options: ApplyOptions, title: Msg | null, saveRecipe: string | null, buildLineMs: number): RunState {
    const task: Task = {
      id: `task-${++taskCounter}`,
      title: title ?? { code: "task.apply", params: { project: project?.name ?? "" } },
      // The project name (the .uproject stem), like the core: it is what the Tasks view compares with the open project.
      project: project?.name ?? "",
      status: "pending",
      startedAt: nowIso(),
      durationMs: null,
      steps: planned.steps.map((s) => ({ id: s.id, title: s.title, status: "pending", startedAt: null, durationMs: null, progress: null, logLines: 0 })),
      error: null,
      changes: clone(planned.changes)
    };
    tasks.unshift(task);
    if (tasks.length > 30) tasks.length = 30;
    const run: RunState = { task, plan: planned, options, saveRecipe, canceled: false, buildLineMs };
    running = run;
    return run;
  }

  // History: a finished task from an earlier session.
  (function seedHistory() {
    const started = Date.parse("2026-10-09T15:20:04Z");
    const done = (id: string, code: string, params: Record<string, string>, offset: number, duration: number): TaskStep => ({
      id, title: { code, params }, status: "done", startedAt: new Date(started + offset).toISOString(), durationMs: duration, progress: null, logLines: 0
    });
    const task: Task = {
      id: "task-0",
      title: { code: "task.install", params: { package: "UBotROS", version: "0.1.0" } },
      project: "MyRobotSim", status: "done", startedAt: new Date(started).toISOString(), durationMs: 112_000,
      steps: [
        done("s1", "step.clone", { package: "UBotROS", version: "0.1.0" }, 0, 5_200),
        done("s2", "step.enablePackage", { package: "UBotROS" }, 5_300, 200),
        done("s3", "step.build", { target: "MyRobotSimEditor" }, 5_600, 106_000)
      ],
      error: null,
      changes: [{ kind: "install", package: "UBotROS", version: "0.1.0" }]
    };
    tasks.push(task);
    logs.set("task-0/s1", [
      "git clone --branch v0.1.0 -- https://github.com/AstreoX/uBotROS.git D:\\Projects\\MyRobotSim\\Plugins\\uBot\\uBotROS",
      "Cloning into 'D:\\Projects\\MyRobotSim\\Plugins\\uBot\\uBotROS'...",
      "Receiving objects: 100% (348/348), 2.04 MiB | 5.10 MiB/s, done.",
      "Resolving deltas: 100% (171/171), done."
    ]);
    logs.set("task-0/s2", ["MyRobotSim.uproject: Plugins += { \"Name\": \"UBotROS\", \"Enabled\": true }"]);
    logs.set("task-0/s3", [
      `Build.bat MyRobotSimEditor Win64 Development -Project="${SCENARIO_PROJECT}" -WaitMutex -NoHotReloadFromIDE`,
      "Building MyRobotSimEditor...",
      "[1/31] Compile [x64] Module.UBotROS.cpp",
      "[31/31] WriteMetadata MyRobotSimEditor.target",
      "Result: Succeeded"
    ]);
    task.steps.forEach((s) => { s.logLines = logs.get(`task-0/${s.id}`)?.length ?? 0; });
  })();

  // Design screenshot of the Tasks view: the UBotVehicles install is building, 15 of 24 actions done.
  if (params.view === "tasks" && project) {
    const options: ApplyOptions = { build: true, reopenEditor: true, runTests: false };
    const planned = plan([{ kind: "install", package: "UBotVehicles", version: "0.1.0" }], options);
    project.editorRunning = false;
    const run = startTask(planned, options, null, null, 4000);
    const task = run.task;
    const begin = Date.now() - 72_000;
    task.startedAt = new Date(begin).toISOString();
    let offset = 0;
    const durations = [4_100, 300, 200];
    planned.steps.slice(0, 3).forEach((s, i) => {
      const ts = task.steps[i];
      if (!ts) return;
      ts.status = "done";
      ts.startedAt = new Date(begin + offset).toISOString();
      ts.durationMs = durations[i] ?? 200;
      offset += ts.durationMs;
      stepLines(s).forEach((line) => pushLog(task, ts.id, [line]));
      mutate(s);
    });
    const buildStep = task.steps[3];
    if (buildStep) {
      buildStep.status = "running";
      buildStep.startedAt = new Date(Date.now() - 65_000).toISOString();
      const actions = buildActions(["UBotVehicles"]);
      pushLog(task, buildStep.id, buildHeader());
      actions.slice(0, 15).forEach((a, i) => pushLog(task, buildStep.id, [`[${i + 1}/${actions.length}] ${a}`]));
      buildStep.progress = { done: 15, total: actions.length };
    }
    task.status = "running";
    void runTask(run, 3);
  }

  // ---------------------------------------------------------------- backend

  const backend: Backend = {
    async getSnapshot() {
      await delay(30);
      return clone(snapshot());
    },
    async refresh(sync) {
      if (sync) {
        syncing = true;
        emitSnapshot();
        await delay(900);
        fetchedAt = nowIso();
        syncing = false;
      } else {
        await delay(120);
      }
      const s = snapshot();
      emitSnapshot();
      return clone(s);
    },
    async openProject(path) {
      await delay(200);
      // A task works on the project it was planned for; the core refuses to switch meanwhile.
      if (running) throw new Error("Another task is running.");
      const trimmed = path.trim();
      if (!/\.uproject$/i.test(trimmed)) throw new Error(`Not a .uproject file: ${trimmed}`);
      project = key(trimmed) === key(SCENARIO_PROJECT) ? (project?.path === SCENARIO_PROJECT ? project : scenarioProject()) : emptyProject(trimmed);
      rememberProject(trimmed);
      // Like the shell: open_project answers with the snapshot and emits no event.
      return clone(snapshot());
    },
    async plan(changes, options) {
      await delay(150);
      return clone(plan(changes, options));
    },
    async apply(changes, options, title, saveRecipe) {
      await delay(80);
      // The core rejects with the English text of the message, which the UI translates.
      if (running) throw new Error("Another task is running.");
      const planned = plan(changes, options);
      // One English message per line, like the core; the UI translates each line.
      if (planned.errors.length > 0) throw new Error(planned.errors.map((m) => renderMsg("en", m)).join("\n"));
      const run = startTask(planned, options, title, saveRecipe, 320);
      emitTask(run.task);
      void runTask(run);
      return run.task.id;
    },
    async cancelTask(taskId) {
      if (running && running.task.id === taskId) running.canceled = true;
    },
    async listTasks() {
      return clone(tasks);
    },
    async getTaskLog(taskId, stepId) {
      if (stepId !== null) return [...(logs.get(`${taskId}/${stepId}`) ?? [])];
      const task = tasks.find((t) => t.id === taskId);
      if (!task) return [];
      const lines = [`Project ${task.project}`, `Started ${task.startedAt}`];
      if (task.error) lines.push(`Error: ${task.error}`);
      return lines;
    },
    async closeEditor() {
      await delay(300);
      if (project) {
        project.editorRunning = false;
        emitSnapshot();
      }
    },
    async ignoreProblem(id) {
      if (project) {
        const set = ignored.get(project.path) ?? new Set<string>();
        set.add(id);
        ignored.set(project.path, set);
      }
      const s = snapshot();
      emitSnapshot();
      return clone(s);
    },
    async getSettings() {
      return { ...clone(settings), ros: rosFor(project) };
    },
    async saveSettings(next) {
      await delay(80);
      // Recent projects and the last project belong to openProject; the dialog does not edit them.
      const stored = normalizeSettings(clone(next));
      stored.recentProjects = settings.recentProjects;
      stored.lastProject = settings.lastProject;
      // Only a changed host or port is written to the project's ini.
      const current = rosFor(project);
      if (project && (stored.ros.host !== current.host || stored.ros.port !== current.port)) {
        projectRos.set(key(project.path), clone(stored.ros));
      }
      stored.ros = project ? settings.ros : stored.ros;
      Object.assign(settings, stored);
      const s = snapshot();
      emitSnapshot();
      return clone(s);
    },
    async testGit(path): Promise<GitTestResult> {
      await delay(250);
      return path.trim()
        ? { ok: true, version: "git version 2.49.0.windows.1", error: null }
        : { ok: false, version: null, error: "No git executable given" };
    },
    async testRos(host, port): Promise<RosTestResult> {
      await delay(600);
      return { ok: false, error: `Connection to ${host}:${port} refused` };
    },
    async listEngines() {
      return clone(ENGINES);
    },
    async checkEnvironment(checks): Promise<EnvCheck[]> {
      await delay(200);
      return checks.filter((c): c is EnvCheck["id"] => KNOWN_CHECKS.includes(c)).map((id) => {
        switch (id) {
          case "git": return { id, ok: true, detail: "git version 2.49.0.windows.1" };
          case "vs2022": return { id, ok: true, detail: "Visual Studio Community 2022 17.10" };
          case "engine": return { id, ok: engine() !== null, detail: engine()?.root ?? null };
          case "rosBridge": return { id, ok: null, detail: null };
        }
      });
    },
    async recipeChanges(recipeId, packages) {
      await delay(100);
      const recipe = recipeId ? recipes().find((r) => r.id === recipeId) : null;
      const names = packages.length > 0 ? packages : recipe?.packages ?? [];
      const changes: Change[] = [];
      for (const name of names) {
        const pkg = indexPackage(name);
        const inst = project?.installed.get(key(name));
        if (!inst) changes.push({ kind: "install", package: pkg?.name ?? name, version: null });
        else if (!inst.enabled) changes.push({ kind: "enable", package: inst.name, version: null });
      }
      return changes;
    },
    onSnapshot(handler) {
      snapshotListeners.add(handler);
      return () => snapshotListeners.delete(handler);
    },
    onTask(handler) {
      taskListeners.add(handler);
      return () => taskListeners.delete(handler);
    },
    onTaskLog(handler) {
      logListeners.add(handler);
      return () => logListeners.delete(handler);
    }
  };

  // Screenshots and checks: what a second `ubot-manager.exe --project` launch does to the running window.
  if (typeof window !== "undefined") {
    (window as unknown as { ubotMock?: object }).ubotMock = {
      /** Switches the open project the way the shell does for a second instance: update, then emit ubot://snapshot. */
      openProjectExternally(path: string) {
        project = key(path) === key(SCENARIO_PROJECT) ? scenarioProject() : emptyProject(path);
        rememberProject(path);
        emitSnapshot();
      }
    };
  }

  const initialChanges: Change[] = project && params.view !== "tasks"
    ? [{ kind: "install", package: "UBotVehicles", version: "0.1.0" }]
    : [];
  return { backend, initialChanges };
}
