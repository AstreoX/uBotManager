import { useEffect, useMemo, useRef, useState } from "react";
import { api } from "../api";
import type { ApplyOptions, Plan, StepGroup } from "../api/types";
import { Dialog } from "../components/Dialog";
import { ChevronIcon, ErrorIcon, WarningIcon } from "../components/Icons";
import { t, tm, tn } from "../i18n";
import { isEditorRunning, sameName } from "../lib/format";
import { useApp } from "../state";

const GROUPS: StepGroup[] = ["download", "project", "build", "after"];

/** How often the dialog asks whether the editor is running; the backend sends no event when it starts or exits. */
const EDITOR_POLL_MS = 2000;

interface DepRow { name: string; note: string; mono: boolean; ok: boolean }

export function ApplyChangesDialog(props: { plan: Plan; options: ApplyOptions }) {
  const { state, actions } = useApp();
  const { plan, options } = props;
  const project = state.snapshot.project;
  const [closed, setClosed] = useState<Set<StepGroup>>(new Set());
  const [saveRecipe, setSaveRecipe] = useState(false);
  const [recipeName, setRecipeName] = useState(project?.name ?? "");

  // Re-plan when the editor starts or stops so the banner and step list stay accurate.
  const editorRunning = project?.editor.running ?? false;
  const firstRender = useRef(true);
  useEffect(() => {
    if (firstRender.current) {
      firstRender.current = false;
      return;
    }
    void actions.replan(options);
  }, [editorRunning]);

  // Nothing tells the frontend when the editor exits (Close Editor) or starts, so ask while the dialog is open.
  const { setSnapshot } = actions;
  useEffect(() => {
    let stopped = false;
    const timer = setInterval(() => {
      api.getSnapshot().then((snapshot) => {
        if (!stopped && isEditorRunning(snapshot) !== editorRunning) setSnapshot(snapshot);
      }, () => undefined);
    }, EDITOR_POLL_MS);
    return () => {
      stopped = true;
      clearInterval(timer);
    };
  }, [editorRunning, setSnapshot]);

  const waitingSteps = plan.editorRunning ? plan.steps.filter((s) => s.needsEditorClosed) : [];
  let editorBanner: string | null = null;
  if (waitingSteps.length > 0) {
    const from = waitingSteps[0]?.n ?? 0;
    const to = waitingSteps[waitingSteps.length - 1]?.n ?? 0;
    if (!state.settings.build.waitForEditor) editorBanner = t("plan.editorRunning.noWait");
    else if (from === to) editorBanner = t("plan.editorRunning.single", { n: from });
    else editorBanner = t("plan.editorRunning.range", { from, to });
  }

  const deps = useMemo<DepRow[]>(() => {
    const rows: DepRow[] = [];
    const handled = (name: string) => plan.steps.some((s) =>
      (s.kind === "clone" || s.kind === "checkout" || s.kind === "enablePackage" || s.kind === "enableEnginePlugin")
      && sameName(s.title.params["package"] ?? s.title.params["plugin"] ?? "", name));
    for (const change of plan.changes) {
      if (change.kind !== "install" && change.kind !== "update" && change.kind !== "enable") continue;
      const pkg = state.snapshot.packages.find((p) => sameName(p.name, change.package));
      for (const r of pkg?.requires ?? []) {
        if (r.optional || rows.some((x) => sameName(x.name, r.name))) continue;
        if (plan.changes.some((c) => sameName(c.package, r.name))) continue;
        if (r.kind === "enginePlugin") {
          rows.push({ name: r.name, note: t("req.enginePlugin"), mono: false, ok: r.status !== "missing" });
          continue;
        }
        if (r.status === "satisfied") {
          rows.push({ name: r.name, note: t("plan.depInstalled", { constraint: r.constraint, version: r.installedVersion ?? "" }), mono: true, ok: true });
        } else {
          const will = r.status === "notInstalled" ? t("req.willInstall") : r.status === "disabled" ? t("req.willEnable") : r.status === "versionMismatch" ? t("req.willUpdate") : t("req.missing");
          rows.push({ name: r.name, note: `${r.constraint} · ${will}`, mono: true, ok: handled(r.name) });
        }
      }
    }
    return rows;
  }, [plan, state.snapshot.packages]);

  const summarySteps = tn("plan.steps", plan.steps.length);
  const summary = plan.steps.length === 0 ? t("plan.nothingToDo") : plan.needsBuild ? t("plan.buildRequired", { steps: summarySteps }) : summarySteps;
  const canApply = plan.errors.length === 0 && plan.steps.length > 0 && !state.busy && (!saveRecipe || recipeName.trim().length > 0);

  return (
    <Dialog className="plan-dialog" title={t("plan.title")} subtitle={project?.name} onClose={actions.closeDialog}>
      <div className="plan-scroll">
        {editorBanner && (
          <div className="banner warning">
            <WarningIcon size={16} strokeWidth={1.3} />
            <span className="text">{editorBanner}</span>
            <button type="button" className="link-button" onClick={actions.closeEditor}>{t("plan.closeEditor")}</button>
          </div>
        )}
        {plan.errors.map((m, i) => (
          <div key={`e${i}`} className="banner error">
            <ErrorIcon size={16} strokeWidth={1.3} />
            <span className="text">{tm(m)}</span>
          </div>
        ))}
        {plan.warnings.map((m, i) => (
          <div key={`w${i}`} className="banner warning">
            <WarningIcon size={16} strokeWidth={1.3} />
            <span className="text">{tm(m)}</span>
          </div>
        ))}

        <div className="plan-steps">
          {GROUPS.map((group) => {
            const steps = plan.steps.filter((s) => s.group === group);
            if (steps.length === 0) return null;
            const isClosed = closed.has(group);
            return (
              <div key={group}>
                <button
                  type="button"
                  className={`plan-group-header${isClosed ? " closed" : ""}`}
                  aria-expanded={!isClosed}
                  onClick={() => {
                    const next = new Set(closed);
                    if (isClosed) next.delete(group);
                    else next.add(group);
                    setClosed(next);
                  }}
                >
                  <ChevronIcon size={12} />
                  {t(`plan.group.${group}`)}
                  <span className="count">{steps.length}</span>
                </button>
                {!isClosed && steps.map((s) => {
                  const detail = tm(s.detail);
                  return (
                    <div key={s.id} className="plan-step">
                      <span className="n">{s.n}</span>
                      <span className="title">{tm(s.title)}</span>
                      <span className="detail" title={detail}>{detail}</span>
                    </div>
                  );
                })}
              </div>
            );
          })}
        </div>

        <div className="plan-extra">
          <div className="col">
            <div className="col-title">{t("plan.dependencies")}</div>
            {deps.length === 0 && <div className="c-muted">{t("common.none")}</div>}
            {deps.map((d) => (
              <div key={d.name} className="dep-row">
                <span>{d.name}</span>
                <span className={`note${d.mono ? " mono" : ""}`}>{d.note}</span>
                {d.ok ? <span className="c-success">✓</span> : <span className="c-error">✗</span>}
              </div>
            ))}
          </div>
          <div className="col options">
            <div className="col-title">{t("plan.options")}</div>
            <label className="check-label">
              <input
                type="checkbox"
                checked={options.reopenEditor}
                onChange={() => void actions.replan({ ...options, reopenEditor: !options.reopenEditor })}
              />
              {t("plan.reopenEditor")}
            </label>
            <label className="check-label">
              <input type="checkbox" checked={saveRecipe} onChange={() => setSaveRecipe(!saveRecipe)} />
              {t("plan.saveRecipe")}
            </label>
            {saveRecipe && (
              <input
                className="text-input recipe-name-input"
                value={recipeName}
                placeholder={t("plan.recipeName")}
                aria-label={t("plan.recipeName")}
                onChange={(e) => setRecipeName(e.target.value)}
                autoFocus
              />
            )}
          </div>
        </div>
      </div>

      <div className="dialog-footer">
        <span className="summary">{summary}</span>
        <span className="spacer" />
        <button type="button" className="dbtn" onClick={actions.closeDialog}>{t("common.cancel")}</button>
        <button type="button" className="dbtn primary" disabled={!canApply} onClick={() => void actions.apply(saveRecipe ? recipeName.trim() : null)}>
          {t("common.apply")}
        </button>
      </div>
    </Dialog>
  );
}
