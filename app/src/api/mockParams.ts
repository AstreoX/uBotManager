import type { Lang, SettingsSection } from "./types";

export type ViewId = "packages" | "graph" | "problems" | "tasks";
export type DialogParam = "recipes" | "plan" | "settings";

/** Mock-only URL parameters used for screenshots (SPEC §11). Ignored inside Tauri. */
export interface MockParams {
  lang: Lang | null;
  theme: "dark" | "light" | null;
  view: ViewId | null;
  dialog: DialogParam | null;
  section: SettingsSection | null;
  select: string | null;
  noProject: boolean;
}

function pick<T extends string>(value: string | null, allowed: readonly T[]): T | null {
  return value !== null && (allowed as readonly string[]).includes(value) ? (value as T) : null;
}

export function readMockParams(search: string): MockParams {
  const q = new URLSearchParams(search);
  return {
    lang: pick(q.get("lang"), ["en", "zh-CN"] as const),
    theme: pick(q.get("theme"), ["dark", "light"] as const),
    view: pick(q.get("view"), ["packages", "graph", "problems", "tasks"] as const),
    dialog: pick(q.get("dialog"), ["recipes", "plan", "settings"] as const),
    section: pick(q.get("section"), ["appearance", "index", "install", "build", "ros"] as const),
    select: q.get("select"),
    noProject: q.get("noproject") === "1"
  };
}
