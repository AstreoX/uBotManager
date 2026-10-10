import type { Layer, PackageInfo } from "../api/types";

// Layered dependency graph: one column per layer, cards in rows, orthogonal edges from a dependency to
// its dependents ("A → B: B depends on A"). Geometry follows Graph.dc.html.

const LAYERS: Layer[] = ["Foundation", "Capability", "Adapter", "Content"];

const MARGIN = 48;
const COLUMN_GAP = 96;
const CARD_W = 200;
const TALL_W = 160;
const CARD_H = 72;
const ROW_PITCH = 120;
const TOP = 48;
/** Spacing of the lanes above the first row that carry edges past cards in between. */
const LANE_PITCH = 8;
const ARROW = 5;
/** A port keeps this far from the top and bottom edge of its card. */
const PORT_MARGIN = 8;
/** Horizontal runs of different edges that overlap in x keep at least this far apart in y. */
const TRACK = 8;

export interface GraphNode {
  pkg: PackageInfo;
  x: number; y: number; w: number; h: number;
  col: number; row: number;
  tall: boolean;
}

export interface GraphEdge {
  id: string;
  from: string;
  to: string;
  path: string;
  problem: boolean;
}

export interface GraphColumn { layer: Layer; x: number; w: number }

export interface GraphLayout {
  nodes: GraphNode[];
  edges: GraphEdge[];
  columns: GraphColumn[];
  labelY: number;
  width: number;
  height: number;
  /** Foundation package drawn as one tall card when every other package requires it. */
  collapsed: string | null;
}

export interface GraphInput {
  packages: PackageInfo[];
  /** Installed package with a warning or error. */
  hasProblem: (pkg: PackageInfo) => boolean;
}

function rank(pkg: PackageInfo, problem: boolean): number {
  if (pkg.state === "notInstalled") return 1;
  if (pkg.state === "planned") return 2;
  return problem ? 3 : 0;
}

type Point = [number, number];

interface RawEdge { id: string; src: GraphNode; dst: GraphNode; problem: boolean; route: "through" | "lane" | "back" }

/** The horizontal run `points[at]`→`points[at + 1]` of a route that touches `node`; its y is the port's. */
interface Stub { at: number; node: GraphNode }

interface Route { raw: RawEdge; points: Point[]; stubs: Stub[] }

interface Run { owner: number; y: number; lo: number; hi: number }

function runAt(points: Point[], at: number, owner: number): Run | null {
  const a = points[at];
  const b = points[at + 1];
  if (!a || !b || a[1] !== b[1] || a[0] === b[0]) return null;
  return { owner, y: a[1], lo: Math.min(a[0], b[0]), hi: Math.max(a[0], b[0]) };
}

/** Distance from `y` to the nearest run of another edge that shares part of the x range of `run`. */
function clearance(runs: Run[], run: Run, y: number): number {
  let nearest = Infinity;
  for (const other of runs) {
    if (other.owner !== run.owner && Math.min(other.hi, run.hi) - Math.max(other.lo, run.lo) > 0.5) {
      nearest = Math.min(nearest, Math.abs(other.y - y));
    }
  }
  return nearest;
}

/**
 * Where the run goes: its own y when other edges keep `TRACK` away, else the y on the card (inside the port
 * range `top`..`bottom`) that is clear of them, nearest to its own; on a crowded card, the one with the most room.
 * `toward` is where the edge's vertical leads: moving that way keeps the vertical from crossing the run it made way for.
 */
function freeY(run: Run, top: number, bottom: number, runs: Run[], toward: number): number {
  const score = (y: number) => Math.min(clearance(runs, run, y), TRACK);
  const cost = (y: number) => Math.abs(y - run.y) + (Math.sign(y - run.y) === -Math.sign(toward - run.y) ? 0.5 : 0);
  let best = run.y;
  let bestScore = score(best);
  if (bestScore >= TRACK) return best;
  for (let y = Math.ceil(top); y <= bottom; y++) {
    const s = score(y);
    if (s > bestScore || (s === bestScore && cost(y) < cost(best))) {
      best = y;
      bestScore = s;
    }
  }
  return best;
}

/**
 * Ports are spread per card, without regard to the other cards, so an edge leaving a card and an edge
 * entering the next column can end up at the same height in the same gap and read as one wire (a false
 * dependency). Moves the stub of such an edge along its card to where the horizontal runs of other edges are at
 * least `TRACK` away, or as far from them as the card allows. A stub only moves when that gives it more room.
 * Straight edges and runs away from the cards stay where they are.
 */
function separateStubs(routes: Route[]): void {
  const runs: Run[] = [];
  const movable: { run: Run; route: Route; stub: Stub }[] = [];
  routes.forEach((route, owner) => {
    route.points.forEach((_, at) => {
      const run = runAt(route.points, at, owner);
      if (!run) return;
      runs.push(run);
      const stub = route.stubs.find((s) => s.at === at);
      if (stub) movable.push({ run, route, stub });
    });
  });
  for (const { run, route, stub } of movable) {
    const toward = (stub.at === 0 ? route.points[2] : route.points[stub.at - 1])?.[1] ?? run.y;
    const y = freeY(run, stub.node.y + PORT_MARGIN, stub.node.y + stub.node.h - PORT_MARGIN, runs, toward);
    if (y === run.y) continue;
    run.y = y;
    const a = route.points[stub.at];
    const b = route.points[stub.at + 1];
    if (a && b) {
      a[1] = y;
      b[1] = y;
    }
  }
}

function toPath(points: Point[]): string {
  const clean: Point[] = [];
  for (const p of points) {
    const last = clean[clean.length - 1];
    if (!last || last[0] !== p[0] || last[1] !== p[1]) clean.push(p);
  }
  // Leave room for the arrow head on the last segment.
  const end = clean[clean.length - 1];
  const before = clean[clean.length - 2];
  if (end && before) {
    const dx = Math.sign(end[0] - before[0]);
    const dy = Math.sign(end[1] - before[1]);
    clean[clean.length - 1] = [end[0] - dx * ARROW, end[1] - dy * ARROW];
  }
  return clean.map((p, i) => `${i === 0 ? "M" : "L"} ${p[0]} ${p[1]}`).join(" ");
}

export function layoutGraph({ packages, hasProblem }: GraphInput): GraphLayout {
  const layers: Layer[] = packages.some((p) => p.layer === "Unknown") ? [...LAYERS, "Unknown"] : [...LAYERS];
  const byName = new Map(packages.map((p) => [p.name.toLowerCase(), p]));
  const problems = new Map(packages.map((p) => [p.name, hasProblem(p)]));
  const columnsPkgs = layers.map((layer) =>
    packages
      .filter((p) => p.layer === layer)
      .map((p, i) => ({ p, i }))
      .sort((a, b) => rank(a.p, problems.get(a.p.name) ?? false) - rank(b.p, problems.get(b.p.name) ?? false) || a.i - b.i)
      .map((x) => x.p));

  const foundation = columnsPkgs[0] ?? [];
  const onlyFoundation = foundation.length === 1 ? foundation[0] : undefined;
  const others = packages.filter((p) => p !== onlyFoundation);
  const collapsed = onlyFoundation && others.length > 0 && others.every((p) =>
    p.requires.some((r) => r.kind === "package" && r.name.toLowerCase() === onlyFoundation.name.toLowerCase()))
    ? onlyFoundation.name : null;

  const rows = Math.max(1, ...columnsPkgs.map((c, i) => (i === 0 && collapsed ? 1 : c.length)));
  const columns: GraphColumn[] = [];
  let x = MARGIN;
  layers.forEach((layer, i) => {
    const w = i === 0 && collapsed ? TALL_W : CARD_W;
    columns.push({ layer, x, w });
    x += w + COLUMN_GAP;
  });

  const nodes: GraphNode[] = [];
  columnsPkgs.forEach((list, col) => {
    const column = columns[col];
    if (!column) return;
    list.forEach((pkg, row) => {
      const tall = collapsed === pkg.name;
      nodes.push({
        pkg, col, row, tall,
        x: column.x, y: TOP + row * ROW_PITCH, w: column.w,
        h: tall ? rows * ROW_PITCH - (ROW_PITCH - CARD_H) : CARD_H
      });
    });
  });
  const nodeOf = new Map(nodes.map((n) => [n.pkg.name, n]));

  // Collect edges: dependency → dependent.
  const raw: RawEdge[] = [];
  for (const dst of nodes) {
    for (const r of dst.pkg.requires) {
      if (r.kind !== "package" || r.optional) continue;
      const depPkg = byName.get(r.name.toLowerCase());
      if (!depPkg || depPkg.name === collapsed) continue;
      const src = nodeOf.get(depPkg.name);
      if (!src) continue;
      const dependentInstalled = dst.pkg.state === "enabled";
      const problem = (problems.get(src.pkg.name) ?? false) || (dependentInstalled && r.status !== "satisfied");
      let route: RawEdge["route"] = "through";
      if (dst.col <= src.col) route = "back";
      else if (dst.col > src.col + 1) {
        const blocked = nodes.some((n) => n.col > src.col && n.col < dst.col && n.y < src.y + src.h && n.y + n.h > src.y);
        if (blocked) route = "lane";
      }
      raw.push({ id: `${src.pkg.name}->${dst.pkg.name}`, src, dst, problem, route });
    }
  }

  // Every lane needs its own track above the cards (the first one 24 px above them, the last at least 6 px
  // from the top); with more than three lanes the cards move down to make room.
  const laneCount = raw.filter((e) => e.route === "lane").length;
  const top = Math.max(TOP, 30 + (laneCount - 1) * LANE_PITCH);
  for (const node of nodes) node.y += top - TOP;

  // Right-side exit ports, ordered by where the edge goes.
  const exitY = new Map<string, number>();
  for (const node of nodes) {
    const outs = raw.filter((e) => e.src === node && (e.route !== "lane" || node.row > 0))
      .sort((a, b) => a.dst.y - b.dst.y || a.dst.x - b.dst.x);
    outs.forEach((e, i) => exitY.set(e.id, node.y + (node.h * (i + 1)) / (outs.length + 1)));
  }
  // Straight edges: the exit port already lies within the target card.
  const straight = new Set(raw.filter((e) => {
    const y = exitY.get(e.id);
    return e.route === "through" && y !== undefined && y >= e.dst.y + 8 && y <= e.dst.y + e.dst.h - 8;
  }).map((e) => e.id));
  // Left-side entry ports for the remaining edges. An edge arriving from the top lane takes the first slot
  // without using it, so side entries stay clear of it.
  const entryY = new Map<string, number>();
  for (const node of nodes) {
    const fromTop = (e: RawEdge) => e.route === "lane" && node.row === 0;
    const ins = raw.filter((e) => e.dst === node && !straight.has(e.id))
      .sort((a, b) => (fromTop(a) ? -Infinity : exitY.get(a.id) ?? a.src.y) - (fromTop(b) ? -Infinity : exitY.get(b.id) ?? b.src.y));
    ins.forEach((e, i) => {
      if (!fromTop(e)) entryY.set(e.id, node.y + (node.h * (i + 1)) / (ins.length + 1));
    });
  }
  // Vertical channels in the gap before each column; parallel segments are spread apart.
  const channelUse = new Map<number, string[]>();
  const useChannel = (gapCol: number, id: string) => {
    const list = channelUse.get(gapCol) ?? [];
    list.push(id);
    channelUse.set(gapCol, list);
  };
  for (const e of raw) {
    if (straight.has(e.id)) continue;
    if (e.route === "through") useChannel(e.dst.col, e.id);
    else if (e.route === "lane") {
      if (e.src.row > 0) useChannel(e.src.col + 1, `${e.id}#out`);
      if (e.dst.row > 0) useChannel(e.dst.col, e.id);
    } else useChannel(e.src.col + 1, e.id);
  }
  const channelX = (gapCol: number, id: string): number => {
    const column = columns[gapCol];
    const prev = columns[gapCol - 1];
    const mid = column && prev ? (prev.x + prev.w + column.x) / 2
      : prev ? prev.x + prev.w + MARGIN / 2 // right of the last column
        : (column?.x ?? MARGIN) - COLUMN_GAP / 2;
    const list = channelUse.get(gapCol) ?? [id];
    const i = Math.max(0, list.indexOf(id));
    return mid + (i - (list.length - 1) / 2) * 8;
  };

  let lanes = 0;
  const topEntries = new Map<GraphNode, number>();
  const topExits = new Map<GraphNode, number>();
  const routes: Route[] = raw.map((e) => {
    const { src, dst } = e;
    const sy = exitY.get(e.id) ?? src.y + src.h / 2;
    let points: Point[];
    const stubs: Stub[] = [];
    if (straight.has(e.id)) {
      points = [[src.x + src.w, sy], [dst.x, sy]];
    } else if (e.route === "through") {
      const cx = channelX(dst.col, e.id);
      const ty = entryY.get(e.id) ?? dst.y + dst.h / 2;
      points = [[src.x + src.w, sy], [cx, sy], [cx, ty], [dst.x, ty]];
      stubs.push({ at: 0, node: src }, { at: 2, node: dst });
    } else if (e.route === "lane") {
      const laneY = top - 24 - lanes++ * LANE_PITCH;
      points = [];
      if (src.row === 0) {
        const k = topExits.get(src) ?? 0;
        topExits.set(src, k + 1);
        const sx = src.x + src.w / 2 + k * 12;
        points.push([sx, src.y], [sx, laneY]);
      } else {
        const cx = channelX(src.col + 1, `${e.id}#out`);
        points.push([src.x + src.w, sy], [cx, sy], [cx, laneY]);
        stubs.push({ at: 0, node: src });
      }
      if (dst.row === 0) {
        const k = topEntries.get(dst) ?? 0;
        topEntries.set(dst, k + 1);
        const tx = dst.x + dst.w / 2 + k * 12;
        points.push([tx, laneY], [tx, dst.y]);
      } else {
        const cx = channelX(dst.col, e.id);
        const ty = entryY.get(e.id) ?? dst.y + dst.h / 2;
        points.push([cx, laneY], [cx, ty], [dst.x, ty]);
        stubs.push({ at: points.length - 2, node: dst });
      }
    } else {
      // Same column or a dependency on a later layer: loop through the gap right of the source.
      const cx = channelX(src.col + 1, e.id);
      const ty = entryY.get(e.id) ?? dst.y + dst.h / 2;
      points = [[src.x + src.w, sy], [cx, sy], [cx, ty], [dst.x + dst.w, ty]];
      stubs.push({ at: 0, node: src }, { at: 2, node: dst });
    }
    return { raw: e, points, stubs };
  });
  separateStubs(routes);
  const edges: GraphEdge[] = routes.map(({ raw: e, points }) => (
    { id: e.id, from: e.src.pkg.name, to: e.dst.pkg.name, path: toPath(points), problem: e.problem }));

  const labelY = top + rows * ROW_PITCH - (ROW_PITCH - CARD_H) + 16;
  const last = columns[columns.length - 1];
  return {
    nodes, edges, columns, labelY, collapsed,
    width: (last ? last.x + last.w : 0) + MARGIN,
    height: labelY + 16 + 16
  };
}
