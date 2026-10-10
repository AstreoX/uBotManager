import { describe, expect, it } from "vitest";
import { createMockBackend } from "../api/mock";
import type { Layer, PackageInfo, Requirement, Snapshot } from "../api/types";
import { layoutGraph, type GraphLayout } from "./graphLayout";

type Seg = { edge: string; a: [number, number]; b: [number, number] };

/** The segments of every edge, from the `M x y L x y …` path strings. */
function segments(layout: GraphLayout): Seg[] {
  const out: Seg[] = [];
  for (const edge of layout.edges) {
    const points = [...edge.path.matchAll(/(-?\d+(?:\.\d+)?) (-?\d+(?:\.\d+)?)/g)].map((m): [number, number] => [Number(m[1]), Number(m[2])]);
    for (let i = 0; i + 1 < points.length; i++) out.push({ edge: edge.id, a: points[i]!, b: points[i + 1]! });
  }
  return out;
}

const overlap = (a0: number, a1: number, b0: number, b1: number) =>
  Math.min(Math.max(a0, a1), Math.max(b0, b1)) - Math.max(Math.min(a0, a1), Math.min(b0, b1));

/** Pairs of segments of different edges that lie on top of each other, or horizontal ones closer than `minGap`. */
function collisions(layout: GraphLayout, minGap = 8): string[] {
  const segs = segments(layout);
  const found: string[] = [];
  for (let i = 0; i < segs.length; i++) {
    for (let j = i + 1; j < segs.length; j++) {
      const p = segs[i]!;
      const q = segs[j]!;
      if (p.edge === q.edge) continue;
      const pHorizontal = p.a[1] === p.b[1];
      const qHorizontal = q.a[1] === q.b[1];
      if (pHorizontal && qHorizontal && Math.abs(p.a[1] - q.a[1]) < minGap && overlap(p.a[0], p.b[0], q.a[0], q.b[0]) > 0.5) {
        found.push(`${p.edge} / ${q.edge}: horizontal at y ${p.a[1]} and ${q.a[1]}`);
      }
      const pVertical = p.a[0] === p.b[0];
      const qVertical = q.a[0] === q.b[0];
      if (pVertical && qVertical && Math.abs(p.a[0] - q.a[0]) < 1 && overlap(p.a[1], p.b[1], q.a[1], q.b[1]) > 0.5) {
        found.push(`${p.edge} / ${q.edge}: vertical at x ${p.a[0]}`);
      }
    }
  }
  return found;
}

/** Path points that lie outside the canvas. */
function outside(layout: GraphLayout): string[] {
  return segments(layout)
    .flatMap((seg) => [seg.a, seg.b].map((p) => ({ edge: seg.edge, p })))
    .filter(({ p }) => p[0] < 0 || p[1] < 0 || p[0] > layout.width || p[1] > layout.height)
    .map(({ edge, p }) => `${edge} at ${p[0]},${p[1]}`);
}

function layoutOf(snapshot: Snapshot): GraphLayout {
  return layoutGraph({
    packages: snapshot.packages,
    hasProblem: (p) => p.state !== "notInstalled" && p.state !== "planned"
      && snapshot.problems.some((x) => p.problemIds.includes(x.id) && x.severity !== "info")
  });
}

describe("layoutGraph edges", () => {
  // Without a project every package is "not installed": UBotEnvironment's edge to UBotCombat leaves the card at the
  // height where UBotSensor's edge enters UBotROS, and the two wires read as one.
  it("keep clear of each other in the mock scenario, with a project and without", async () => {
    for (const noProject of [false, true]) {
      const { backend } = createMockBackend({
        lang: null, theme: null, view: null, dialog: null, section: null, select: null, noProject, prebuilt: true
      });
      const layout = layoutOf(await backend.getSnapshot());
      expect(layout.edges.length).toBeGreaterThan(0);
      expect(collisions(layout), `noProject=${noProject}`).toEqual([]);
    }
  });

  it("never overlap in random layered graphs", () => {
    let seed = 20261010;
    const random = () => {
      seed = (Math.imul(seed, 1664525) + 1013904223) >>> 0;
      return seed / 2 ** 32;
    };
    const layers: Layer[] = ["Foundation", "Capability", "Adapter", "Content"];
    const states: PackageInfo["state"][] = ["enabled", "disabled", "notInstalled", "planned"];
    const failures: string[] = [];
    for (let round = 0; round < 500; round++) {
      // Packages in layer order, each requiring up to three packages of lower layers (a package of the same
      // layer now and then): the shape of real indexes, from a single card up to a crowded graph.
      const count = 3 + Math.floor(random() * 8);
      const layerOf = Array.from({ length: count }, (_, i) => Math.min(3, Math.floor((i / count) * 4 + random() * 0.8)));
      const packages: PackageInfo[] = layerOf.map((layer, i) => {
        const candidates = layerOf.map((l, j) => ({ l, j })).filter(({ l, j }) => j !== i && (l < layer || (l === layer && j < i && random() < 0.1)));
        const requires: Requirement[] = candidates
          .filter(() => random() < 0.5)
          .slice(0, 3)
          .map(({ j }) => ({ name: `P${j}`, constraint: "^1", optional: false, kind: "package", status: "satisfied", installedVersion: null }));
        return {
          name: `P${i}`, friendlyName: `P${i}`, description: { en: "", "zh-CN": null }, layer: layers[layer]!,
          state: states[Math.floor(random() * 4)]!, installedVersion: null, latestVersion: null, versions: [], plannedVersion: null,
          updateAvailable: false, location: null, git: null, installKind: null, prebuiltAvailable: false, prebuilt: null, repository: null,
          webUrl: null, docsUrl: null, indexSource: null,
          requires, provides: [], tags: [], problemIds: []
        };
      });
      const layout = layoutGraph({ packages, hasProblem: (p) => p.name.length > 2 && random() < 0.2 });
      const found = collisions(layout, 4);
      if (found.length > 0) failures.push(`round ${round}: ${found[0]}`);
      const off = outside(layout);
      if (off.length > 0) failures.push(`round ${round}: outside the canvas: ${off[0]}`);
    }
    expect(failures).toEqual([]);
  });
});
