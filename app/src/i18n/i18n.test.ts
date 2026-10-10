import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import type { Lang } from "../api/types";
import en from "./en.json";
import zhCN from "./zh-CN.json";
import { format, msgFromEnglish, renderMsg, setLanguage, tn, translate, translateIn, type LocaleTable } from "./index";

/** Message codes of docs/SPEC.md §8. Every code the backend emits must exist in both locale files. */
const SPEC_CODES = [
  "problem.requiresVersion", "problem.requiresMissing", "problem.requiresDisabled", "problem.enginePluginMissing",
  "problem.enginePluginDisabled", "problem.legacyConflict", "problem.descriptorInvalid", "problem.dependencyCycle",
  "problem.engineNotFound", "problem.indexSourceFailed", "problem.rosBridgeUnreachable", "problem.prebuiltEngineMismatch",
  "fact.package", "fact.requires", "fact.installed", "fact.legacy", "fact.replacement", "fact.redirects", "fact.source",
  "fact.address", "fact.time", "fact.plugin", "fact.engine", "fact.file",
  "session.lastPie", "session.lastEditor", "session.lastGame",
  "fix.updateTo", "fix.install", "fix.enable", "fix.disable", "fix.ignore", "fix.openRosSettings", "fix.openBuildSettings",
  "fix.reinstall",
  "step.clone", "step.clone.detail", "step.checkout", "step.checkout.detail", "step.download", "step.download.detail",
  "step.enablePackage", "step.disablePackage",
  "step.enableEnginePlugin", "step.removePackage", "step.removePackage.detail", "step.detail.project",
  "step.detail.requiredBy", "step.build", "step.build.detail", "step.runTests", "step.runTests.detail", "step.openEditor",
  "step.openEditor.detail",
  "plan.notInIndex", "plan.planned", "plan.noVersion", "plan.notInstalled", "plan.alreadyInstalled", "plan.notGit",
  "plan.dirty", "plan.onBranch", "plan.breaksDependent", "plan.alsoDisable", "plan.enginePluginMissing",
  "plan.folderExists", "plan.unsafeUrl", "plan.noEngine", "plan.engineVersion", "plan.outsideProject", "plan.pluginRoot",
  "plan.notPluginDir", "plan.gitUnreadable", "plan.unsafeRef", "plan.unsafeName", "plan.unsafeProjectName",
  "plan.unsafeBuildPath", "plan.projectReadOnly", "plan.noProject", "plan.prebuiltUnavailable", "plan.noPrebuilt",
  "plan.noCompiler",
  "task.apply", "task.recipe", "task.install", "task.busy", "task.editorRunning",
  // Verbatim text (session messages, anything the core has no code for), shown as it is.
  "text"
];

/** Placeholders each §8 template must use (from the parameter lists in the spec table). */
const SPEC_PARAMS: Record<string, string[]> = {
  "problem.requiresVersion": ["dep", "range", "installed"],
  "problem.requiresMissing": ["dep", "range"],
  "problem.requiresDisabled": ["dep"],
  "problem.enginePluginMissing": ["plugin"],
  "problem.legacyConflict": ["legacy", "replacement"],
  "problem.rosBridgeUnreachable": ["address"],
  "problem.prebuiltEngineMismatch": ["package", "built", "engine"],
  "fix.reinstall": ["package"],
  "fix.updateTo": ["package", "version"],
  "step.clone": ["package", "version"],
  "step.clone.detail": ["repo", "dest"],
  "step.download": ["package", "version"],
  "step.download.detail": ["source", "dest"],
  "step.build": ["target"],
  "step.build.detail": ["platform", "configuration"],
  "plan.breaksDependent": ["package", "dependent", "range"],
  "plan.engineVersion": ["package", "version", "range", "engine"],
  "plan.pluginRoot": ["path"],
  "plan.notPluginDir": ["path"],
  "plan.gitUnreadable": ["package", "error"],
  "plan.unsafeRef": ["package", "ref"],
  "plan.unsafeName": ["package"],
  "plan.unsafeProjectName": ["name"],
  "plan.unsafeBuildPath": ["path"],
  "plan.projectReadOnly": ["file"],
  "plan.prebuiltUnavailable": ["package", "version", "engine"],
  "plan.noPrebuilt": ["package", "version", "engine"],
  "plan.noCompiler": [],
  "task.recipe": ["recipe", "project"],
  "text": ["text"]
};

function flatten(table: LocaleTable): Map<string, string> {
  const out = new Map<string, string>();
  for (const [section, entries] of Object.entries(table)) {
    for (const [rest, value] of Object.entries(entries)) out.set(`${section}.${rest}`, value);
  }
  return out;
}

function placeholders(text: string): string[] {
  return [...text.matchAll(/\{(\w+)\}/g)].map((m) => m[1] ?? "").sort();
}

describe("locale files", () => {
  const enKeys = flatten(en);
  const zhKeys = flatten(zhCN);

  it("are two-level objects of strings", () => {
    for (const table of [en, zhCN] as LocaleTable[]) {
      for (const section of Object.values(table)) {
        expect(typeof section).toBe("object");
        for (const value of Object.values(section)) expect(typeof value).toBe("string");
      }
    }
  });

  it("have the same keys in both languages", () => {
    expect([...zhKeys.keys()].filter((k) => !enKeys.has(k))).toEqual([]);
    expect([...enKeys.keys()].filter((k) => !zhKeys.has(k))).toEqual([]);
  });

  it("have no empty strings", () => {
    for (const [key, value] of [...enKeys, ...zhKeys]) expect(value.trim(), key).not.toBe("");
  });

  it("contain every message code of SPEC §8 under msg.", () => {
    for (const code of SPEC_CODES) {
      expect(enKeys.has(`msg.${code}`), `en msg.${code}`).toBe(true);
      expect(zhKeys.has(`msg.${code}`), `zh-CN msg.${code}`).toBe(true);
    }
  });

  it("use the spec's parameters in message templates", () => {
    for (const [code, params] of Object.entries(SPEC_PARAMS)) {
      for (const table of [enKeys, zhKeys]) {
        expect(placeholders(table.get(`msg.${code}`) ?? ""), code).toEqual([...params].sort());
      }
    }
  });

  it("use the same placeholders in both languages", () => {
    // Durations pad seconds in English ("1 min 05 s") but not in Chinese ("1 分 5 秒").
    const exempt = new Set(["duration.minutes", "duration.hours"]);
    for (const [key, value] of enKeys) {
      if (exempt.has(key)) continue;
      expect(placeholders(zhKeys.get(key) ?? ""), key).toEqual(placeholders(value));
    }
  });

  it("follow the Chinese typography rules", () => {
    const cjk = "[\\u3400-\\u9fff]";
    const latin = "[A-Za-z0-9{}<^]";
    for (const [key, value] of zhKeys) {
      expect(value, `${key}: space between CJK and Latin`).not.toMatch(new RegExp(`${cjk}${latin}|${latin.replace("<", "")}${cjk}`));
      expect(value, `${key}: full-width punctuation`).not.toMatch(new RegExp(`${cjk}[,;:!?]|${cjk}\\.(?!\\.)`));
    }
  });

  it("keep \"Package\" in English", () => {
    for (const [key, value] of zhKeys) expect(value, key).not.toMatch(/包/);
  });
});

const MSG_RS = fileURLToPath(new URL("../../../crates/ubot-core/src/msg.rs", import.meta.url));

/** The English templates of the core (`EN` in crates/ubot-core/src/msg.rs), in table order. */
function coreTemplates(): { code: string; template: string }[] {
  const source = readFileSync(MSG_RS, "utf8");
  const start = source.indexOf("pub const EN:");
  const end = source.indexOf("\n];", start);
  if (start < 0 || end < 0) throw new Error(`The EN table was not found in ${MSG_RS}`);
  const table = source.slice(start, end);
  const entries = [...table.matchAll(/\(\s*"([\w.]+)"\s*,\s*"((?:[^"\\]|\\.)*)"\s*,?\s*\)/g)]
    .map((m) => ({ code: m[1] ?? "", template: (m[2] ?? "").replace(/\\(.)/g, "$1") }));
  // Every entry starts with a parenthesis at the table's indentation; one the pattern missed fails here.
  expect(entries.length, "entries of the EN table the pattern recognised").toBe((table.match(/^ {4}\(/gm) ?? []).length);
  return entries;
}

describe("locale files and the core", () => {
  const core = coreTemplates();

  it("read the whole table of the core", () => {
    expect(core.length).toBeGreaterThan(80);
    expect(new Set(core.map((e) => e.code)).size).toBe(core.length);
    expect(core.map((e) => e.code)).toContain("plan.projectReadOnly");
  });

  it("have every code the core can emit, in both languages with the core's placeholders", () => {
    for (const { code, template } of core) {
      for (const [name, table] of [["en", en], ["zh-CN", zhCN]] as [string, LocaleTable][]) {
        const text = table["msg"]?.[code];
        expect(text, `${name} msg.${code}`).toBeDefined();
        expect(placeholders(text ?? ""), `${name} msg.${code} placeholders`).toEqual(placeholders(template));
      }
    }
  });

  it("show the English text of the core in the English locale, which rejections are recognised by", () => {
    for (const { code, template } of core) expect((en as LocaleTable)["msg"]?.[code], `msg.${code}`).toBe(template);
  });

  it("recognise every plan and task rejection of the core from its English text", () => {
    for (const { code, template } of core) {
      if (!code.startsWith("plan.") && !code.startsWith("task.")) continue;
      const params = Object.fromEntries(placeholders(template).map((name) => [name, `<${name}>`]));
      const rendered = format(template, params);
      const msg = msgFromEnglish(rendered);
      expect(msg?.code, rendered).toBe(code);
      expect(msg?.params, rendered).toEqual(params);
    }
  });
});

describe("translation", () => {
  const fake: Record<Lang, LocaleTable> = {
    en: { a: { hello: "Hello {name}", only: "English only" } },
    "zh-CN": { a: { hello: "你好，{name}" } }
  };

  it("formats placeholders and keeps unknown ones", () => {
    expect(format("{a} and {b}", { a: "x" })).toBe("x and {b}");
    expect(format("{n} steps", { n: 4 })).toBe("4 steps");
  });

  it("falls back from Chinese to English to the key", () => {
    expect(translateIn(fake, "zh-CN", "a.hello", { name: "uBot" })).toBe("你好，uBot");
    expect(translateIn(fake, "zh-CN", "a.only")).toBe("English only");
    expect(translateIn(fake, "zh-CN", "a.missing")).toBe("a.missing");
    expect(translateIn(fake, "en", "nosection")).toBe("nosection");
  });

  it("looks up keys whose rest contains dots", () => {
    expect(translate("en", "msg.step.clone.detail", { repo: "r", dest: "d" })).toBe("r → d");
    expect(translate("zh-CN", "msg.step.removePackage.detail", { dest: "Plugins/uBot/uBotMCP" })).toBe("Plugins/uBot/uBotMCP → 回收站");
  });

  it("renders a Msg in both languages", () => {
    const msg = { code: "problem.requiresVersion", params: { package: "UBotEnvironment", dep: "UBotCore", range: "^0.2.0", installed: "0.1.0" } };
    expect(renderMsg("en", msg)).toBe("Requires UBotCore ^0.2.0, but 0.1.0 is installed.");
    expect(renderMsg("zh-CN", msg)).toBe("需要 UBotCore ^0.2.0，当前安装的是 0.1.0。");
    expect(renderMsg("en", { code: "task.apply", params: { project: "MyRobotSim" } })).toBe("Apply Changes · MyRobotSim");
    expect(renderMsg("zh-CN", { code: "step.detail.requiredBy", params: { package: "UBotVehicles" } })).toBe("UBotVehicles 需要");
  });

  it("renders the prebuilt binaries messages in both languages", () => {
    const download = { code: "step.download", params: { package: "UBotVehicles", version: "0.1.0" } };
    expect(renderMsg("en", download)).toBe("Download UBotVehicles 0.1.0");
    expect(renderMsg("zh-CN", download)).toBe("下载 UBotVehicles 0.1.0");
    const unavailable = { code: "plan.prebuiltUnavailable", params: { package: "UBotMCP", version: "0.1.0", engine: "5.5" } };
    expect(renderMsg("en", unavailable)).toBe("No prebuilt UBotMCP 0.1.0 for UE 5.5; it will be built from source.");
    expect(renderMsg("zh-CN", unavailable)).toBe("没有适用于 UE 5.5 的 UBotMCP 0.1.0 预编译版本，将从源码编译。");
    const mismatch = { code: "problem.prebuiltEngineMismatch", params: { package: "UBotCombat", built: "5.4", engine: "5.5" } };
    expect(renderMsg("en", mismatch)).toBe("UBotCombat was installed prebuilt for UE 5.4, but this project uses UE 5.5.");
    expect(renderMsg("zh-CN", mismatch)).toBe("UBotCombat 安装的是 UE 5.4 的预编译版本，但此项目使用 UE 5.5。");
    expect(renderMsg("zh-CN", { code: "fix.reinstall", params: { package: "UBotCombat" } })).toBe("重新安装 UBotCombat");
    expect(renderMsg("zh-CN", { code: "plan.noCompiler", params: {} })).toBe("需要编译 C++，但没有找到 Visual Studio 2022。");
  });

  it("renders verbatim text as it is", () => {
    const msg = { code: "text", params: { text: "Could not connect to ubot_ros_bridge at 127.0.0.1:8268." } };
    expect(renderMsg("en", msg)).toBe("Could not connect to ubot_ros_bridge at 127.0.0.1:8268.");
    expect(renderMsg("zh-CN", msg)).toBe("Could not connect to ubot_ros_bridge at 127.0.0.1:8268.");
  });

  it("falls back to the raw message for unknown codes", () => {
    expect(renderMsg("en", { code: "problem.somethingNew", params: { message: "Raw text." } })).toBe("Raw text.");
    expect(renderMsg("zh-CN", { code: "problem.somethingNew", params: {} })).toBe("problem.somethingNew");
  });

  it("picks singular and plural forms", () => {
    setLanguage("en");
    expect(tn("plan.steps", 1)).toBe("1 step");
    expect(tn("plan.steps", 4)).toBe("4 steps");
    setLanguage("zh-CN");
    expect(tn("plan.steps", 4)).toBe("4 个步骤");
    expect(tn("recipes.count", 1)).toBe("1 个 Package");
    setLanguage("en");
  });

  it("has singular and plural forms for the counts in tooltips", () => {
    setLanguage("en");
    expect(tn("status.updates", 1)).toBe("1 update available");
    expect(tn("status.updates", 2)).toBe("2 updates available");
    expect(tn("status.warnings", 1)).toBe("1 problem");
    expect(tn("status.errors", 1)).toBe("1 error");
    expect(tn("status.errors", 3)).toBe("3 errors");
    expect(tn("header.indexOk", 1, { name: "uBotCore" })).toBe("uBotCore: 1 package");
    expect(tn("header.indexCached", 6, { name: "uBotCore" })).toBe("uBotCore: 6 packages (cached)");
    setLanguage("zh-CN");
    expect(tn("status.updates", 1)).toBe("1 个更新");
    expect(tn("header.indexCached", 6, { name: "uBotCore" })).toBe("uBotCore：6 个 Package（缓存）");
    setLanguage("en");
  });

  it("translates the index problem group", () => {
    expect(translate("en", "problems.groupIndex")).toBe("Package Index");
    expect(translate("zh-CN", "problems.groupIndex")).toBe("Package 索引");
  });
});

describe("rejections of the core", () => {
  it("are recognised from their English text and shown in the UI language", () => {
    expect(msgFromEnglish("Another task is running.")).toEqual({ code: "task.busy", params: {} });
    expect(msgFromEnglish("No project is open.")).toEqual({ code: "plan.noProject", params: {} });
    expect(msgFromEnglish("UBotSensor is not installed.")).toEqual({ code: "plan.notInstalled", params: { package: "UBotSensor" } });
    expect(msgFromEnglish("UBotA requires UBotCore ^0.2.0.")).toEqual({
      code: "plan.breaksDependent", params: { dependent: "UBotA", package: "UBotCore", range: "^0.2.0" }
    });
    expect(msgFromEnglish("fatal: unable to access 'https://x/y.git'")).toBeNull();
    expect(msgFromEnglish("Cannot install UBotCore")).toBeNull();
  });

  it("only match the whole text", () => {
    expect(msgFromEnglish("Error: Another task is running.")).toBeNull();
    expect(msgFromEnglish("Another task is running. Try again.")).toBeNull();
  });
});
