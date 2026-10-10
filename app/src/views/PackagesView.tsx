import { useMemo, useRef, useState, type KeyboardEvent } from "react";
import { platform } from "../api";
import type { Change, Fix, PackageInfo, Problem, Requirement } from "../api/types";
import { useProjectMenu } from "../components/Chrome";
import { ChevronIcon, ErrorIcon, InfoIcon, SearchIcon, WarningIcon } from "../components/Icons";
import { OptionsMenu } from "../components/Menu";
import { hasKey, localized, t, tm } from "../i18n";
import {
  changeWord, defaultVersion, detailsKey, displayUrl, findChange, installedFromText, isInstalled, problemsFor, sameChange, sameName,
  versionChoices
} from "../lib/format";
import { compareVersionStrings } from "../lib/semver";
import { canSwitchProject } from "../lib/tasks";
import { useApp } from "../state";

export function PackagesView() {
  const { state } = useApp();
  const menu = useProjectMenu();
  return (
    <main className="tool-window">
      <div className="tw-header">
        <span className="tw-title">{t("view.packages")}</span>
        <span className="spacer" />
        <OptionsMenu entries={menu} />
      </div>
      {state.snapshot.project ? (
        <div className="split">
          <PackageList />
          <PackageDetailsPane />
        </div>
      ) : (
        <NoProject />
      )}
    </main>
  );
}

function matches(pkg: PackageInfo, query: string): boolean {
  if (!query) return true;
  return pkg.name.toLowerCase().includes(query)
    || pkg.friendlyName.toLowerCase().includes(query)
    || pkg.provides.some((c) => c.toLowerCase().includes(query));
}

function PackageList() {
  const { state, actions } = useApp();
  const [query, setQuery] = useState("");
  const listRef = useRef<HTMLDivElement>(null);
  const q = query.trim().toLowerCase();
  const { problems } = state.snapshot;

  const groups = useMemo(() => {
    const packages = state.snapshot.packages;
    const installed = packages.filter((p) => isInstalled(p) && matches(p, q));
    const available = packages.filter((p) =>
      !isInstalled(p) && (p.state === "notInstalled" || state.settings.index.showPlanned) && matches(p, q));
    return [
      { id: "installed", title: t("packages.installed"), items: installed },
      { id: "index", title: t("packages.indexGroup"), items: available }
    ];
  }, [state.snapshot.packages, state.settings.index.showPlanned, q]);

  const flat = groups.flatMap((g) => g.items);

  const onKeyDown = (e: KeyboardEvent) => {
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
    e.preventDefault();
    if (flat.length === 0) return;
    const index = flat.findIndex((p) => state.selected !== null && sameName(p.name, state.selected));
    const next = e.key === "ArrowDown" ? Math.min(flat.length - 1, index + 1) : Math.max(0, index < 0 ? 0 : index - 1);
    const pkg = flat[next];
    if (!pkg) return;
    actions.select(pkg.name);
    listRef.current?.querySelector(`[data-name="${pkg.name}"]`)?.scrollIntoView({ block: "nearest" });
  };

  return (
    <section className="pkg-list" aria-label={t("packages.list")}>
      <div className="search-wrap">
        <label className="search-box">
          <SearchIcon size={14} />
          <span className="sr-only">{t("common.search")}</span>
          <input type="search" value={query} placeholder={t("common.search")} onChange={(e) => setQuery(e.target.value)} onKeyDown={onKeyDown} />
        </label>
      </div>
      <div className="list-scroll" ref={listRef} tabIndex={0} onKeyDown={onKeyDown} role="listbox" aria-label={t("packages.list")}>
        {groups.map((g) => g.items.length > 0 && (
          <div key={g.id}>
            <div className="group-header">
              <ChevronIcon size={12} />
              {g.title}
              <span className="count">{g.items.length}</span>
            </div>
            {g.items.map((pkg) => (
              <PackageRow key={pkg.name} pkg={pkg} problems={problems} />
            ))}
          </div>
        ))}
        {flat.length === 0 && <div className="list-empty">{t("packages.noMatches")}</div>}
      </div>
    </section>
  );
}

function PackageRow({ pkg, problems }: { pkg: PackageInfo; problems: Problem[] }) {
  const { state, actions } = useApp();
  const selected = state.selected !== null && sameName(pkg.name, state.selected);
  const queued = findChange(state.pending, pkg.name);
  const top = problemsFor(pkg, problems).find((p) => p.severity !== "info");
  let meta = pkg.installedVersion ?? pkg.latestVersion ?? "";
  let metaClass = "meta";
  if (pkg.state === "planned") meta = t("packages.planned");
  else if (pkg.updateAvailable && pkg.installedVersion && pkg.latestVersion) {
    meta = `${pkg.installedVersion} → ${pkg.latestVersion}`;
    metaClass = "meta update";
  }
  if (queued) {
    meta = t(`packages.pending.${changeWord(queued, pkg)}`);
    metaClass = "meta pending";
  }
  const dim = pkg.state === "disabled" || pkg.state === "planned";
  return (
    <button
      type="button"
      role="option"
      aria-selected={selected}
      data-name={pkg.name}
      className={`pkg-row${selected ? " selected" : ""}`}
      onClick={() => actions.select(pkg.name)}
      title={localized(pkg.description)}
    >
      <span className={`name${dim ? " dim" : ""}`}>{pkg.name}</span>
      {top && (top.severity === "error"
        ? <ErrorIcon size={14} strokeWidth={1.4} className="badge c-error" />
        : <WarningIcon size={14} strokeWidth={1.4} className="badge c-warning" />)}
      <span className={metaClass}>{meta}</span>
    </button>
  );
}

function PackageDetailsPane() {
  const { state } = useApp();
  const pkg = state.snapshot.packages.find((p) => state.selected !== null && sameName(p.name, state.selected));
  if (!pkg) return <section className="pkg-details" aria-label={t("packages.details")} />;
  // Starts over (version choice included) when a refresh changes the facts the choice is derived from.
  return <PackageDetails key={detailsKey(pkg)} pkg={pkg} />;
}

interface ActionButton { label: string; primary?: boolean; disabled?: boolean; run: () => void }

function stateLabel(pkg: PackageInfo, problems: Problem[]): { text: string; className: string } {
  const top = problemsFor(pkg, problems).find((p) => p.severity !== "info");
  if (isInstalled(pkg) && top) return { text: t("state.problem"), className: top.severity === "error" ? "c-error" : "c-warning" };
  if (isInstalled(pkg) && pkg.updateAvailable) return { text: t("state.update"), className: "c-link" };
  switch (pkg.state) {
    case "enabled": return { text: t("state.enabled"), className: "c-success" };
    case "disabled": return { text: t("state.disabled"), className: "c-muted" };
    case "notInstalled": return { text: t("state.notInstalled"), className: "c-muted" };
    case "planned": return { text: t("state.planned"), className: "c-muted" };
  }
}

function requirementStatus(pkg: PackageInfo, r: Requirement): { text: string; className: string } {
  const installed = isInstalled(pkg);
  switch (r.status) {
    case "satisfied":
      return { text: t("req.satisfied"), className: "c-success" };
    case "versionMismatch":
      return installed
        ? { text: t("req.installedVersion", { version: r.installedVersion ?? "" }), className: "c-warning" }
        : { text: t("req.willUpdate"), className: "c-info" };
    case "notInstalled":
      return installed ? { text: t("req.notInstalled"), className: "c-warning" } : { text: t("req.willInstall"), className: "c-info" };
    case "disabled":
      if (!installed) return { text: t("req.willEnable"), className: "c-info" };
      return { text: t("req.disabled"), className: pkg.state === "enabled" ? "c-warning" : "c-muted" };
    case "missing":
      return { text: t("req.missing"), className: "c-error" };
  }
}

function fixLabel(fix: Fix): string {
  const short = `fixShort.${fix.label.code}`;
  return hasKey(short) ? t(short, fix.label.params) : tm(fix.label);
}

function PackageDetails({ pkg }: { pkg: PackageInfo }) {
  const { state, actions } = useApp();
  const problems = problemsFor(pkg, state.snapshot.problems);
  const queued = findChange(state.pending, pkg.name);
  const queuedVersion = queued?.version ?? null;
  const versions = useMemo(() => versionChoices(pkg, queuedVersion), [pkg.versions, pkg.installedVersion, queuedVersion]);
  const [picked, setPicked] = useState<string>(() => defaultVersion(pkg));
  // A queued change (also one queued from a banner or the Problems view) shows the version it targets.
  const version = queuedVersion ?? picked;
  const label = stateLabel(pkg, problems);
  const installedFrom = installedFromText(pkg);

  const queue = (change: Change) => actions.queue(change);
  const simple = (kind: Change["kind"]) => () => queue({ kind, package: pkg.name, version: null });

  let buttons: ActionButton[];
  if (queued) {
    buttons = [{ label: t(`action.cancel.${changeWord(queued, pkg)}`), run: () => actions.unqueue(pkg.name) }];
  } else if (pkg.state === "planned") {
    buttons = [{ label: t("action.notAvailable"), disabled: true, run: () => undefined }];
  } else if (pkg.state === "notInstalled") {
    buttons = [{ label: t("action.install"), primary: true, run: () => queue({ kind: "install", package: pkg.name, version: version || null }) }];
  } else {
    buttons = [];
    const changesVersion = !!version && !!pkg.installedVersion && version !== pkg.installedVersion;
    if (changesVersion) {
      const newer = compareVersionStrings(version, pkg.installedVersion ?? "") > 0;
      buttons.push({
        label: t(newer ? "action.updateTo" : "action.downgradeTo", { version }), primary: true,
        run: () => queue({ kind: "update", package: pkg.name, version })
      });
    }
    if (pkg.state === "disabled") buttons.push({ label: t("action.enable"), primary: !changesVersion, run: simple("enable") });
    else buttons.push({ label: t("action.disable"), run: simple("disable") });
    buttons.push({ label: t("action.uninstall"), run: simple("uninstall") });
  }

  const top = problems[0];
  const fix = top ? top.fixes.find((f) => f.primary) ?? top.fixes[0] : undefined;
  const fixQueued = fix?.action.kind === "changes" && fix.action.changes.every((c) => state.pending.some((p) => sameChange(p, c)));
  const showFix = !!fix && !fixQueued && !(fix.action.kind === "changes" && queued);
  const BannerIcon = top?.severity === "error" ? ErrorIcon : top?.severity === "info" ? InfoIcon : WarningIcon;

  return (
    <section className="pkg-details" aria-label={t("packages.details")}>
      <div className="details-header">
        <span className="details-name">{pkg.name}</span>
        <span className={`state-label ${label.className}`}>{label.text}</span>
        <span className="spacer" />
        <label className="version-label">
          {t("details.version")}
          <select
            className="version-select"
            value={pkg.state === "planned" ? "—" : version}
            disabled={pkg.state === "planned" || !!queued}
            onChange={(e) => setPicked(e.target.value)}
          >
            {pkg.state === "planned" ? <option value="—">—</option> : versions.map((v) => <option key={v} value={v}>{v}</option>)}
          </select>
        </label>
        {buttons.map((b) => (
          <button key={b.label} type="button" className={`btn${b.primary ? " primary" : ""}`} disabled={b.disabled} onClick={b.run}>
            {b.label}
          </button>
        ))}
      </div>

      {top && (
        <div className={`banner ${top.severity}`}>
          <BannerIcon size={16} strokeWidth={1.3} />
          <span className="text">{tm(top.message)}</span>
          {showFix && fix && (
            <button type="button" className="link-button" onClick={() => actions.runFix(top, fix)}>{fixLabel(fix)}</button>
          )}
        </div>
      )}

      <div className="details-body">
        <article className="details-article">
          <div>
            <h1>{pkg.name}</h1>
            <p>{localized(pkg.description)}</p>
          </div>
          <dl className="fields">
            <dt>{t("details.layer")}</dt><dd>{t(`layer.${pkg.layer}`)}</dd>
            <dt>{t("details.installed")}</dt><dd className="mono">{pkg.installedVersion ?? "—"}</dd>
            <dt>{t("details.latest")}</dt>
            <dd>
              <span className="mono">{pkg.latestVersion ?? "—"}</span>
              {!isInstalled(pkg) && pkg.prebuiltAvailable && <span className="field-note">{t("details.prebuiltNote")}</span>}
            </dd>
            <dt>{t("details.location")}</dt><dd className="mono">{pkg.location ?? "—"}</dd>
            <dt>{t("details.installedFrom")}</dt><dd className={installedFrom ? undefined : "mono"}>{installedFrom ?? "—"}</dd>
            <dt>{t("details.repository")}</dt>
            <dd>
              {pkg.webUrl ? (
                <a href={pkg.webUrl} onClick={(e) => { e.preventDefault(); platform.openUrl(pkg.webUrl ?? ""); }}>{displayUrl(pkg.webUrl)}</a>
              ) : pkg.repository ? displayUrl(pkg.repository) : "—"}
            </dd>
          </dl>
          <div>
            <h2>{t("details.requires")}</h2>
            {pkg.requires.length === 0 && <div className="c-muted">{t("common.none")}</div>}
            {pkg.requires.map((r) => {
              const status = requirementStatus(pkg, r);
              return (
                <div key={`${r.kind}:${r.name}`} className="req-row">
                  <span>{r.name}</span>
                  <span className="constraint">{r.kind === "enginePlugin" ? t("req.enginePlugin") : r.constraint}</span>
                  <span className={r.optional ? "c-muted" : status.className}>
                    {status.text}{r.optional ? ` · ${t("req.optional")}` : ""}
                  </span>
                </div>
              );
            })}
          </div>
          <div>
            <h2>{t("details.provides")}</h2>
            {pkg.provides.length === 0
              ? <div className="c-muted">{t("common.none")}</div>
              : <div className="provides">{pkg.provides.map((c) => <div key={c}>{c}</div>)}</div>}
          </div>
        </article>
      </div>
    </section>
  );
}

function NoProject() {
  const { state, actions } = useApp();
  // The core refuses to open a project while a task runs.
  const busy = !canSwitchProject(state.tasks);
  const title = busy ? tm({ code: "task.busy", params: {} }) : undefined;
  return (
    <div className="empty-project">
      <div className="inner">
        <h1>{t("noProject.title")}</h1>
        <div>
          <button type="button" className="dbtn primary" disabled={busy} title={title} onClick={() => void actions.openProject()}>{t("noProject.open")}</button>
        </div>
        {state.settings.recentProjects.length > 0 && (
          <>
            <div className="recent-title">{t("noProject.recent")}</div>
            <div className="recent-list">
              {state.settings.recentProjects.map((path) => (
                <button key={path} type="button" className="recent-item" disabled={busy} title={title} onClick={() => void actions.openProject(path)}>
                  <span>{path.split(/[\\/]/).pop()?.replace(/\.uproject$/i, "")}</span>
                  <span className="path">{path}</span>
                </button>
              ))}
            </div>
          </>
        )}
      </div>
    </div>
  );
}
