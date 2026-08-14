# What Guac takes from the interoperability literature, and what it doesn't

Guac's message layer is derived from the four agent protocols surveyed in
[arXiv 2505.02279](https://arxiv.org/abs/2505.02279) (MCP, ACP, A2A, ANP). The
survey is a useful map of the design space and a poor specification: it
describes how agents address each other, never when they stop. This document
records which ideas were adopted, which were deliberately dropped, and which
gaps had to be filled from scratch.

The governing rule: an idea earns its place only if it pays off inside a
single-process, single-user, local desktop app. Most of what these protocols
specify exists to cross an organizational trust boundary. Guac has no such
boundary, so importing that machinery would be cargo cult.

## Adopted

| Idea | Source | Where it lives | Why it survived |
|---|---|---|---|
| **Agent Card** — a self-describing capability document used for discovery | A2A | `domain::agent::AgentCard` | An agent asked to "introduce yourself to everyone" needs a roster. The card is what makes `directory` answerable. |
| **Directory as a first-class operation** | A2A, ANP | `llm::tools::DIRECTORY` | Hardcoding peers into prompts breaks the moment an agent is added. Discovery at call time is strictly better and costs one tool. |
| **Typed, ordered multipart messages** rather than a bare string | ACP | `domain::envelope::Part` | A message can carry a guard notice, a tool trail, and prose without any of them being parsed back out of the others. |
| **Explicit lifecycle** | all four | `domain::agent::Lifecycle` | Pause and delete need real states. See "Reduced" below for how it was trimmed. |
| **Card versioning** | A2A Update phase | `AgentCard::version` | The only mechanism that lets a peer notice a card changed underneath it. Bumped on every edit. |
| **Lifecycle-phase threat model** | the survey's Tables 3-6 | `guard.rs`, `prompt.rs`, `config.rs` | The survey's most reusable contribution. Its Creation/Operation/Update/Termination framing is a genuinely good checklist. |
| **Prompt injection between agents treated as the primary threat** | MCP "tool poisoning", A2A "task injection" | `domain::envelope::Trust`, `runtime::prompt` | Both names describe one failure: wire content read as principal instruction. Guac tags provenance on the envelope and restates it in the system prompt. |

## Reduced

| Idea | What the protocols specify | What Guac does | Why |
|---|---|---|---|
| Agent Card hosting | Served at `/.well-known/agent-card.json` over HTTP | A row in SQLite | There is no network peer. An HTTP server to talk to yourself is pure overhead. |
| Identity | W3C DIDs, `did:wba`, DID documents, signature verification (ANP) | A UUID | DIDs solve "prove you are who you claim across an untrusted network". Inside one process there is no claim to verify. |
| Manifest signing | Sigstore, JWS, signed manifest diffs | Nothing | Signatures defend against a supply chain Guac does not have. Adding them would be security theatre with a maintenance cost. |
| Transport | JSON-RPC 2.0 or REST over HTTP, SSE, gRPC | A `tokio::mpsc` channel per agent | The protocols' transport layer exists to cross a process boundary. Guac's agents share an address space. |
| Registry / broker | Central registry with runtime registration (ACP) | A `HashMap<AgentId, Inbox>` | Same reason. |
| Lifecycle phases | Creation → Operation → Update → Termination | `Active`, `Paused`, `Terminated` | Creation and Update are transitions, not resting states. Modelling them as states creates states no observer can ever see. |

## Invented, because the survey does not cover it

This is the part that mattered most in practice.

**None of the four protocols specifies a termination condition.** They define
how to address a peer and how to describe a capability. They are silent on what
happens when agent A messages agent B, B replies to A, and A replies to B. That
is not an edge case; it is the default behaviour of polite language models, and
it costs real money on every cycle.

Guac supplies five independent limits (`runtime::guard`), because each catches a
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

**Provenance.** The protocols carry no causality. Guac's envelope records
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
