import type { Change, PackageInfo, Problem, Severity, Snapshot } from "../api/types";
import { t } from "../i18n";
import { compareVersionStrings } from "./semver";

const pad2 = (n: number) => String(n).padStart(2, "0");

/** "< 1 s", "4 s", "1 min 05 s" / "<1 秒", "4 秒", "1 分 5 秒". */
export function formatDuration(ms: number): string {
  if (ms < 1000) return t("duration.lessThanSecond");
  const total = Math.floor(ms / 1000);
  if (total < 60) return t("duration.seconds", { s: total });
  const minutes = Math.floor(total / 60);
  const seconds = total % 60;
  if (minutes < 60) return t("duration.minutes", { m: minutes, s: seconds, ss: pad2(seconds) });
  const hours = Math.floor(minutes / 60);
  return t("duration.hours", { h: hours, m: minutes % 60, mm: pad2(minutes % 60) });
}

export function formatClock(iso: string): string {
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? iso : `${pad2(d.getHours())}:${pad2(d.getMinutes())}`;
}

export function sameName(a: string, b: string): boolean {
  return a.toLowerCase() === b.toLowerCase();
}

/** Version a change targets, falling back to the package's latest version for installs. */
function changeVersion(change: Change, pkg: PackageInfo | undefined): string {
  return change.version ?? (change.kind === "install" ? pkg?.latestVersion ?? "" : "");
}

/** "Install UBotVehicles 0.1.0", "Update UBotSensor to 0.1.1", … */
export function changeText(change: Change, pkg: PackageInfo | undefined): string {
  return t(`change.${change.kind}`, { package: pkg?.name ?? change.package, version: changeVersion(change, pkg) }).trim();
}

/** At most one pending change per package: a new one replaces the old. */
export function upsertChange(list: Change[], change: Change): Change[] {
  const rest = list.filter((c) => !sameName(c.package, change.package));
  return [...rest, change];
}

export function findChange(list: Change[], name: string): Change | undefined {
  return list.find((c) => sameName(c.package, name));
}

export function sameChange(a: Change, b: Change): boolean {
  return a.kind === b.kind && sameName(a.package, b.package) && (a.version ?? null) === (b.version ?? null);
}

const severityRank: Record<Severity, number> = { error: 0, warning: 1, info: 2 };

export function bySeverity(a: Problem, b: Problem): number {
  return severityRank[a.severity] - severityRank[b.severity];
}

/** The package's problems, most severe first. */
export function problemsFor(pkg: PackageInfo, problems: Problem[]): Problem[] {
  return problems.filter((p) => pkg.problemIds.includes(p.id)).sort(bySeverity);
}

/** What the status bar counts: errors and warnings. Info problems (e.g. an unreachable ROS bridge) stay in the Problems view. */
export function problemCounts(problems: Problem[]): { errors: number; warnings: number } {
  return {
    errors: problems.filter((p) => p.severity === "error").length,
    warnings: problems.filter((p) => p.severity === "warning").length
  };
}

/** Group name the core gives problems of the package index sources (a fixed string, not a package or file). */
export const INDEX_PROBLEM_GROUP = "Package Index";

/** Display text of a problem's `group`: the index group is translated, package and file names are shown as they are. */
export function problemGroupTitle(group: string): string {
  return group === INDEX_PROBLEM_GROUP ? t("problems.groupIndex") : group;
}

/** Project paths compare case-insensitively with `/` and `\` equivalent; `null` is "no project". */
export function sameProjectPath(a: string | null | undefined, b: string | null | undefined): boolean {
  const normal = (path: string | null | undefined) => (path ?? "").replace(/\\/g, "/").toLowerCase();
  return normal(a) === normal(b);
}

export function isEditorRunning(snapshot: Snapshot): boolean {
  return snapshot.project?.editor.running ?? false;
}

/** The version the details pane starts on: the newer one when an update exists, else the installed (or latest) one. */
export function defaultVersion(pkg: PackageInfo): string {
  return (pkg.updateAvailable ? pkg.latestVersion : pkg.installedVersion ?? pkg.latestVersion) ?? "";
}

/** Versions the details pane offers, newest first: the index versions, the installed one and the queued one. */
export function versionChoices(pkg: PackageInfo, queuedVersion: string | null): string[] {
  const list = [...pkg.versions];
  for (const extra of [pkg.installedVersion, queuedVersion]) {
    if (extra && !list.includes(extra)) list.push(extra);
  }
  return list.sort((a, b) => compareVersionStrings(b, a));
}

/** Changes whenever the facts `defaultVersion` is derived from change, so the details pane starts over on them. */
export function detailsKey(pkg: PackageInfo): string {
  return [pkg.name, pkg.installedVersion ?? "", pkg.latestVersion ?? "", pkg.updateAvailable].join("|");
}

export function isInstalled(pkg: PackageInfo): boolean {
  return pkg.state === "enabled" || pkg.state === "disabled";
}

/** "https://github.com/AstreoX/uBotSensor" → "github.com/AstreoX/uBotSensor". */
export function displayUrl(url: string): string {
  return url.replace(/^[a-z]+:\/\//i, "").replace(/^[^@/]+@/, "").replace(/\.git$/, "").replace(/\/$/, "");
}
