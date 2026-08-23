#!/usr/bin/env bash
#
# One directive, a crew of eight, and a real model deciding what to do with it.
#
# The evals ask whether a crew communicates sensibly and assert the answer. This
# asks what a whole team actually does, and a real model does something slightly
# different every time, so the answer is a recording rather than an assertion:
# every run writes its events, its messages, its transcript and its numbers to
# runs/<timestamp>/, and a run more than once writes down what was different.
#
#   ./scripts/crew.sh                             one run
#   GUACA_RUNS=5 ./scripts/crew.sh                five, to see what varies
#   GUACA_MODEL=anthropic/claude-sonnet-5 ./scripts/crew.sh
#
# Everything it reads from the environment, and where the defaults live:
#
#   GUACA_RUNS         how many times to send the same directive   (crew.rs)
#   GUACA_MODEL        what every agent runs on                    (crew.rs)
#   GUACA_STEPS        model calls one run may spend               (crew.rs)
#   GUACA_SETTLE_SECS  how long a run has to go quiet              (crew.rs)
#   GUACA_DIRECTIVE    what the Chief of Staff is asked            (crew.rs)
#
# The defaults are in the test rather than here so there is one of each; the run
# prints what it resolved them to before it spends anything.
#
# The endpoint, the key and the guard's limits come from the app's own settings,
# so what this measures is the workspace the operator is actually running. The
# one exception is the step budget, which is capped here: a crew of eight that
# will not converge is a bill rather than a finding.

set -euo pipefail

cd "$(dirname "$0")/.."

CONFIG="$HOME/Library/Application Support/com.madebywelch.guac/config.json"

if [ ! -f "$CONFIG" ]; then
  echo "no settings at $CONFIG; open Guaca and set an endpoint and key first" >&2
  exit 1
fi

printf '\033[1m==> A live crew, against your own key\033[0m\n'
echo "These make real model calls and cost real money. Ctrl-C now if that is a surprise."
echo

# --nocapture so the runs print as they happen: a five-run comparison takes
# minutes and a person watching it wants the first run's numbers before the
# fifth one starts. One thread, because two crews thinking at once against the
# same endpoint makes the timings meaningless and interleaves the output.
cargo test --manifest-path src-tauri/Cargo.toml --test crew \
  -- --ignored --nocapture --test-threads=1
