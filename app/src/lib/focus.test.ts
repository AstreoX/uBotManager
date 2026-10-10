import { describe, expect, it } from "vitest";
import { tabTarget } from "./focus";

describe("tabTarget", () => {
  it("wraps at the ends of the dialog", () => {
    expect(tabTarget(4, 3, false)).toBe(0);
    expect(tabTarget(4, 0, true)).toBe(3);
  });

  it("leaves moves inside the dialog to the browser", () => {
    expect(tabTarget(4, 1, false)).toBeNull();
    expect(tabTarget(4, 2, true)).toBeNull();
    expect(tabTarget(1, 0, false)).toBe(0);
  });

  it("brings the focus in from the dialog frame or from outside", () => {
    expect(tabTarget(4, -1, false)).toBe(0);
    expect(tabTarget(4, -1, true)).toBe(3);
  });

  it("does nothing for a dialog without controls", () => {
    expect(tabTarget(0, -1, false)).toBeNull();
  });
});
