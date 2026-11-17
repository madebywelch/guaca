# A permission request in the conversation

An agent asks to change a setting. The request parks its turn and appears in
the conversation with its before and after; Allow applies it and Deny does
not. A setting only the person may change is refused without asking.

## Sub-features

- `park` shows the request and changes nothing yet.
- `allow` applies the change and tells the agent.
- `deny` changes nothing and tells the agent.
- `operator-only` refuses `baseUrl` and keys before any request exists.

## How to get to it (user POV)

- Ask an agent to call you something; it asks to change `Your name`.

## Driving it with scripts/verify.py

Preconditions: `first_run` has finished.

- **Ask.** Script the agent to call `settings` with `action: "update"` and
  `changes: {"operatorName": "Robin"}`. Buttons `Allow` and `Deny` appear,
  `approvals` has one `changeSettings` row in state `pending`, and
  `get_settings` still has an empty `operatorName`.
- **Allow.** Press `Allow`. `operatorName` becomes `Robin`; the tool result
  starts `The operator allowed it`.
- **Deny.** Ask again for `Sam`, press `Deny`. `operatorName` stays `Robin`;
  the row's state is `deny`.
- **Operator only.** Ask for `baseUrl`. No new `approvals` row; the tool result
  says only the operator can change it.

## Gotchas

- Approval actions are camelCase in the table (`changeSettings`).
- A settings change has no standing yes; "Always" is recorded as one Allow.
