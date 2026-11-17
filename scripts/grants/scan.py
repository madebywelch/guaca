import ast
import io
import json
import sys
import tokenize

SKIP_DECORATORS = {"skip", "skipIf", "skipUnless", "expectedFailure", "skipif", "xfail"}
SKIP_CALLS = {("skipTest",), ("pytest", "skip"), ("pytest", "xfail")}
LOOPS = (ast.For, ast.AsyncFor, ast.While)
CLOCKS = {("monotonic",), ("time",), ("perf_counter",), ("monotonic_ns",), ("now",)}
SCOPES = (ast.FunctionDef, ast.AsyncFunctionDef, ast.Lambda, ast.ClassDef)


def dotted(node):
    parts = []
    while isinstance(node, ast.Attribute):
        parts.append(node.attr)
        node = node.value
    if isinstance(node, ast.Name):
        parts.append(node.id)
    return tuple(reversed(parts))


def bare_string(node):
    if not isinstance(node, ast.Expr):
        return False
    value = node.value
    return isinstance(value, ast.JoinedStr) or (isinstance(value, ast.Constant) and isinstance(value.value, str))


def guarded_branches(node):
    if isinstance(node, ast.If) and not isinstance(node.test, ast.Constant):
        return node.body + node.orelse
    if isinstance(node, ast.Match):
        return node.cases
    if isinstance(node, ast.Try):
        return node.handlers + node.orelse
    return []


def exits(node, guarded):
    if isinstance(node, (ast.Break, ast.Return)):
        return guarded
    if isinstance(node, SCOPES + LOOPS):
        return False
    branches = guarded_branches(node)
    return any(exits(child, guarded or child in branches) for child in ast.iter_child_nodes(node))


def reads_clock(node):
    if isinstance(node, ast.Call) and dotted(node.func)[-1:] in CLOCKS:
        return True
    if isinstance(node, SCOPES):
        return False
    return any(reads_clock(child) for child in ast.iter_child_nodes(node))


def bounded_poll(loop):
    counted = isinstance(loop, (ast.For, ast.AsyncFor))
    conditional = isinstance(loop, ast.While) and not isinstance(loop.test, ast.Constant)
    rechecks = conditional or any(exits(statement, False) for statement in loop.body)
    return (counted or reads_clock(loop)) and rechecks


def scan(text, wait):
    found = {"comments": [], "strings": [], "marks": []}
    for token in tokenize.generate_tokens(io.StringIO(text).readline):
        if token.type == tokenize.COMMENT:
            found["comments"].append([*token.start, *token.end])

    def visit(node, in_loop):
        if bare_string(node):
            found["strings"].append([node.lineno, node.end_lineno])
        for decorator in getattr(node, "decorator_list", []):
            names = dotted(decorator.func if isinstance(decorator, ast.Call) else decorator)
            if names and names[-1] in SKIP_DECORATORS:
                found["marks"].append([decorator.lineno, "@" + ".".join(names)])
        if isinstance(node, ast.Call):
            names = dotted(node.func)
            if names[-1:] in SKIP_CALLS or names[-2:] in SKIP_CALLS:
                found["marks"].append([node.lineno, ".".join(names)])
            elif wait and names[-1:] == ("sleep",) and not (in_loop and bounded_poll(in_loop)):
                found["marks"].append([node.lineno, "sleep"])
        for child in ast.iter_child_nodes(node):
            visit(child, node if isinstance(node, LOOPS) else None if isinstance(node, SCOPES) else in_loop)

    visit(ast.parse(text), None)
    return found


def main():
    answers = []
    for entry in json.load(sys.stdin):
        try:
            answers.append(scan(entry["text"], entry["wait"]))
        except (SyntaxError, tokenize.TokenError, ValueError) as error:
            answers.append({"error": str(error)})
    json.dump(answers, sys.stdout)


main()
