---
name: systematic-debugging
description: "Use for any bug, test failure, failed build, error, performance problem or unexpected behavior, before proposing a fix, and especially under time pressure or after a fix has already failed. Root cause first: reproduce, isolate, test hypotheses, then fix with a regression test."
license: MIT
author: "Hermes Agent (adapted from obra/superpowers), adapted for Guaca"
---

# Systematic debugging

Random fixes waste time and create new bugs. Quick patches hide the underlying
problem.

**Core principle:** find the root cause before attempting any fix. A fix for a
symptom is a failure, and following the letter of this process while dodging
its intent is not following it.

```
NO FIXES WITHOUT ROOT CAUSE INVESTIGATION FIRST
```

If you have not finished Phase 1, you cannot propose a fix.

## The feedback loop

The feedback loop is the debugging. Before reading code to build a theory,
create or find a tight command that goes red on the exact symptom reported and
green once the bug is fixed. A tight loop is fast, deterministic, something you
can run yourself, and specific enough to catch this bug rather than merely
"does not crash".

When a clean reproduction is hard, spend disproportionate effort on building
the loop. Guessing without a loop that can go red is the failure this skill
exists to prevent.

## Where you do the work

- **With a repository:** `shell` runs one command there and hands back its
  output: `git log`, `git diff`, `git grep`, one failing test. It is stopped
  after two minutes, so a full test suite, a build or any edit goes to `code`.
  A `code` job cannot see this conversation and answers later, so its brief
  carries the symptom, the loop command, and whether to report the root cause
  without fixing it or to fix it with a regression test.
- **With a computer:** `run_command` runs on your own Linux machine, which is
  not where a repository is.
- **With neither:** you can still debug by directing. Get the exact error, the
  logs and the steps from the operator or from a crewmate who has the system
  (`read_file` for anything they attach), and hold to the same phases: no fix is
  proposed before the root cause is known.

## When to use it

Any technical issue: test failures, production bugs, unexpected behavior,
performance problems, build failures, integration issues.

Especially when:
- There is time pressure, because emergencies make guessing tempting.
- "One quick fix" seems obvious.
- You have already tried a fix, or several, and they did not work.
- You do not fully understand the issue.

Do not skip it because the issue seems simple (simple bugs have root causes
too), because you are in a hurry (rushing guarantees rework), or because
someone wants it fixed now (systematic is faster than thrashing).

## The four phases

Finish each phase before starting the next.

### Phase 1: root cause investigation

1. **Read the error carefully.** Do not skip past errors or warnings; they often
   contain the answer. Read stack traces completely, and note line numbers,
   file paths and error codes. Find where the message comes from:
   `git grep -n "the message"`.

2. **Build a tight feedback loop.**
   - Can you trigger the exact symptom with one command?
   - Does it fail for this bug, and pass only once the bug is fixed?
   - Is it fast enough to run over and over? Is it deterministic? For a flaky
     bug, can you raise the reproduction rate high enough to work with?
   - If you cannot reproduce it, gather more data. Do not guess.

   Ways to build one, roughly in this order:
   1. A failing test at the seam that reaches the bug: unit, integration or end
      to end.
   2. An HTTP request (`curl`) against a running server.
   3. A CLI invocation with fixture input, diffing its output against what is
      expected.
   4. A headless browser script asserting on the DOM, the console or the network.
   5. A captured trace replayed: a HAR file, a request payload, an event log, a
      queue message, a webhook body.
   6. A throwaway harness that boots the smallest useful slice of the system and
      calls the failing path.
   7. A property or fuzz loop, for intermittently wrong output over a wide input
      space.
   8. A bisection script for `git bisect run`, when the bug appeared between two
      known states.
   9. A differential loop comparing old and new versions, two configurations,
      two providers or two datasets.
   10. A scripted human step, only as a last resort, capturing its result so the
       loop stays structured.

   Then tighten it. Faster: cache setup, narrow the scope, skip unrelated
   initialization. Sharper: assert the exact symptom, not generic success. More
   deterministic: pin the time, seed randomness, isolate the filesystem, freeze
   the network.

   For a nondeterministic bug the first goal is a higher reproduction rate, not
   perfection: run it a hundred times, in parallel, under stress, with narrowed
   timing windows or injected sleeps. A 50% flake can be debugged; a 1% flake
   usually cannot.

   ```bash
   pytest tests/test_module.py::test_name -v
   for i in {1..100}; do pytest tests/test_flake.py::test_name -q || break; done
   ```

3. **Check recent changes.** What changed that could cause this: commits,
   uncommitted changes, new dependencies, configuration?

   ```bash
   git log --oneline -10
   git diff
   git log -p --follow src/problem_file.py | head -100
   ```

4. **Gather evidence across components.** When the system has several (API,
   service, database; CI, build, deploy), add instrumentation before proposing
   anything. At each boundary, log what enters and what leaves, and check that
   environment and configuration propagate. Run once to see where it breaks,
   then investigate that component.

5. **Trace the data flow.** When the error is deep in a call stack, ask where
   the bad value originated and what called this with it. Keep tracing upstream
   to the source, and fix it there, not where it surfaced.

   ```bash
   git grep -n "function_name("
   git grep -nE "variable_name\s*="
   ```

Phase 1 is done when:
- The error messages are read and understood.
- A tight loop exists and has been run at least once.
- The loop asserts the exact symptom reported, not a nearby failure.
- The loop is deterministic, or a flaky bug reproduces often enough to work
  with.
- Recent changes have been reviewed.
- Evidence is gathered: logs, state, data flow.
- The problem is isolated to a specific component or piece of code.
- Root cause hypotheses can be stated and tested.

Do not go on to Phase 2 until you understand why it is happening.

### Phase 2: pattern analysis

1. **Minimize the reproduction.** Once the loop is red, shrink it to the
   smallest scenario that still fails. Cut inputs, callers, configuration, data
   and steps one at a time, running the loop after each cut, and keep only what
   the failure depends on. It is minimal when removing anything left makes it
   pass. A minimal reproduction narrows the hypotheses and often becomes the
   cleanest regression test.
2. **Find working examples.** Locate similar code in the same codebase that
   works.
3. **Compare against references.** If you are implementing a pattern, read the
   reference implementation completely, every line, before applying it.
4. **Identify differences.** List every difference between working and broken,
   however small. Do not assume any of them cannot matter.
5. **Understand dependencies.** What other components, settings, configuration
   and environment does it need? What does it assume?

### Phase 3: hypotheses and testing

1. **Form ranked, falsifiable hypotheses.** Generate three to five plausible
   ones before testing any. Rank them by likelihood and by how cheap they are to
   falsify. State what each predicts: "if X is the cause, then changing or
   observing Y should make Z happen." Sharpen or discard any that predicts
   nothing testable. If the operator is working with you right now, show them
   the ranked list before testing, because they may know something that
   reorders it. If they are not, proceed with your ranking and report it with
   the result.
2. **Test minimally.** Test the top hypothesis with the smallest possible probe,
   changing one variable at a time. Do not fix several things at once. A
   debugger or a REPL beats ten log lines when you have one. Tag every temporary
   log line with a unique prefix such as `[DEBUG-a4f2]` so removing them is one
   search.
3. **Verify before continuing.** If it is confirmed, go to Phase 4. If not, form
   a new hypothesis. Do not stack more fixes on top.
4. **When you do not know,** say "I don't understand X". Do not pretend. Ask the
   operator or a crewmate who knows the system, or research more.

### Phase 4: implementation

1. **Write the failing test first.** The simplest reproduction, automated if at
   all possible. It must exist before the fix.
2. **Make one fix.** Address the root cause. One change at a time, no "while
   I'm here" improvements, no bundled refactoring.
3. **Verify the fix.** Run the regression test, then the whole suite for
   regressions (through `code` if the suite outlasts `shell`).
4. **If the fix does not work, stop and count.** Fewer than three attempts: go
   back to Phase 1 with what you learned. Three or more: stop and question the
   architecture. Do not attempt a fourth fix without that discussion.
5. **After three failed fixes, question the architecture.** The signs: each fix
   reveals new shared state or coupling somewhere else, fixes need massive
   refactoring, each fix creates new symptoms elsewhere. Ask whether the
   pattern is fundamentally sound or kept through sheer inertia, and whether to
   refactor rather than keep fixing symptoms. Take it to the operator before
   trying again: in your reply if they are in the conversation, otherwise as a
   `decision` with your recommendation. This is not a failed hypothesis; it is
   a wrong architecture.

## Red flags

If you catch yourself thinking any of these, stop and return to Phase 1:
- "Quick fix for now, investigate later."
- "Just try changing X and see if it works."
- "Make several changes, then run the tests."
- "Skip the test, I'll check it by hand."
- "It's probably X, let me fix that."
- "I don't fully understand this, but it might work."
- "The pattern says X, but I'll adapt it differently."
- Listing fixes before investigating, or proposing solutions before tracing the
  data flow.
- "One more fix attempt," after two or more have failed.
- Each fix reveals a new problem in a different place.

## Common rationalizations

| Excuse | Reality |
|---|---|
| "The issue is simple, it doesn't need a process." | Simple issues have root causes too, and the process is fast for them. |
| "It's an emergency, there's no time." | Systematic debugging is faster than guess-and-check. |
| "Just try this first, then investigate." | The first fix sets the pattern. Do it right from the start. |
| "I'll write the test after the fix works." | Untested fixes do not stick. A test first proves the fix. |
| "Several fixes at once saves time." | Then you cannot tell what worked, and you cause new bugs. |
| "The reference is too long; I'll adapt the pattern." | Partial understanding guarantees bugs. Read all of it. |
| "I see the problem, let me fix it." | Seeing a symptom is not understanding its root cause. |
| "One more fix attempt." (after two or more failures) | Three failures mean an architectural problem. Question the pattern. |

## Quick reference

| Phase | Activities | Done when |
|---|---|---|
| 1. Root cause | Read errors, reproduce, check changes, gather evidence, trace data | You understand what and why |
| 2. Pattern | Minimize, find working examples, compare, list differences | You know what is different |
| 3. Hypothesis | Rank theories, test minimally, one variable at a time | One is confirmed, or a new one formed |
| 4. Implementation | Regression test, one root-cause fix, verify | The bug is gone and every test passes |
