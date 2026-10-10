import type { Lang, LocalizedText, Msg } from "../api/types";
import en from "./en.json";
import zhCN from "./zh-CN.json";

/**
 * Locale files are two-level: a section object whose keys are the rest of the dotted key, so
 * `msg.step.clone.detail` lives at `json["msg"]["step.clone.detail"]`. Backend message codes may contain
 * dots, which is why sections are not nested further.
 */
export type LocaleTable = Record<string, Record<string, string>>;

export const locales: Record<Lang, LocaleTable> = { en, "zh-CN": zhCN };

let current: Lang = "en";

export function setLanguage(lang: Lang): void {
  current = lang;
  if (typeof document !== "undefined") document.documentElement.lang = lang;
}

export function getLanguage(): Lang {
  return current;
}

export function lookup(table: LocaleTable, key: string): string | undefined {
  const dot = key.indexOf(".");
  if (dot <= 0) return undefined;
  return table[key.slice(0, dot)]?.[key.slice(dot + 1)];
}

/** Replaces `{name}` placeholders; unknown placeholders are left as written. */
export function format(template: string, params: Record<string, string | number> = {}): string {
  return template.replace(/\{(\w+)\}/g, (whole, name: string) =>
    Object.prototype.hasOwnProperty.call(params, name) ? String(params[name]) : whole);
}

/** Missing in the language → English → the key itself. */
export function translateIn(tables: Record<Lang, LocaleTable>, lang: Lang, key: string, params?: Record<string, string | number>): string {
  const template = lookup(tables[lang], key) ?? lookup(tables.en, key);
  return template === undefined ? key : format(template, params);
}

export function translate(lang: Lang, key: string, params?: Record<string, string | number>): string {
  return translateIn(locales, lang, key, params);
}

export function hasKey(key: string): boolean {
  return lookup(locales[current], key) !== undefined || lookup(locales.en, key) !== undefined;
}

export function t(key: string, params?: Record<string, string | number>): string {
  return translate(current, key, params);
}

/** Picks `<key>.one` for 1 and `<key>.other` otherwise; `{n}` is the count. */
export function tn(key: string, n: number, params: Record<string, string | number> = {}): string {
  return t(`${key}.${n === 1 ? "one" : "other"}`, { n, ...params });
}

export function renderMsg(lang: Lang, msg: Msg): string {
  const key = `msg.${msg.code}`;
  if (lookup(locales[lang], key) === undefined && lookup(locales.en, key) === undefined) {
    return msg.params["message"] ?? msg.code;
  }
  return translate(lang, key, msg.params);
}

export function tm(msg: Msg): string {
  return renderMsg(current, msg);
}

interface EnglishTemplate { code: string; names: string[]; pattern: RegExp; literal: number }

/** Backend rejections are English text rendered from these message codes (SPEC §7: errors reject with a string). */
const REJECTION_PREFIXES = ["plan.", "task."];

let englishTemplates: EnglishTemplate[] | null = null;

function escapeRegExp(text: string): string {
  return text.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

function rejectionTemplates(): EnglishTemplate[] {
  if (englishTemplates) return englishTemplates;
  const list: EnglishTemplate[] = [];
  for (const [code, template] of Object.entries(locales.en["msg"] ?? {})) {
    if (!REJECTION_PREFIXES.some((prefix) => code.startsWith(prefix))) continue;
    // split() with a capture group alternates literal text and placeholder names.
    const parts = template.split(/\{(\w+)\}/);
    const names: string[] = [];
    let source = "";
    let literal = 0;
    parts.forEach((part, i) => {
      if (i % 2 === 0) {
        source += escapeRegExp(part);
        literal += part.length;
      } else {
        names.push(part);
        source += "(.+?)";
      }
    });
    if (literal === 0) continue;
    list.push({ code, names, pattern: new RegExp(`^${source}$`), literal });
  }
  // The most specific template wins when several could match.
  englishTemplates = list.sort((a, b) => b.literal - a.literal);
  return englishTemplates;
}

/**
 * The message a rendered English rejection came from, so it can be shown in the UI language.
 * Only `plan.*` and `task.*` codes are recognised; anything else (git output, internal errors) stays as is.
 */
export function msgFromEnglish(text: string): Msg | null {
  const trimmed = text.trim();
  for (const { code, names, pattern } of rejectionTemplates()) {
    const match = pattern.exec(trimmed);
    if (!match) continue;
    const params: Record<string, string> = {};
    names.forEach((name, i) => { params[name] = match[i + 1] ?? ""; });
    return { code, params };
  }
  return null;
}

export function localized(text: LocalizedText): string {
  return (current === "zh-CN" ? text["zh-CN"] : null) ?? text.en;
}
