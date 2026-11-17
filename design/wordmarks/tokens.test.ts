import { execFileSync, spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const LIFT = `
import ast, json, re, sys
tree = ast.parse(open("design/wordmarks/wordmarks.py").read())
wanted = [node for node in tree.body
          if (isinstance(node, ast.FunctionDef) and node.name == "surface")
          or (isinstance(node, ast.Assign) and any(getattr(t, "id", None) == "SURFACE" for t in node.targets))]
if len(wanted) != 2:
    raise SystemExit("design/wordmarks/wordmarks.py no longer defines both SURFACE and surface")
scope = {"re": re}
exec(compile(ast.Module(body=wanted, type_ignores=[]), "wordmarks.py", "exec"), scope)
ask = json.load(sys.stdin)
print(json.dumps(scope["surface"](ask["css"], ask["selector"])))
`;

const STYLES = readFileSync("src/styles.css", "utf8");
const LIGHT = ":root";
const DARK = ':root[data-surface="dark"]';

function lifted(css: string, selector: string) {
  return spawnSync("python3", ["-I", "-c", LIFT], {
    input: JSON.stringify({ css, selector }),
    encoding: "utf8",
  });
}

function declared(selector: string, name: string): string {
  const start = STYLES.indexOf(`${selector} {\n`);
  const block = STYLES.slice(start, STYLES.indexOf("\n}", start));
  const found = new RegExp(`^\\s*--${name}: (#[0-9a-fA-F]{6});$`, "m").exec(block);
  if (start < 0 || !found?.[1]) throw new Error(`src/styles.css has no --${name} in ${selector}`);
  return found[1];
}

describe("design/wordmarks/wordmarks.py colors", () => {
  it("fails naming the token a surface block lacks", () => {
    const run = lifted(STYLES.replace(/^ {2}--attention: #[0-9a-fA-F]{6};\n/m, ""), LIGHT);
    expect(run.status).not.toBe(0);
    expect(run.stderr).toContain("--attention in its :root block");
  });

  it("fails when the dark block is gone", () => {
    const run = lifted(STYLES.replace(DARK, ":root.gone"), DARK);
    expect(run.status).not.toBe(0);
    expect(run.stderr).toContain(`no ${DARK} block`);
  });

  it.each([
    ["paper", LIGHT],
    ["dark", DARK],
  ])("draws %s in the rail and attention tokens", (_, selector) => {
    const colors = JSON.parse(
      execFileSync("python3", ["-I", "-c", LIFT], {
        input: JSON.stringify({ css: STYLES, selector }),
        encoding: "utf8",
      }),
    );
    expect(colors).toEqual({
      ground: declared(selector, "rail-ground"),
      edge: declared(selector, "rail-edge"),
      raised: declared(selector, "rail-raised"),
      ink: declared(selector, "rail-text"),
      muted: declared(selector, "rail-muted"),
      amber: declared(selector, "attention"),
    });
  });
});
