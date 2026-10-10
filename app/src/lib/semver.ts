// Version parsing and constraint matching for the in-memory mock backend. Follows the grammar of
// uBotCore's UBotSemVer (lenient parse, `= >= > <= < ^ ~`, X-range partials, upper bounds that exclude
// the bound's prereleases); the Rust core is the authoritative implementation.

export interface SemVer { major: number; minor: number; patch: number; pre: string }

interface Parsed { version: SemVer; count: number }

function parseWithCount(input: string): Parsed | null {
  let text = input.trim();
  if (/^v/i.test(text)) text = text.slice(1);
  const plus = text.indexOf("+");
  if (plus >= 0) text = text.slice(0, plus);
  let pre = "";
  const dash = text.indexOf("-");
  if (dash >= 0) {
    pre = text.slice(dash + 1);
    text = text.slice(0, dash);
    if (!/^[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*$/.test(pre)) return null;
  }
  const parts = text.split(".");
  if (parts.length < 1 || parts.length > 3 || parts.some((p) => !/^\d+$/.test(p))) return null;
  const [major = 0, minor = 0, patch = 0] = parts.map(Number);
  return { version: { major, minor, patch, pre }, count: parts.length };
}

export function parseVersion(text: string): SemVer | null {
  return parseWithCount(text)?.version ?? null;
}

function compareRelease(a: SemVer, b: SemVer): number {
  return Math.sign(a.major - b.major) || Math.sign(a.minor - b.minor) || Math.sign(a.patch - b.patch);
}

function comparePre(a: string, b: string): number {
  if (a === b) return 0;
  if (!a) return 1;
  if (!b) return -1;
  const ia = a.split(".");
  const ib = b.split(".");
  for (let i = 0; i < Math.min(ia.length, ib.length); i++) {
    const x = ia[i] ?? "";
    const y = ib[i] ?? "";
    const nx = /^\d+$/.test(x);
    const ny = /^\d+$/.test(y);
    if (nx && ny) {
      const d = Math.sign(Number(x) - Number(y));
      if (d) return d;
    } else if (nx !== ny) {
      return nx ? -1 : 1;
    } else if (x !== y) {
      return x < y ? -1 : 1;
    }
  }
  return Math.sign(ia.length - ib.length);
}

function compareVersions(a: SemVer, b: SemVer): number {
  return compareRelease(a, b) || comparePre(a.pre, b.pre);
}

/** Compares two version strings; unparsable versions sort first. */
export function compareVersionStrings(a: string, b: string): number {
  const pa = parseVersion(a);
  const pb = parseVersion(b);
  if (!pa || !pb) return pa ? 1 : pb ? -1 : a.localeCompare(b);
  return compareVersions(pa, pb);
}

type Op = ">=" | ">" | "<=" | "<" | "=" | "<rel";
interface Comparator { op: Op; version: SemVer }

function release(major: number, minor: number, patch: number): SemVer {
  return { major, minor, patch, pre: "" };
}

function parseComparator(token: string): Comparator[] | null {
  const match = /^(>=|<=|>|<|=|\^|~)?(.*)$/.exec(token);
  if (!match) return null;
  const op = match[1] ?? "";
  const rest = match[2] ?? "";
  if (/^[*xX]$/.test(rest) && (op === "" || op === "=")) return [];
  const parsed = parseWithCount(rest.replace(/\.[*xX]$/g, "").replace(/\.[*xX]$/g, ""));
  if (!parsed) return null;
  const { version: v, count } = parsed;
  switch (op) {
    case ">=": case ">": case "<=": case "<":
      return [{ op, version: v }];
    case "^": {
      const upper = v.major > 0 || count === 1 ? release(v.major + 1, 0, 0)
        : v.minor > 0 || count === 2 ? release(0, v.minor + 1, 0)
          : release(0, 0, v.patch + 1);
      return [{ op: ">=", version: v }, { op: "<rel", version: upper }];
    }
    case "~": {
      const upper = count >= 2 ? release(v.major, v.minor + 1, 0) : release(v.major + 1, 0, 0);
      return [{ op: ">=", version: v }, { op: "<rel", version: upper }];
    }
    default:
      if (count === 3) return [{ op: "=", version: v }];
      return [
        { op: ">=", version: v },
        { op: "<rel", version: count === 1 ? release(v.major + 1, 0, 0) : release(v.major, v.minor + 1, 0) }
      ];
  }
}

function test(c: Comparator, v: SemVer): boolean {
  const d = compareVersions(v, c.version);
  switch (c.op) {
    case ">=": return d >= 0;
    case ">": return d > 0;
    case "<=": return d <= 0;
    case "<": return d < 0;
    case "=": return d === 0;
    case "<rel": return compareRelease(v, c.version) < 0;
  }
}

/** True when `version` satisfies every whitespace-separated comparator of `constraint`. */
export function satisfies(version: string, constraint: string): boolean {
  const v = parseVersion(version);
  if (!v) return false;
  const tokens = constraint.trim().split(/\s+/).filter(Boolean);
  for (const token of tokens) {
    const comparators = parseComparator(token);
    if (!comparators) return false;
    if (!comparators.every((c) => test(c, v))) return false;
  }
  return true;
}
