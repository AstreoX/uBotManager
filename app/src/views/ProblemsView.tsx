import { useMemo, useState, type KeyboardEvent } from "react";
import type { Problem, Severity } from "../api/types";
import { ChevronIcon, ErrorIcon, InfoIcon, WarningIcon } from "../components/Icons";
import { t, tm } from "../i18n";
import { problemGroupTitle } from "../lib/format";
import { useApp } from "../state";

type Scope = Problem["scope"];

function SeverityIcon({ severity, size = 14 }: { severity: Severity; size?: number }) {
  if (severity === "error") return <ErrorIcon size={size} strokeWidth={1.4} className="c-error" />;
  if (severity === "info") return <InfoIcon size={size} strokeWidth={1.4} className="c-info" />;
  return <WarningIcon size={size} strokeWidth={1.4} className="c-warning" />;
}

interface Group { title: string; items: Problem[] }

function groupProblems(problems: Problem[]): Group[] {
  const groups: Group[] = [];
  for (const p of problems) {
    let g = groups.find((x) => x.title === p.group);
    if (!g) {
      g = { title: p.group, items: [] };
      groups.push(g);
    }
    g.items.push(p);
  }
  return groups;
}

export function ProblemsView() {
  const { state, actions } = useApp();
  const [scope, setScope] = useState<Scope>("packages");
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  const all = state.snapshot.problems;
  const inScope = useMemo(() => all.filter((p) => p.scope === scope), [all, scope]);
  const groups = useMemo(() => groupProblems(inScope), [inScope]);
  const visible = groups.flatMap((g) => (collapsed.has(g.title) ? [] : g.items));
  const current = inScope.find((p) => p.id === selectedId) ?? visible[0] ?? inScope[0];

  const count = (s: Scope) => all.filter((p) => p.scope === s).length;

  const onKeyDown = (e: KeyboardEvent) => {
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
    e.preventDefault();
    const index = visible.findIndex((p) => p.id === current?.id);
    const next = visible[e.key === "ArrowDown" ? Math.min(visible.length - 1, index + 1) : Math.max(0, index - 1)];
    if (next) setSelectedId(next.id);
  };

  return (
    <main className="tool-window">
      <div className="tw-header tabs">
        <span className="tw-title">{t("view.problems")}</span>
        <div role="tablist" aria-label={t("problems.scope")} className="tablist">
          {(["packages", "session"] as const).map((s) => (
            <button
              key={s}
              type="button"
              role="tab"
              aria-selected={scope === s}
              className={`tab${scope === s ? " active" : ""}`}
              onClick={() => {
                setScope(s);
                setSelectedId(null);
              }}
            >
              {t(`problems.${s}`)} <span className="count">{count(s)}</span>
            </button>
          ))}
        </div>
        <span className="spacer" />
      </div>
      <div className="split">
        <section className="problem-list" aria-label={t("problems.list")} tabIndex={0} onKeyDown={onKeyDown}>
          {groups.length === 0 && <div className="list-placeholder">{t("problems.none")}</div>}
          {groups.map((g) => {
            const closed = collapsed.has(g.title);
            return (
              <div key={g.title}>
                <button
                  type="button"
                  className="problem-group-header"
                  aria-expanded={!closed}
                  onClick={() => {
                    const next = new Set(collapsed);
                    if (closed) next.delete(g.title);
                    else next.add(g.title);
                    setCollapsed(next);
                  }}
                >
                  <ChevronIcon size={12} style={closed ? { transform: "rotate(-90deg)" } : undefined} />
                  {problemGroupTitle(g.title)}
                  <span className="count">{g.items.length}</span>
                </button>
                {!closed && g.items.map((p) => (
                  <button
                    key={p.id}
                    type="button"
                    className={`problem-row${current?.id === p.id ? " selected" : ""}`}
                    onClick={() => setSelectedId(p.id)}
                  >
                    <SeverityIcon severity={p.severity} />
                    <span className="text">{tm(p.message)}</span>
                  </button>
                ))}
              </div>
            );
          })}
        </section>
        <section className="problem-detail" aria-label={t("problems.details")}>
          {current && (
            <>
              <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
                <span className="where">{problemGroupTitle(current.group)}</span>
                <span className="headline">{tm(current.message)}</span>
              </div>
              {current.facts.length > 0 && (
                <div className="facts">
                  {current.facts.map((f, i) => (
                    <div key={i} className="fact">
                      <span className="k">{tm(f.label)}</span>
                      <span className="v">{f.valueMsg ? tm(f.valueMsg) : f.value}</span>
                    </div>
                  ))}
                </div>
              )}
              {current.fixes.length > 0 && (
                <div className="fixes">
                  <span className="label">{t("problems.quickFixes")}</span>
                  {current.fixes.map((fix, i) => (
                    <button
                      key={i}
                      type="button"
                      className={`fix-button${fix.primary ? " primary" : ""}`}
                      onClick={() => actions.runFix(current, fix)}
                    >
                      {tm(fix.label)}
                    </button>
                  ))}
                </div>
              )}
            </>
          )}
        </section>
      </div>
    </main>
  );
}
