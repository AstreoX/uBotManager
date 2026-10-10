import { afterEach, describe, expect, it } from "vitest";
import { setLanguage } from "../i18n";
import { errorText } from "./errors";

afterEach(() => setLanguage("en"));

describe("errorText", () => {
  it("shows the core's English rejections in the UI language", () => {
    setLanguage("zh-CN");
    expect(errorText("Another task is running.")).toBe("另一个任务正在运行。");
    expect(errorText("No project is open.")).toBe("尚未打开项目。");
    expect(errorText(new Error("Unreal Editor is running this project."))).toBe("UE 编辑器正在运行此项目。");
    expect(errorText("UBotSensor is not installed.")).toBe("UBotSensor 尚未安装。");
    setLanguage("en");
    expect(errorText("Another task is running.")).toBe("Another task is running.");
  });

  it("translates a bare message code", () => {
    setLanguage("zh-CN");
    expect(errorText("task.busy")).toBe("另一个任务正在运行。");
  });

  it("translates every line of a rejection that holds several messages", () => {
    setLanguage("zh-CN");
    expect(errorText("UBotA is not installed.\nUBotB is not installed.")).toBe("UBotA 尚未安装。\nUBotB 尚未安装。");
    expect(errorText("UBotA is not installed.\r\nD:\\P\\Stuff is outside the project.")).toBe("UBotA 尚未安装。\nD:\\P\\Stuff 不在项目目录内。");
    setLanguage("en");
    expect(errorText("UBotA is not installed.\nUBotB is not installed.")).toBe("UBotA is not installed.\nUBotB is not installed.");
  });

  it("translates each line as a message of its own", () => {
    setLanguage("zh-CN");
    // Matched as a whole, none of these texts is a message of the core: the templates match a single line.
    expect(errorText("Refusing package name A.\nRefusing package name B.")).toBe("拒绝使用 Package 名称 A。\n拒绝使用 Package 名称 B。");
    expect(errorText("C:\\P\\x.uproject is read-only; check it out or clear the read-only flag first.\nUBotA has local changes."))
      .toBe("C:\\P\\x.uproject 为只读；请先签出它或清除只读属性。\nUBotA 有本地修改。");
  });

  it("keeps the lines that are not messages of the core, blank lines included", () => {
    setLanguage("zh-CN");
    expect(errorText("UBotA is not installed.\nfatal: unable to access 'https://x/y.git'\n\nhint: check the URL"))
      .toBe("UBotA 尚未安装。\nfatal: unable to access 'https://x/y.git'\n\nhint: check the URL");
    expect(errorText("fatal: one\nfatal: two")).toBe("fatal: one\nfatal: two");
  });

  it("leaves everything else as it is", () => {
    setLanguage("zh-CN");
    expect(errorText("Internal error: panic")).toBe("Internal error: panic");
    expect(errorText("not.a.code")).toBe("not.a.code");
    expect(errorText(42)).toBe("42");
  });
});
