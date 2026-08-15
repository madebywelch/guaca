# What Guaca takes from the interoperability literature, and what it doesn't

Guaca's message layer is derived from the four agent protocols surveyed in
*A survey of agent interoperability protocols: Model Context Protocol (MCP),
Agent Communication Protocol (ACP), Agent-to-Agent Protocol (A2A), and Agent
Network Protocol (ANP)* by Abul Ehtesham, Aditi Singh, Gaurav Kumar Gupta and
Saket Kumar, [arXiv 2505.02279](https://arxiv.org/abs/2505.02279). The
survey is a useful map of the design space and a poor specification: it
describes how agents address each other, never when they stop. This document
records which ideas were adopted, which were deliberately dropped, and which
gaps had to be filled from scratch.

The governing rule: an idea earns its place only if it pays off inside a
single-process, single-user, local desktop app. Most of what these protocols
specify exists to cross an organizational trust boundary. Guaca has no such
boundary, so importing that machinery would be cargo cult.

## Adopted

| Idea | Source | Where it lives | Why it survived |
|---|---|---|---|
| **Agent Card**, a self-describing capability document used for discovery | A2A | `domain::agent::AgentCard` | An agent asked to "introduce yourself to everyone" needs a roster. The card is what makes `directory` answerable. |
| **Directory as a first-class operation** | A2A, ANP | `llm::tools::DIRECTORY` | Hardcoding peers into prompts breaks the moment an agent is added. Discovery at call time is strictly better and costs one tool. |
| **Typed, ordered multipart messages** rather than a bare string | ACP | `domain::envelope::Part` | A message can carry a guard notice, a tool trail, and prose without any of them being parsed back out of the others. |
| **Explicit lifecycle** | all four | `domain::agent::Lifecycle` | Pause and delete need real states. See "Reduced" below for how it was trimmed. |
| **Card versioning** | A2A Update phase | `AgentCard::version` | The only mechanism that lets a peer notice a card changed underneath it. Bumped on every edit. |
| **Lifecycle-phase threat model** | the survey's Tables 3-6 | `guard.rs`, `prompt.rs`, `config.rs` | The survey's most reusable contribution. Its Creation/Operation/Update/Termination framing is a genuinely good checklist. |
| **Prompt injection between agents treated as the primary threat** | MCP "tool poisoning", A2A "task injection" | `domain::envelope::Trust`, `runtime::prompt` | Both names describe one failure: wire content read as principal instruction. Guaca tags provenance on the envelope and restates it in the system prompt. |
| **The card describes capability, so peers can route to it** | A2A | `domain::agent::DirectoryEntry::reaches` | Extended rather than adopted: see "Connectors" below. |

## Reduced

| Idea | What the protocols specify | What Guaca does | Why |
|---|---|---|---|
| Agent Card hosting | Served at `/.well-known/agent-card.json` over HTTP | A row in SQLite | There is no network peer. An HTTP server to talk to yourself is pure overhead. |
| Identity | W3C DIDs, `did:wba`, DID documents, signature verification (ANP) | A UUID | DIDs solve "prove you are who you claim across an untrusted network". Inside one process there is no claim to verify. |
| Manifest signing | Sigstore, JWS, signed manifest diffs | Nothing | Signatures defend against a supply chain Guaca does not have. Adding them would be security theatre with a maintenance cost. |
| Transport | JSON-RPC 2.0 or REST over HTTP, SSE, gRPC | A `tokio::mpsc` channel per agent | The protocols' transport layer exists to cross a process boundary. Guaca's agents share an address space. |
| Registry / broker | Central registry with runtime registration (ACP) | A `HashMap<AgentId, Inbox>` | Same reason. |
| Lifecycle phases | Creation → Operation → Update → Termination | `Active`, `Paused`, `Terminated` | Creation and Update are transitions, not resting states. Modelling them as states creates states no observer can ever see. |

## Invented, because the survey does not cover it

This is the part that mattered most in practice.

**None of the four protocols specifies a termination condition.** They define
how to address a peer and how to describe a capability. They are silent on what
happens when agent A messages agent B, B replies to A, and A replies to B. That
is not an edge case; it is the default behaviour of polite language models, and
it costs real money on every cycle.

Guaca supplies five independent limits (`runtime::guard`), because each catches a
different shape of runaway and every one of them alone has a hole:

| Limit | Catches |
|---|---|
| Hop depth | Long delegation chains: A → B → C → D → … |
| Run budget (model calls) | Everything else, as a hard ceiling on spend |
| Per-pair sends | Two agents ping-ponging inside the hop budget |
| Content dedup | An agent restating itself to make progress |
| Fan-out width | One call blasting the entire roster |

Two further mechanisms have no analogue in the literature:

- **`expects_reply` asymmetry** (`domain::envelope`). A human message or an
  explicit `send_message` expects an answer; an automatic reply does not. An
  agent reading a non-reply-expecting message still takes a turn to absorb it,
  but its output is filed as a note rather than delivered onward. This is what
  makes a cascade terminate in three levels instead of grinding against the hop
  limit.

- **Refusals are addressed to the model, not the developer.** Every guard
  refusal (`Refusal::explain`) states the wall, the numbers, and what to do
  instead, and is returned as a tool result. An agent told "you have already
  sent Chef 3 messages this run" stops. An agent whose message silently vanishes
  retries.

## Connectors, and the second thing the survey does not cover

The protocols describe how an agent declares what it *can do*. They say nothing
about what it *has access to*, which in practice is the question that decides
whether a task is possible. A2A's Agent Card comes closest, declaring
OpenAPI-style security schemes, but those describe how a caller authenticates to
the agent, not what the agent is already authenticated to.

The gap shows up as a specific, repeated failure. An agent with a browser signed
in to Gmail says it has no way to read mail, because nothing in its prompt says
otherwise. The access was never missing; the knowledge was. `domain::connector`
is that knowledge, and two decisions in it came from outside the protocol
literature.

**Two kinds, not one.** *Beyond Browsing: API-Based Web Agents*
([arXiv 2410.16464](https://arxiv.org/abs/2410.16464)) ran API-calling agents,
browsing agents and a hybrid over the same WebArena tasks. APIs beat browsing;
the hybrid beat both, by 24.0 points absolute over browsing alone. So an agent
is told about both a signed-in browser and a stored credential in the same list,
and it chooses. Building only the browser kind would have been simpler and
measurably worse; building only the API kind cannot be done at all, because
LinkedIn has no API to call, which is what makes the browser kind the general
one.

**Capability is observed, not declared.** The protocols assume an agent
publishes what it can do. For half of this, that is the wrong direction: the
browser is holding the cookies, so the truthful move is to read the machine
rather than to ask a person to maintain a manifest that drifts the moment they
log out. What a declaration buys, and what this gives up, is certainty: a
declared capability is exact, while an observed one has to survive the fact that
a cookie's presence does not mean a login. `google.com` cookies on a signed-out
browser and a `PHPSESSID` handed to anonymous visitors were both real false
positives on a live machine. The answer is a signature table for services worth
naming, a visited-and-identity-shaped rule for the rest, and an explicit
distinction in the prompt between what is known and what is merely likely. An
observed capability that overclaims is worse than no capability at all, because
the agent spends a turn discovering it and the operator sees a broken account
rather than an absent one.

**A capability, once real, has to be scoped where it physically lives.** A
credential is a string and goes to every machine in the group. A signed-in
session is cookies on one disk and belongs to one agent, so it appears in that
agent's own prompt as something it can use, and in every peer's roster as
something to delegate for. This is the Agent Card's discovery idea applied to a
fact rather than a claim: skills are written by the agent about itself, and
`reaches` is written by the operator.

**Two invariants, both structural rather than promised.** A credential's value
has no field on `Connector` at all, so it cannot be serialized to the webview or
rendered into a prompt; it travels from SQLite into one sandbox command's
environment and nowhere else. And there is nowhere to type a password: the
operator signs in at the real site, on the agent's screen, and Guaca records
only that it happened.

**The threat the survey's tables do not reach.** Its Operation-phase threats are
about peers. A signed-in agent's larger exposure is the page it is reading:
*BrowseSafe* ([arXiv 2511.20597](https://arxiv.org/abs/2511.20597)) makes the
point that the injections that matter drive actions rather than text, and being
signed in is precisely what makes the attempt worth making, since the payload no
longer needs to obtain access. Guaca takes the architectural half of their
layered defence, which is the half a local app can hold honestly: page content
is labelled where it enters the turn (`runtime::WEB_LABEL`) rather than only in
a system prompt written thousands of tokens earlier, credentials never enter the
model's context, and the prompt names the line a signed-in agent stops at.
Their model-based layers are not reimplemented here and are not claimed.

**Provenance.** The protocols carry no causality. Guaca's envelope records
`run_id`, `hop`, and `cause`, which is what makes a cascade reconstructable
after the fact. This is the first thing you want when five agents have been
talking and something went wrong.

## Where the survey is wrong

Worth knowing if you read it alongside this code.

- It documents MCP's transport as "HTTP with optional SSE". Streamable HTTP had
  already replaced HTTP+SSE in the 2025-03-26 spec revision, five weeks before
  the paper was submitted.
- It attributes DID-based authentication to A2A. A2A uses OpenAPI-style security
  schemes declared in the Agent Card. The DID attribution is invented.
- Its comparison table calls ACP registry-based and brokered while its own
  prose describes stateless servers with manifest discovery, and lists "offline
  agent discovery" as a strength and "registry required" as a limitation in the
  same column.
- Its roadmap ordering contradicts itself between Section 1 and Section 9.
- ACP no longer exists independently: IBM merged it into A2A under the Linux
  Foundation in September 2025, which collapses the paper's four-protocol
  taxonomy to three.

The lifecycle threat tables are the part worth keeping.

## Credit

The protocols themselves, and the people behind them:

- **MCP**: Model Context Protocol, Anthropic.
- **A2A**: Agent-to-Agent Protocol, Google. The Agent Card, the directory as a
  first-class operation, and the Update phase that card versioning comes from
  are all A2A's, and they are the ideas this app leans on hardest.
- **ACP**: Agent Communication Protocol, IBM Research / BeeAI. Typed ordered
  multipart messages are ACP's shape.
- **ANP**: Agent Network Protocol. Decentralised discovery, most of which this
  app has no use for, and one idea it does.

The survey above is what made comparing them tractable, and its
Creation/Operation/Update/Termination threat framing is used directly in
`guard.rs` and `prompt.rs`.

Two papers outside that literature shaped connectors:

- *Beyond Browsing: API-Based Web Agents*, Yueqi Song, Frank Xu, Shuyan Zhou and
  Graham Neubig, [arXiv 2410.16464](https://arxiv.org/abs/2410.16464). The
  measurement that made two kinds worth building instead of one.
- *BrowseSafe: Understanding and Preventing Prompt Injection Within AI Browser
  Agents*, Kaiyuan Zhang, Mark Tenenholtz, Kyle Polley, Jerry Ma, Denis Yarats
  and Ninghui Li, [arXiv 2511.20597](https://arxiv.org/abs/2511.20597). The
  threat model for an agent that is already logged in.

Adopting an idea is not an endorsement by any of these authors, and every
simplification recorded here is this app's own.
