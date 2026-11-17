# Agents talking to each other

An agent hands a question to another agent, the answer comes back to it, it
tells the person, and the conversation ends. Two agents that would pass work
back and forth forever are stopped by the guard, and the run still settles.

## Sub-features

- `new-agent` adds a second agent from the top bar's plus.
- `relay` delivers a `send_message` and returns the reply to the sender.
- `settle` records one outcome for the whole conversation.
- `hop-limit` refuses a message past the hop limit and ends the volley.

## How to get to it (user POV)

- Button `Make something new`, menuitem `New agent Somebody new in this group`.
- Write to one agent; it decides to ask the other.

## Driving it with scripts/verify.py

Preconditions: `first_run` has finished.

- **Add Critic.** `new_agent(w, "Critic", instructions)` fills `Name` and
  `Instructions` in dialog `New agent` and presses `Create agent`.
- **Relay.** Script the first agent to call `send_message` with
  `to: ["Critic"]` and `intent: "work"`, Critic to answer in words, and the
  first agent to repeat the answer. `between(w)` shows the two messages in one
  run, and the person reads the relayed answer.
- **Volley.** Script both agents to send a numbered `work` message to each other
  on every message they get. A tool result says `… hops from the operator and the
  limit is 8`, and `outcomes` gains one row.

## Gotchas

- A message identical to one already sent in the run is refused as a repeat
  before the hop limit is reached. Number each volley to reach the guard.
- A peer's message reaches the model as `[AGENT "Name"]` then the text.
- A reply in words to a peer is delivered back to it as `courtesy`; script an
  answer for it too, or the run records an unscripted call.
