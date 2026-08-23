---
name: run-guaca
description: Launch Guaca locally and get a change on screen, including the scratch-workspace path that needs no API key and costs nothing. Use when asked to run, start, launch or screenshot the app, to check a change in the real app rather than in tests, or when the app will not start.
---

# Running Guaca

Two ways in. Pick by what you are actually trying to see.

| You want to see | Use |
|---|---|
| A transcript row, a card, a dialog, anything the webview draws | **The harness.** Seconds, no Rust build, no key, no spend. |
| IPC, the runtime, migrations, the file store, a real agent turn | **The app.** |

Reach for the app when the harness cannot answer the question. Most chat work
is the harness.

## The harness

`src/preview.tsx` is not committed. Write one, point Vite at it, screenshot it
headless, then delete it. Nothing about it is precious.

```sh
cat > preview.html <<'EOF'
<!doctype html>
<html lang="en"><head><meta charset="UTF-8" /></head>
<body><div id="root"></div><script type="module" src="/src/preview.tsx"></script></body></html>
EOF
npx vite --port 5199 --strictPort &
```

Two things the harness needs, and both bite silently:

- **`convertFileSrc` reads the Tauri bridge off `window`.** Anything drawing a
  file throws `Cannot read properties of undefined` without a stub. Set
  `globalThis.__TAURI_INTERNALS__ = { convertFileSrc: (p, s) => `${s}://localhost/${p}` }`
  before the first import that reaches `lib/files.ts`.
- **`lib/ipc.ts` reaches for Tauri too.** Render leaf components rather than
  `App`, or stub `api`.

Screenshot it, and then **look at the image**. A render that throws still
produces a PNG.

```sh
"/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" \
  --headless --disable-gpu --hide-scrollbars --user-data-dir=/tmp/shot \
  --window-size=900,700 --screenshot=/tmp/out.png --virtual-time-budget=5000 \
  http://localhost:5199/preview.html
```

Use a fresh `--user-data-dir` per shot; a reused one hangs on the second run.
Chrome takes tens of seconds to exit, so run it in the background and wait on
the file rather than on the process.

## The app

```sh
pnpm app                      # tauri dev
GUAC_LOG=guac=debug pnpm app  # what you almost always want
```

The default filter is `guac=info,warn`, which hides `served a file`, the proxy,
and every other line you would launch the app to read.

Wait for the readiness line, not for the process: the binary exists long before
the window does.

```sh
(GUAC_LOG=guac=debug pnpm app > /tmp/guac/app.log 2>&1 &)
until grep -q "guac ready" /tmp/guac/app.log; do sleep 2; done
```

Write the log somewhere you own. A stale `/tmp/*.log` from another account
fails the redirect with `permission denied` and the launch never happens, while
the old file sits there looking like output.

First build is ~440 crates. After that a relaunch is seconds.

## It will not start

**`database is at version N, newer than this build supports (M)`** is the
common one, and it is the migration guard working. Your branch is behind a
branch that added a migration, and the app refuses a schema it does not know
rather than writing to it.

```sh
git fetch origin && git rebase origin/main
```

Do not delete the database and do not lower `user_version`. Either one throws
away a real workspace to avoid a rebase.

## The operator's data

macOS: `~/Library/Application Support/com.madebywelch.guac/`

`guac.db`, `config.json` (**holds the API key in plaintext — never print this
file, and never print a field of it without redacting nested objects**), and
`files/` addressed by digest.

There is one profile per bundle identifier, so every workspace on this machine
shares it. Treat it as production:

- Back it up before writing to it, and put it back afterward.
- For anything experimental, rename it aside and let the app make a fresh one.
  A running app follows its open file descriptors, so you can rename the
  scratch profile out and the real one back in without stopping it.
- Never seed test rows into a database the operator is using.

## A workspace to test against

`seed.py` fills a scratch profile with a crew and a transcript covering every
row a channel can draw: bubbles, a peer burst, a folded tool trail with a
failure and a credential spend, a routine firing, and files. No key, no
network, no spend.

```sh
python3 .claude/skills/run-guaca/seed.py --help
```

## What you cannot do from here

`screencapture` needs Screen Recording permission and `osascript` needs
Accessibility, and neither is granted to the terminal. There is no way to
screenshot or drive the real window from a shell. Use the harness for anything
visual, and ask the operator to look when only the real app will do.

## Before you say it works

```sh
./scripts/ci.sh
```
