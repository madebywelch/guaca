import { execFileSync } from "node:child_process";
import { describe, expect, it } from "vitest";

import { stroke } from "../../src/avatars/drawn";
import type { Point } from "../../src/avatars/form";

const LIFT = `
import ast, json, math, sys
tree = ast.parse(open("design/wordmarks/wordmarks.py").read())
wanted = [node for node in tree.body if isinstance(node, ast.FunctionDef) and node.name in ("num", "stroke")]
if len(wanted) != 2:
    raise SystemExit("design/wordmarks/wordmarks.py no longer defines both num and stroke")
scope = {"math": math}
exec(compile(ast.Module(body=wanted, type_ignores=[]), "wordmarks.py", "exec"), scope)
print(json.dumps([scope["stroke"](line["center"], line["widths"]) for line in json.load(sys.stdin)]))
`;

const LINES: { center: Point[]; widths: number[] }[] = [
  { center: [[0, 0], [10, 0]], widths: [4, 6] },
  { center: [[6, -1], [9, 8], [9, 58], [15, 86], [34, 100]], widths: [3, 12, 17, 9, 2] },
  { center: [[80, 24], [62, 6], [38, 2], [17, 16], [8, 44], [13, 74]], widths: [5, 6, 11, 17, 17, 14] },
  { center: [[0, 0], [0, 0], [3, 4]], widths: [2, 2, 2] },
];

function points(path: string): number[] {
  return [...path.matchAll(/-?\d+(?:\.\d+)?/g)].map((m) => Number(m[0]));
}

describe("design/wordmarks/wordmarks.py", () => {
  it("letters with the brush drawn.ts paints the cast with", () => {
    const lettered: string[] = JSON.parse(
      execFileSync("python3", ["-I", "-c", LIFT], { input: JSON.stringify(LINES), encoding: "utf8" }),
    );
    LINES.forEach((line, i) => {
      const cast = points(stroke(line.center, line.widths));
      const word = points(lettered[i] ?? "");
      expect(word).toHaveLength(cast.length);
      word.forEach((v, k) => {
        expect(Math.abs(v - (cast[k] ?? Number.NaN)), `line ${i}, number ${k}`).toBeLessThanOrEqual(0.06);
      });
    });
  });
});
