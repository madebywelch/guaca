# First run and a reply

A person opens a new workspace, picks a first task, connects a model, starts
the task and reads the agent's answer; then writes again in the field and is
answered in the same conversation.

## Sub-features

- `onboard-task` picks a first task and writes the brief.
- `onboard-connect` tests the endpoint and key before saving them.
- `onboard-reject` reports a key the endpoint rejects and saves nothing.
- `reply` shows the agent's answer in the conversation.
- `field` sends a later message from the field at the foot of the window.

## How to get to it (user POV)

- Open the invitation link on a workspace with no agents.
- Later: the field at the foot of an agent's page.

## Driving it with scripts/verify.py

Preconditions: an empty workspace (every scenario starts with one).

- **Task.** Press button `Something else Start with a task in your own words.`,
  fill textbox `Your first brief`, press `Continue`.
- **Connect.** Press `API key Use OpenRouter or OpenAI, with usage billed to that account.`,
  choose `Custom endpoint` in combobox `API provider`, fill `Inference endpoint`,
  `Default model`, then the textbox whose name starts with `API key`. Press
  `Test and continue`. A wrong key leaves an `alert` naming HTTP 401, and
  `get_settings` still has the old endpoint and `apiKeySet: false`.
- **Start.** Press `Start task`. The log `Conversation with Executive Assistant`
  shows the brief and the reply; `messages` holds one row each way and
  `outcomes` one `done`.
- **Field.** `write(w, "Executive Assistant", text)`: combobox `Recipient`,
  combobox `Write to Executive Assistant…`, button `Send`.
- **Proof.** `said(w, AGENT)` lists both replies in order, and the page received
  `messageAppended`, `streamStarted`, `streamEnded` and `runSettled`.

## Gotchas

- The key field is cleared when the endpoint changes. Fill the endpoint first.
- `Default model` arrives prefilled; use `fill`, which selects all before
  typing, or the model name is appended to the default.
- The connection test sends one call whose only message is `ping`. Script it.
- `provider` is `compatible` before anything is saved; prove a save by
  `baseUrl` and `apiKeySet`, not by the provider.
