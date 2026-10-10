import { useEffect, useMemo, useState } from "react";
import { api, platform } from "../api";
import type { EnvCheck, PackageInfo, Recipe, Snapshot } from "../api/types";
import { Dialog } from "../components/Dialog";
import { CheckIcon, CloseIcon, FolderIcon, SmallInfoIcon } from "../components/Icons";
import { getLanguage, localized, t, tm, tn } from "../i18n";
import { sameName, sameProjectPath } from "../lib/format";
import { canSwitchProject } from "../lib/tasks";
import { useApp } from "../state";

const CORE = "UBotCore";
const BASE_CHECKS = ["vs2022", "git"];

/** First recipe that would change something in the open project, so the dialog opens on a useful one. */
function initialRecipe(snapshot: Snapshot): Recipe | undefined {
  const done = (name: string) => snapshot.packages.some((p) => sameName(p.name, name) && p.state === "enabled");
  return snapshot.recipes.find((r) => r.available && r.packages.some((n) => !done(n))) ?? snapshot.recipes[0];
}

export function ApplyRecipeDialog() {
  const { state, actions } = useApp();
  const { snapshot, settings } = state;
  const [recipeId, setRecipeId] = useState(() => initialRecipe(snapshot)?.id ?? null);
  const [extra, setExtra] = useState<Set<string>>(new Set());
  const [removed, setRemoved] = useState<Set<string>>(new Set());
  const [path, setPath] = useState(snapshot.project?.path ?? settings.lastProject ?? "");
  const [checks, setChecks] = useState<EnvCheck[] | null>(null);
  const [note, setNote] = useState<string | null>(null);
  const [working, setWorking] = useState(false);
  const recipe = snapshot.recipes.find((r) => r.id === recipeId);

  useEffect(() => {
    let disposed = false;
    setChecks(null);
    const ids = [...BASE_CHECKS, ...(recipe?.checks ?? []).filter((c) => !BASE_CHECKS.includes(c))];
    api.checkEnvironment(ids).then((result) => { if (!disposed) setChecks(result); }, actions.report);
    return () => { disposed = true; };
  }, [recipe, actions.report]);

  const inRecipe = (name: string) => !!recipe?.packages.some((n) => sameName(n, name));
  const packages = useMemo(() => snapshot.packages.filter((p) =>
    p.state !== "planned" || !!recipe?.packages.some((n) => sameName(n, p.name))), [snapshot.packages, recipe]);
  const isChecked = (p: PackageInfo) =>
    p.state !== "planned" && (sameName(p.name, CORE) || ((inRecipe(p.name) || extra.has(p.name)) && !removed.has(p.name)));
  const checked = packages.filter(isChecked);

  const sameProject = !!snapshot.project && sameProjectPath(path.trim(), snapshot.project.path);
  // Reviewing in another project opens it first, which the core refuses while a task runs.
  const blocked = !sameProject && !canSwitchProject(state.tasks);
  const engine = sameProject ? snapshot.project?.engine ?? null : null;

  const pick = (r: Recipe) => {
    setRecipeId(r.id);
    setExtra(new Set());
    setRemoved(new Set());
    setNote(null);
  };

  const toggle = (p: PackageInfo) => {
    if (sameName(p.name, CORE) || p.state === "planned") return;
    setNote(null);
    if (isChecked(p)) {
      setRemoved(new Set(removed).add(p.name));
      const next = new Set(extra);
      next.delete(p.name);
      setExtra(next);
    } else {
      setExtra(new Set(extra).add(p.name));
      const next = new Set(removed);
      next.delete(p.name);
      setRemoved(next);
    }
  };

  const browse = async () => {
    const picked = await platform.pickProjectFile(t("dialog.openProject"), t("dialog.uprojectFilter"));
    if (picked) setPath(picked);
  };

  const review = async () => {
    if (!recipe) return;
    setWorking(true);
    try {
      if (!sameProject && !(await actions.openProject(path.trim()))) return;
      const changes = await api.recipeChanges(recipe.id, checked.map((p) => p.name));
      if (changes.length === 0) {
        setNote(t("recipes.nothingToChange"));
        return;
      }
      const projectName = path.trim().split(/[\\/]/).pop()?.replace(/\.uproject$/i, "") ?? "";
      await actions.reviewChanges(changes, { code: "task.recipe", params: { recipe: localized(recipe.name), project: projectName } });
    } catch (e) {
      actions.report(e);
    } finally {
      setWorking(false);
    }
  };

  const checkLabel = (c: EnvCheck) => t(`env.${c.id}`);
  const checkDetail = (c: EnvCheck) => c.ok === null ? t("env.notChecked") : c.ok ? t("env.found") : c.detail ?? t("env.notFound");

  return (
    <Dialog className="recipe-dialog" title={t("recipes.title")} onClose={actions.closeDialog}>
      <div className="dialog-split">
        <nav className="side-nav" aria-label={t("recipes.list")}>
          {snapshot.recipes.map((r) => (
            <button
              key={r.id}
              type="button"
              className={`side-item${r.id === recipeId ? " selected" : ""}${r.available ? "" : " dim"}`}
              onClick={() => pick(r)}
              title={[localized(r.name), localized(r.description)].filter(Boolean).join(" — ")}
            >
              <span className="label">{localized(r.name)}</span>
              {!r.available && <span className="tag">{t("recipes.planned")}</span>}
            </button>
          ))}
        </nav>

        <div className="recipe-page" style={{ ["--label-width" as string]: getLanguage() === "zh-CN" ? "72px" : "110px" }}>
          {recipe && (
            <div className="heading">
              <span className="name">{localized(recipe.name)}</span>
              <span className="desc">{localized(recipe.description)}</span>
            </div>
          )}

          <label htmlFor="recipe-project" className="label" style={{ paddingTop: 4, alignSelf: "start" }}>{t("recipes.project")}</label>
          <div className="inline-row">
            <input
              id="recipe-project"
              className="text-input"
              style={{ flex: 1, minWidth: 0 }}
              value={path}
              onChange={(e) => { setPath(e.target.value); setNote(null); }}
            />
            <button type="button" className="browse-button" aria-label={t("recipes.browse")} title={t("recipes.browse")} onClick={() => void browse()}>
              <FolderIcon size={15} />
            </button>
          </div>

          <span className="label">{t("recipes.engine")}</span>
          <span>
            {engine ? (
              <>
                <span>{t("recipes.engineName", { version: engine.version.split(".").slice(0, 2).join(".") })}</span>
                <span className="mono c-muted" style={{ marginLeft: 8 }}>{engine.root}</span>
              </>
            ) : <span className="c-muted">{t("recipes.engineUnknown")}</span>}
          </span>

          <span className="label" style={{ paddingTop: 2, alignSelf: "start" }}>{t("recipes.packages")}</span>
          <div className="package-box">
            {packages.map((p) => {
              const locked = sameName(p.name, CORE);
              const on = isChecked(p);
              const planned = p.state === "planned";
              return (
                <label key={p.name} className={`package-check${on ? "" : " off"}${locked || planned ? " locked" : ""}`}>
                  <input type="checkbox" checked={on} disabled={locked || planned} onChange={() => toggle(p)} />
                  <span className="name">{p.name}</span>
                  <span className="note">
                    {locked ? t("recipes.required") : planned ? t("recipes.planned") : p.latestVersion ?? p.installedVersion}
                  </span>
                </label>
              );
            })}
          </div>

          <span className="label" style={{ alignSelf: "start" }}>{t("recipes.requirements")}</span>
          <div className="checks">
            {checks === null && <span className="c-muted">{t("env.checking")}</span>}
            {checks?.map((c) => (
              <div key={c.id} className="check-row">
                {c.ok === null ? <SmallInfoIcon size={14} className="c-info" />
                  : c.ok ? <CheckIcon size={14} strokeWidth={1.5} className="c-success" />
                    : <CloseIcon size={14} strokeWidth={1.5} className="c-error" />}
                <span>{checkLabel(c)}</span>
                <span className="detail" title={c.detail ?? undefined}>{checkDetail(c)}</span>
              </div>
            ))}
          </div>
        </div>
      </div>

      <div className="dialog-footer">
        <span className="summary">{note ?? (blocked ? tm({ code: "task.busy", params: {} }) : tn("recipes.count", checked.length))}</span>
        <span className="spacer" />
        <button type="button" className="dbtn" onClick={actions.closeDialog}>{t("common.cancel")}</button>
        <button type="button" className="dbtn primary" disabled={!recipe || !path.trim() || working || state.busy || blocked} onClick={() => void review()}>
          {t("recipes.review")}
        </button>
      </div>
    </Dialog>
  );
}
