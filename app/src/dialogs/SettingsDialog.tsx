import { useEffect, useMemo, useRef, useState, type CSSProperties } from "react";
import { api, platform } from "../api";
import type { EngineInfo, IndexSourceConfig, Settings, SettingsSection } from "../api/types";
import { sourceName } from "../components/Chrome";
import { Dialog } from "../components/Dialog";
import { ArrowDownIcon, ArrowUpIcon, SearchIcon } from "../components/Icons";
import { getLanguage, t } from "../i18n";
import { parsePort, rebaseSettings, sameJson } from "../lib/settings";
import { applyTheme, useApp } from "../state";

const SECTIONS: SettingsSection[] = ["appearance", "index", "install", "build", "ros"];

/** Labels searched by the settings filter, per section. */
const SECTION_TERMS: Record<SettingsSection, string[]> = {
  appearance: ["settings.theme", "settings.language", "theme.dark", "theme.light", "theme.system"],
  index: ["settings.sources", "settings.syncOnStart", "settings.showPlanned", "settings.name", "settings.location"],
  install: ["settings.installDir", "settings.git", "settings.usePrebuilt"],
  build: ["settings.engine", "settings.configuration", "settings.waitForEditor", "settings.reopenEditor", "settings.runTests"],
  ros: ["settings.host", "settings.port", "settings.testConnection", "settings.checkOnRefresh"]
};

const HELP_URL = "https://github.com/AstreoX/uBotManager";

type TestState = { kind: "idle" } | { kind: "running" } | { kind: "ok"; text: string } | { kind: "failed"; text: string };

function TestResult({ state }: { state: TestState }) {
  if (state.kind === "idle") return null;
  if (state.kind === "running") return <span className="result c-muted">{t("settings.testing")}</span>;
  return <span className={`result ${state.kind === "ok" ? "c-success" : "c-warning"}`}>{state.text}</span>;
}

export function SettingsDialog(props: { section: SettingsSection }) {
  const { state, actions } = useApp();
  const saved = useRef<Settings>(state.settings);
  const [draft, setDraft] = useState<Settings>(() => structuredClone(state.settings));
  const [section, setSection] = useState<SettingsSection>(props.section);
  const [query, setQuery] = useState("");
  const [engines, setEngines] = useState<EngineInfo[]>([]);
  const [selectedSource, setSelectedSource] = useState<number | null>(null);
  const [portText, setPortText] = useState(String(state.settings.ros.port));
  const port = parsePort(portText);
  const portValid = port !== null;
  const [gitTest, setGitTest] = useState<TestState>({ kind: "idle" });
  const [rosTest, setRosTest] = useState<TestState>({ kind: "idle" });
  const tested = useRef(new Set<SettingsSection>());
  const runningLanguage = getLanguage();

  const dirty = !sameJson(draft, saved.current);
  const set = (patch: (s: Settings) => Settings) => setDraft((d) => patch(structuredClone(d)));

  useEffect(() => {
    api.listEngines().then(setEngines, actions.report);
  }, [actions.report]);

  // The backend changes settings behind the dialog: a second launch switches the project (recent projects, the
  // project's ROS host and port), UE edits Config/DefaultUBotROS.ini. Read them when the dialog opens, and
  // carry the draft over to whatever arrives while it is open: edited values stay, the rest follows.
  const { refreshSettings } = actions;
  useEffect(() => {
    void refreshSettings();
  }, [refreshSettings]);
  useEffect(() => {
    const fresh = state.settings;
    const base = saved.current;
    if (fresh === base) return;
    saved.current = fresh;
    if (!sameJson(fresh, base)) setDraft((d) => rebaseSettings(d, base, fresh));
  }, [state.settings]);

  // The port field follows the draft when something else changes it (rebase, save); typing keeps its text.
  useEffect(() => {
    setPortText((text) => (parsePort(text) === draft.ros.port ? text : String(draft.ros.port)));
  }, [draft.ros.port]);

  // Theme previews immediately; Cancel restores the saved one.
  useEffect(() => {
    applyTheme(draft.theme);
  }, [draft.theme]);

  const testGit = () => {
    setGitTest({ kind: "running" });
    api.testGit(draft.install.git).then(
      (r) => setGitTest(r.ok ? { kind: "ok", text: r.version ?? "" } : { kind: "failed", text: r.error ?? "" }),
      (e) => setGitTest({ kind: "failed", text: String(e) }));
  };
  const testRos = () => {
    setRosTest({ kind: "running" });
    api.testRos(draft.ros.host, draft.ros.port).then(
      (r) => setRosTest(r.ok ? { kind: "ok", text: t("settings.reachable") } : { kind: "failed", text: t("settings.notReachable") }),
      () => setRosTest({ kind: "failed", text: t("settings.notReachable") }));
  };

  useEffect(() => {
    if (tested.current.has(section)) return;
    tested.current.add(section);
    if (section === "install") testGit();
    if (section === "ros") testRos();
    // Only the first visit of a section runs its test automatically.
  }, [section]);

  const q = query.trim().toLowerCase();
  const visibleSections = useMemo(() => SECTIONS.filter((s) =>
    !q || [`settings.${s}`, ...SECTION_TERMS[s]].some((k) => t(k).toLowerCase().includes(q))), [q]);
  const shown: SettingsSection | undefined = visibleSections.includes(section) ? section : visibleSections[0];

  const close = () => {
    applyTheme(saved.current.theme);
    actions.closeDialog();
  };
  const save = async (): Promise<boolean> => {
    const next = structuredClone(draft);
    // Recent projects and the last project are not edited here, and the ROS host and port come from the project:
    // send the backend's current values, not the ones the dialog opened with, unless ROS was edited here.
    const fresh = await actions.refreshSettings();
    if (fresh) {
      next.recentProjects = fresh.recentProjects;
      next.lastProject = fresh.lastProject;
      if (sameJson(next.ros, saved.current.ros)) next.ros = fresh.ros;
    }
    const stored = await actions.saveSettings(next);
    if (!stored) return false;
    // Show what the backend kept (blank sources are dropped, values trimmed), not the draft.
    saved.current = stored;
    setDraft(structuredClone(stored));
    setSelectedSource(null);
    return true;
  };

  const sources = draft.index.sources;
  const selected = selectedSource !== null ? sources[selectedSource] : undefined;
  const updateSource = (index: number, patch: Partial<IndexSourceConfig>) =>
    set((s) => {
      const src = s.index.sources[index];
      if (src) Object.assign(src, patch);
      return s;
    });
  const moveSource = (delta: number) => {
    if (selectedSource === null) return;
    const target = selectedSource + delta;
    if (target < 0 || target >= sources.length) return;
    set((s) => {
      const list = s.index.sources;
      const [item] = list.splice(selectedSource, 1);
      if (item) list.splice(target, 0, item);
      return s;
    });
    setSelectedSource(target);
  };
  const addSource = () => {
    set((s) => {
      s.index.sources.push({ id: `user-${Date.now().toString(36)}`, name: t("settings.newSource"), location: "", enabled: true, builtin: false });
      return s;
    });
    setSelectedSource(sources.length);
  };
  const removeSource = () => {
    if (selectedSource === null || !selected || selected.builtin) return;
    set((s) => {
      s.index.sources.splice(selectedSource, 1);
      return s;
    });
    setSelectedSource(null);
  };

  const engineValue = draft.build.engineId ?? state.snapshot.project?.engine?.id ?? engines[0]?.id ?? "";
  const gridStyle = { "--label-width": runningLanguage === "zh-CN" ? "120px" : "150px" } as CSSProperties;

  return (
    <Dialog className="settings-dialog" title={t("settings.title")} onClose={close}>
      <div className="dialog-split">
        <nav className="side-nav" aria-label={t("settings.sections")}>
          <label className="search-box">
            <SearchIcon size={14} />
            <span className="sr-only">{t("settings.search")}</span>
            <input type="search" value={query} placeholder={t("common.search")} onChange={(e) => setQuery(e.target.value)} />
          </label>
          {visibleSections.map((s) => (
            <button key={s} type="button" className={`side-item${shown === s ? " selected" : ""}`} onClick={() => setSection(s)}>
              <span className="label">{t(`settings.${s}`)}</span>
            </button>
          ))}
        </nav>

        <div className="settings-page">
          {shown && <div className="page-title">{t(`settings.${shown}`)}</div>}

          {shown === "appearance" && (
            <div className="form-grid" style={gridStyle}>
              <span className="label">{t("settings.theme")}</span>
              <div role="radiogroup" aria-label={t("settings.theme")} className="radio-row">
                {(["dark", "light", "system"] as const).map((theme) => (
                  <label key={theme}>
                    <input type="radio" name="theme" checked={draft.theme === theme} onChange={() => set((s) => ({ ...s, theme }))} />
                    {t(`theme.${theme}`)}
                  </label>
                ))}
              </div>
              <label htmlFor="settings-language" className="label">{t("settings.language")}</label>
              <div className="inline-row wide">
                <select
                  id="settings-language"
                  className="select"
                  style={{ width: 180 }}
                  value={draft.language}
                  onChange={(e) => {
                    const language = e.target.value === "zh-CN" ? "zh-CN" : "en";
                    set((s) => ({ ...s, language }));
                  }}
                >
                  <option value="en">{t("settings.languageEn")}</option>
                  <option value="zh-CN">{t("settings.languageZh")}</option>
                </select>
                {draft.language !== runningLanguage && <span className="result c-muted">{t("settings.restartNote")}</span>}
              </div>
            </div>
          )}

          {shown === "index" && (
            <div className="stack" style={{ gap: 14 }}>
              <div className="sources">
                <span className="label">{t("settings.sources")}</span>
                <div className="table">
                  <div className="table-toolbar">
                    <button type="button" aria-label={t("settings.addSource")} title={t("settings.addSource")} onClick={addSource}>+</button>
                    <button type="button" aria-label={t("settings.removeSource")} title={t("settings.removeSource")} disabled={!selected || selected.builtin} onClick={removeSource}>−</button>
                    <button type="button" aria-label={t("settings.moveUp")} title={t("settings.moveUp")} disabled={selectedSource === null || selectedSource === 0} onClick={() => moveSource(-1)}>
                      <ArrowUpIcon size={12} strokeWidth={1.5} />
                    </button>
                    <button type="button" aria-label={t("settings.moveDown")} title={t("settings.moveDown")} disabled={selectedSource === null || selectedSource === sources.length - 1} onClick={() => moveSource(1)}>
                      <ArrowDownIcon size={12} />
                    </button>
                  </div>
                  <div className="table-head"><span /><span>{t("settings.name")}</span><span>{t("settings.location")}</span></div>
                  {sources.map((src, i) => {
                    const editing = selectedSource === i && !src.builtin;
                    const status = state.snapshot.index.find((x) => x.id === src.id);
                    return (
                      <div
                        key={src.id}
                        className={`table-row${selectedSource === i ? " selected" : ""}`}
                        onClick={() => setSelectedSource(i)}
                        title={status && !status.ok && status.error ? status.error : undefined}
                      >
                        <input
                          type="checkbox"
                          checked={src.builtin || src.enabled}
                          disabled={src.builtin}
                          aria-label={sourceName(src)}
                          onChange={() => updateSource(i, { enabled: !src.enabled })}
                        />
                        {editing ? (
                          <input className="cell-input" value={src.name} aria-label={t("settings.name")} onChange={(e) => updateSource(i, { name: e.target.value })} />
                        ) : <span>{sourceName(src)}</span>}
                        {editing ? (
                          <input className="cell-input mono" value={src.location} aria-label={t("settings.location")} onChange={(e) => updateSource(i, { location: e.target.value })} />
                        ) : <span className="loc">{src.location}</span>}
                      </div>
                    );
                  })}
                </div>
              </div>
              <label className="check-label">
                <input type="checkbox" checked={draft.index.syncOnStart} onChange={() => set((s) => { s.index.syncOnStart = !s.index.syncOnStart; return s; })} />
                {t("settings.syncOnStart")}
              </label>
              <label className="check-label">
                <input type="checkbox" checked={draft.index.showPlanned} onChange={() => set((s) => { s.index.showPlanned = !s.index.showPlanned; return s; })} />
                {t("settings.showPlanned")}
              </label>
            </div>
          )}

          {shown === "install" && (
            <div className="form-grid" style={gridStyle}>
              <label htmlFor="settings-install-dir" className="label">{t("settings.installDir")}</label>
              <input id="settings-install-dir" className="text-input" value={draft.install.directory} onChange={(e) => set((s) => { s.install.directory = e.target.value; return s; })} />
              <label htmlFor="settings-git" className="label">{t("settings.git")}</label>
              <div className="inline-row">
                <input id="settings-git" className="text-input" style={{ flex: 1, minWidth: 0 }} value={draft.install.git} onChange={(e) => set((s) => { s.install.git = e.target.value; return s; })} />
                <button type="button" className="dbtn inline" onClick={testGit}>{t("settings.test")}</button>
              </div>
              <span />
              <span><TestResult state={gitTest} /></span>
              <span />
              <label className="check-label">
                <input type="checkbox" checked={draft.install.usePrebuilt} onChange={() => set((s) => { s.install.usePrebuilt = !s.install.usePrebuilt; return s; })} />
                {t("settings.usePrebuilt")}
              </label>
            </div>
          )}

          {shown === "build" && (
            <div className="form-grid" style={gridStyle}>
              <label htmlFor="settings-engine" className="label">{t("settings.engine")}</label>
              <select
                id="settings-engine"
                className="select"
                value={engineValue}
                onChange={(e) => set((s) => { s.build.engineId = e.target.value || null; return s; })}
              >
                {engines.length === 0 && <option value="">—</option>}
                {engines.map((e) => (
                  <option key={e.id} value={e.id}>{t("settings.engineOption", { version: e.version.split(".").slice(0, 2).join("."), root: e.root })}</option>
                ))}
              </select>
              <label htmlFor="settings-config" className="label">{t("settings.configuration")}</label>
              <select
                id="settings-config"
                className="select"
                style={{ width: 180 }}
                value={draft.build.configuration}
                onChange={(e) => set((s) => { s.build.configuration = e.target.value === "DebugGame" ? "DebugGame" : "Development"; return s; })}
              >
                <option value="Development">Development</option>
                <option value="DebugGame">DebugGame</option>
              </select>
              <span />
              <div className="stack">
                <label className="check-label">
                  <input type="checkbox" checked={draft.build.waitForEditor} onChange={() => set((s) => { s.build.waitForEditor = !s.build.waitForEditor; return s; })} />
                  {t("settings.waitForEditor")}
                </label>
                <label className="check-label">
                  <input type="checkbox" checked={draft.build.reopenEditor} onChange={() => set((s) => { s.build.reopenEditor = !s.build.reopenEditor; return s; })} />
                  {t("settings.reopenEditor")}
                </label>
                <label className="check-label">
                  <input type="checkbox" checked={draft.build.runTests} onChange={() => set((s) => { s.build.runTests = !s.build.runTests; return s; })} />
                  {t("settings.runTests")}
                </label>
              </div>
            </div>
          )}

          {shown === "ros" && (
            <div className="form-grid" style={gridStyle}>
              <label htmlFor="settings-ros-host" className="label">{t("settings.host")}</label>
              <input id="settings-ros-host" className="text-input" style={{ width: 220 }} value={draft.ros.host} onChange={(e) => set((s) => { s.ros.host = e.target.value; return s; })} />
              <label htmlFor="settings-ros-port" className="label">{t("settings.port")}</label>
              <div className="inline-row">
                <input
                  id="settings-ros-port"
                  className={`text-input${portValid ? "" : " invalid"}`}
                  style={{ width: 90 }}
                  inputMode="numeric"
                  value={portText}
                  aria-invalid={!portValid}
                  onChange={(e) => {
                    setPortText(e.target.value);
                    const typed = parsePort(e.target.value);
                    if (typed !== null) set((s) => { s.ros.port = typed; return s; });
                  }}
                />
                <button type="button" className="dbtn inline" disabled={!portValid} onClick={testRos}>{t("settings.testConnection")}</button>
              </div>
              <span />
              <span>{portValid ? <TestResult state={rosTest} /> : <span className="result c-error">{t("settings.portInvalid")}</span>}</span>
              <span />
              <label className="check-label">
                <input type="checkbox" checked={draft.ros.checkOnRefresh} onChange={() => set((s) => { s.ros.checkOnRefresh = !s.ros.checkOnRefresh; return s; })} />
                {t("settings.checkOnRefresh")}
              </label>
            </div>
          )}
        </div>
      </div>

      <div className="dialog-footer">
        <button type="button" className="help-button" aria-label={t("common.help")} title={t("common.help")} onClick={() => platform.openUrl(HELP_URL)}>?</button>
        <span className="spacer" />
        <button
          type="button"
          className="dbtn primary"
          disabled={!portValid}
          title={portValid ? undefined : t("settings.portInvalid")}
          onClick={() => void (async () => { if (!dirty || (await save())) actions.closeDialog(); })()}
        >
          {t("common.ok")}
        </button>
        <button type="button" className="dbtn" onClick={close}>{t("common.cancel")}</button>
        <button
          type="button"
          className="dbtn"
          disabled={!dirty || !portValid}
          title={portValid ? undefined : t("settings.portInvalid")}
          onClick={() => void save()}
        >
          {t("common.apply")}
        </button>
      </div>
    </Dialog>
  );
}
