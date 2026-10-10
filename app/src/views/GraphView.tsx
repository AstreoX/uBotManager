import { useCallback, useEffect, useMemo, useRef, useState, type PointerEvent } from "react";
import type { PackageInfo, Problem } from "../api/types";
import { useProjectMenu } from "../components/Chrome";
import { ErrorIcon, WarningIcon } from "../components/Icons";
import { OptionsMenu } from "../components/Menu";
import { localized, t } from "../i18n";
import { isInstalled, problemsFor, sameName } from "../lib/format";
import { layoutGraph, type GraphNode } from "../lib/graphLayout";
import { useApp } from "../state";

const ZOOM_MIN = 0.3;
const ZOOM_MAX = 2;
const ZOOM_STEP = 0.1;
const clampZoom = (z: number) => Math.min(ZOOM_MAX, Math.max(ZOOM_MIN, Math.round(z * 100) / 100));

function blockingProblem(pkg: PackageInfo, problems: Problem[]): Problem | undefined {
  if (!isInstalled(pkg)) return undefined;
  return problemsFor(pkg, problems).find((p) => p.severity !== "info");
}

function problemLine(problem: Problem): string {
  const p = problem.message.params;
  switch (problem.message.code) {
    case "problem.requiresVersion":
    case "problem.requiresMissing":
      return t("graph.needs", { dep: p["dep"] ?? "", range: p["range"] ?? "" });
    case "problem.requiresDisabled":
      return t("graph.needsOnly", { dep: p["dep"] ?? "" });
    case "problem.enginePluginMissing":
    case "problem.enginePluginDisabled":
      return t("graph.needsOnly", { dep: p["plugin"] ?? "" });
    default:
      return t("state.problem");
  }
}

export function GraphView() {
  const { state, actions } = useApp();
  const menu = useProjectMenu();
  const [showAvailable, setShowAvailable] = useState(true);
  const [showPlanned, setShowPlanned] = useState(state.settings.index.showPlanned);
  const [zoom, setZoom] = useState(1);
  const [dragging, setDragging] = useState(false);
  const canvasRef = useRef<HTMLDivElement>(null);
  const drag = useRef<{ x: number; y: number; left: number; top: number; id: number } | null>(null);
  const { problems } = state.snapshot;

  const visible = useMemo(() => state.snapshot.packages.filter((p) =>
    isInstalled(p) || (p.state === "notInstalled" && showAvailable) || (p.state === "planned" && showPlanned)),
  [state.snapshot.packages, showAvailable, showPlanned]);

  const layout = useMemo(() => layoutGraph({
    packages: visible,
    hasProblem: (pkg) => blockingProblem(pkg, problems) !== undefined
  }), [visible, problems]);

  /** Changes the zoom keeping the point under (clientX, clientY), or the viewport centre, in place. */
  const zoomTo = useCallback((next: (current: number) => number, clientX?: number, clientY?: number) => {
    const canvas = canvasRef.current;
    setZoom((current) => {
      const target = clampZoom(next(current));
      if (canvas && target !== current) {
        const rect = canvas.getBoundingClientRect();
        const px = (clientX ?? rect.left + rect.width / 2) - rect.left - 24;
        const py = (clientY ?? rect.top + rect.height / 2) - rect.top - 24;
        const ox = px + canvas.scrollLeft;
        const oy = py + canvas.scrollTop;
        const ratio = target / current;
        requestAnimationFrame(() => {
          canvas.scrollLeft = ox * ratio - px;
          canvas.scrollTop = oy * ratio - py;
        });
      }
      return target;
    });
  }, []);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const onWheel = (e: WheelEvent) => {
      if (!e.ctrlKey) return;
      e.preventDefault();
      zoomTo((z) => z * (e.deltaY < 0 ? 1.1 : 1 / 1.1), e.clientX, e.clientY);
    };
    canvas.addEventListener("wheel", onWheel, { passive: false });
    return () => canvas.removeEventListener("wheel", onWheel);
  }, [zoomTo]);

  const fit = () => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const w = canvas.clientWidth - 48;
    const h = canvas.clientHeight - 48;
    setZoom(clampZoom(Math.min(w / layout.width, h / layout.height)));
    canvas.scrollTo(0, 0);
  };

  const onPointerDown = (e: PointerEvent<HTMLDivElement>) => {
    if (e.button !== 0 || (e.target as HTMLElement).closest(".card")) return;
    const canvas = canvasRef.current;
    if (!canvas) return;
    drag.current = { x: e.clientX, y: e.clientY, left: canvas.scrollLeft, top: canvas.scrollTop, id: e.pointerId };
    canvas.setPointerCapture(e.pointerId);
    setDragging(true);
  };
  const onPointerMove = (e: PointerEvent<HTMLDivElement>) => {
    const d = drag.current;
    const canvas = canvasRef.current;
    if (!d || !canvas) return;
    canvas.scrollLeft = d.left - (e.clientX - d.x);
    canvas.scrollTop = d.top - (e.clientY - d.y);
  };
  const endDrag = () => {
    if (!drag.current) return;
    canvasRef.current?.releasePointerCapture(drag.current.id);
    drag.current = null;
    setDragging(false);
  };

  return (
    <main className="tool-window">
      <div className="tw-header">
        <span className="tw-title">{t("view.graph")}</span>
        <span className="spacer" />
        <OptionsMenu entries={menu} />
      </div>
      <div className="graph-toolbar">
        <label className="check-label compact">
          <input type="checkbox" checked={showAvailable} onChange={() => setShowAvailable(!showAvailable)} />
          {t("graph.available")}
        </label>
        <label className="check-label compact">
          <input type="checkbox" checked={showPlanned} onChange={() => setShowPlanned(!showPlanned)} />
          {t("graph.planned")}
        </label>
        <span className="toolbar-divider" />
        <span className="legend">
          <svg width="22" height="8" viewBox="0 0 22 8" aria-hidden="true">
            <path d="M1 4h15" style={{ stroke: "var(--muted)" }} strokeWidth="1.5" />
            <path d="M15 1l5 3-5 3z" style={{ fill: "var(--muted)" }} />
          </svg>
          {t("graph.legend")}
        </span>
        <span className="spacer" />
        <span className="zoom-group">
          <button type="button" className="zoom-button" aria-label={t("graph.zoomOut")} title={t("graph.zoomOut")} onClick={() => zoomTo((z) => z - ZOOM_STEP)}>−</button>
          <span className="zoom-value">{Math.round(zoom * 100)}%</span>
          <button type="button" className="zoom-button" aria-label={t("graph.zoomIn")} title={t("graph.zoomIn")} onClick={() => zoomTo((z) => z + ZOOM_STEP)}>+</button>
        </span>
        <button type="button" className="fit-button" onClick={fit}>{t("graph.fit")}</button>
      </div>
      <div
        ref={canvasRef}
        className={`graph-canvas${dragging ? " dragging" : ""}`}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={endDrag}
        onPointerCancel={endDrag}
      >
        <div className="graph-stage" style={{ width: layout.width * zoom, height: layout.height * zoom }}>
          <div className="graph-inner" style={{ width: layout.width, height: layout.height, transform: `scale(${zoom})` }}>
            <svg className="graph-edges" width={layout.width} height={layout.height} aria-hidden="true">
              <defs>
                <marker id="graph-arrow" orient="auto" markerWidth="5" markerHeight="5" refX="2" refY="2" overflow="visible">
                  <path d="M0 0 L4 2 L0 4 Z" style={{ fill: "var(--edge)" }} />
                </marker>
                <marker id="graph-arrow-problem" orient="auto" markerWidth="5" markerHeight="5" refX="2" refY="2" overflow="visible">
                  <path d="M0 0 L4 2 L0 4 Z" style={{ fill: "var(--warning)" }} />
                </marker>
              </defs>
              {layout.edges.map((e) => (
                <path
                  key={e.id}
                  d={e.path}
                  fill="none"
                  strokeWidth={2}
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  style={{ stroke: e.problem ? "var(--warning)" : "var(--edge)" }}
                  markerEnd={`url(#${e.problem ? "graph-arrow-problem" : "graph-arrow"})`}
                />
              ))}
            </svg>
            {layout.nodes.map((node) => (
              <GraphCard
                key={node.pkg.name}
                node={node}
                problem={blockingProblem(node.pkg, problems)}
                selected={state.selected !== null && sameName(state.selected, node.pkg.name)}
                onSelect={() => actions.select(node.pkg.name)}
                onOpen={() => {
                  actions.select(node.pkg.name);
                  actions.setView("packages");
                }}
              />
            ))}
            {layout.columns.map((c) => (
              <div key={c.layer} className="layer-label" style={{ left: c.x, top: layout.labelY, width: c.w }}>
                {t(`layer.${c.layer}`)}
              </div>
            ))}
          </div>
        </div>
      </div>
    </main>
  );
}

function GraphCard(props: { node: GraphNode; problem: Problem | undefined; selected: boolean; onSelect: () => void; onOpen: () => void }) {
  const { node, problem } = props;
  const { pkg } = node;
  const layerVar = `--layer-${pkg.layer.toLowerCase()}`;
  const classes = ["card"];
  let sub: { text: string; className: string };
  if (pkg.state === "planned") {
    classes.push("planned");
    sub = { text: t("graph.plannedCard"), className: "sub" };
  } else if (pkg.state === "notInstalled") {
    classes.push("ghost");
    sub = { text: t("graph.notInstalled"), className: "sub c-muted" };
  } else if (problem) {
    classes.push("problem");
    if (problem.severity === "error") classes.push("error");
    sub = { text: problemLine(problem), className: `sub ${problem.severity === "error" ? "c-error" : "c-warning"}` };
  } else if (pkg.state === "disabled") {
    classes.push("disabled");
    sub = { text: t("graph.disabled"), className: "sub c-muted" };
  } else if (pkg.updateAvailable && pkg.installedVersion && pkg.latestVersion) {
    sub = { text: `${pkg.installedVersion} → ${pkg.latestVersion}`, className: "sub mono c-link" };
  } else {
    sub = { text: pkg.installedVersion ?? "", className: "sub mono c-muted" };
  }
  if (node.tall) classes.push("tall");
  if (props.selected) classes.push("selected");
  const topColor = pkg.state === "planned" ? `var(${layerVar}-dim)` : `var(${layerVar})`;
  const Icon = problem?.severity === "error" ? ErrorIcon : WarningIcon;
  return (
    <div
      className={classes.join(" ")}
      style={{ left: node.x, top: node.y, width: node.w, height: node.h, borderTopColor: topColor }}
      title={localized(pkg.description)}
      onClick={props.onSelect}
      onDoubleClick={props.onOpen}
      role="button"
      tabIndex={0}
      onKeyDown={(e) => {
        if (e.key === "Enter") props.onOpen();
        else if (e.key === " ") props.onSelect();
      }}
    >
      {problem && !node.tall && <Icon size={16} strokeWidth={1.4} className={problem.severity === "error" ? "c-error" : "c-warning"} />}
      <span className="card-text">
        <b className="card-name">{pkg.name}</b>
        <span className={sub.className}>{sub.text}</span>
        {node.tall && <span className="tall-note c-muted">{t("graph.requiredByAll")}</span>}
      </span>
    </div>
  );
}
