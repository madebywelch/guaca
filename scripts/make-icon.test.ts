import { execFileSync } from "node:child_process";
import { describe, expect, it } from "vitest";

import { press } from "../src/avatars/form";

const LOAD = `
import importlib.util, json, sys
spec = importlib.util.spec_from_file_location("icon", "scripts/make-icon.py")
icon = importlib.util.module_from_spec(spec)
spec.loader.exec_module(icon)
print(json.dumps([icon.press(*case) for case in json.load(sys.stdin)]))
`;

describe("scripts/make-icon.py", () => {
  it("presses the body exactly as form.ts does, including angles that wrap", () => {
    const cases: [number, number, number, number][] = [];
    for (const a of [-9.5, -Math.PI, -1, 0, 0.5, Math.PI, 4, 12])
      for (const th of [-0.75, 0, 0.3, 0.5, 1.2])
        for (const w of [0.4, 0.7, 0.8]) cases.push([a, th, w, 0.64]);
    const drawn: number[] = JSON.parse(
      execFileSync("python3", ["-I", "-c", LOAD], { input: JSON.stringify(cases), encoding: "utf8" }),
    );
    expect(drawn).toHaveLength(cases.length);
    cases.forEach((c, i) => {
      expect(drawn[i], `press(${c.join(", ")})`).toBeCloseTo(press(...c), 12);
    });
  });
});
