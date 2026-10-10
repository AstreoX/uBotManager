import type { Settings } from "../api/types";

export function sameJson(a: unknown, b: unknown): boolean {
  return JSON.stringify(a) === JSON.stringify(b);
}

/** A TCP port typed in the ROS section: digits only, 1-65535. `null` when it is not one. */
export function parsePort(text: string): number | null {
  const trimmed = text.trim();
  if (!/^\d{1,5}$/.test(trimmed)) return null;
  const port = Number(trimmed);
  return port >= 1 && port <= 65535 ? port : null;
}

function isPlainObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function rebase(draft: unknown, base: unknown, fresh: unknown): unknown {
  if (sameJson(draft, base)) return fresh;
  if (isPlainObject(draft) && isPlainObject(base) && isPlainObject(fresh)) {
    const merged: Record<string, unknown> = { ...fresh };
    for (const key of Object.keys(draft)) {
      const value = rebase(draft[key], base[key], fresh[key]);
      if (value === undefined) delete merged[key];
      else merged[key] = value;
    }
    return merged;
  }
  return draft;
}

/**
 * Carries a settings draft over to settings the backend changed behind the dialog (a second launch
 * switched the project, UE edited DefaultUBotROS.ini): values the user has not edited (draft equals `base`,
 * what the draft started from) follow `fresh`, edited ones stay. Objects merge key by key, lists are
 * replaced as a whole.
 */
export function rebaseSettings(draft: Settings, base: Settings, fresh: Settings): Settings {
  return rebase(draft, base, fresh) as Settings;
}
