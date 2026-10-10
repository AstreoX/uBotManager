import { describe, expect, it } from "vitest";
import { projectSwitch } from "./project";

const A = "E:\\Sims\\A\\A.uproject";
const B = "E:\\Sims\\B\\B.uproject";

describe("projectSwitch", () => {
  it("drops the project's state when another project is taken in", () => {
    expect(projectSwitch(A, B)).toEqual({ switched: true, dropsProjectState: true });
  });

  it("keeps the queued changes when the open project is opened again, however its path is spelled", () => {
    expect(projectSwitch(A, A)).toEqual({ switched: false, dropsProjectState: false });
    expect(projectSwitch(A, "e:/sims/a/a.uproject")).toEqual({ switched: false, dropsProjectState: false });
    expect(projectSwitch("E:/Sims/A/A.uproject", A.toUpperCase())).toEqual({ switched: false, dropsProjectState: false });
  });

  it("takes the first project in without dropping anything", () => {
    expect(projectSwitch(null, A)).toEqual({ switched: true, dropsProjectState: false });
    expect(projectSwitch(null, null)).toEqual({ switched: false, dropsProjectState: false });
  });

  it("drops the project's state when the project goes away", () => {
    expect(projectSwitch(A, null)).toEqual({ switched: true, dropsProjectState: true });
  });
});
