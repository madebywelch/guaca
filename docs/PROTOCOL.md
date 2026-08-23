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
| Manifest signing | Sigstore, JWS, signed manifest diffs | Nothing | Signatures defend against a supply chain Guaca does not have. Adding them would be security theater with a maintenance cost. |
| Transport | JSON-RPC 2.0 or REST over HTTP, SSE, gRPC | A `tokio::mpsc` channel per agent | The protocols' transport layer exists to cross a process boundary. Guaca's agents share an address space. |
| Registry / broker | Central registry with runtime registration (ACP) | A `HashMap<AgentId, Inbox>` | Same reason. |
| Lifecycle phases | Creation → Operation → Update → Termination | `Active`, `Paused`, `Terminated` | Creation and Update are transitions, not resting states. Modeling them as states creates states no observer can ever see. |

## Invented, because the survey does not cover it

This is the part that mattered most in practice.

**None of the four protocols specifies a termination condition.** They define
how to address a peer and how to describe a capability. They are silent on what
happens when agent A messages agent B, B replies to A, and A replies to B. That
is not an edge case; it is the default behavior of polite language models, and
it costs real money on every cycle.

That claim was argued from first principles here before anybody had measured
it. It has since been measured. *Why Do Multi-Agent LLM Systems Fail?*
([arXiv 2503.13657](https://arxiv.org/abs/2503.13657)) annotated 1642 execution
traces from seven multi-agent frameworks and built a 14-mode failure taxonomy
from them. Two of its modes are the ones this section is about: "unaware of
termination conditions" at 12.4% of failures, and "step repetition" at 15.7%,
which is the single largest mode in its category. The five limits below are
aimed at exactly those two, and the content fingerprint is the second one
directly. The paper's own conclusion is the argument for putting them in the
runtime rather than in a prompt: these are design failures rather than model
failures, and better base models do not remove them.

Guaca supplies five independent limits (`runtime::guard`), because each catches a
different shape of runaway and every one of them alone has a hole:

| Limit | Catches |
|---|---|
| Hop depth | Long delegation chains: A → B → C → D → … |
| Run budget (model calls) | Everything else, as a hard ceiling on spend |
| Per-pair sends | Two agents ping-ponging inside the hop budget |
| Content dedup | An agent restating itself to make progress |
| Fan-out width | One call blasting the entire roster |

Two further mechanisms have no analog in the literature:

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

**The same taxonomy names the cost of overdoing this, which is the risk a
document about termination should state against itself.** "Premature
termination" is 6.2% of the failures in that dataset, and the paper attributes
it specifically to a star topology with no predefined workflow, which is this
app's shape: every agent answers to one operator and nothing prescribes the
order. Everything above pushes toward stopping, and this app has twice shipped a
bug that pushed too far: an authorized external send refused because nobody was
waiting on it, and an agent given work in a mode whose prompt told it silence
was usually right. `intent` and `ReplyMode::Assigned` are the fixes; the reason
neither was caught is that the eval suite could only see the noisy direction.
`Fault::AssignedAndSaidNothing` is the other half, and an `Assigned` turn that
produces nothing now says so in the channel rather than leaving the operator
watching an agent that has apparently stopped.

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
publishes what it can do. For half of this, that is the wrong direction:
whatever is holding the cookies knows, so the truthful move is to read it rather
than to ask a person to maintain a manifest that drifts the moment they log out.
An agent has two places that can hold them, a computer and a browser, and each
is read on its own and recorded against itself, because a session in one is not
reachable from the other and an agent told only the service name reaches for the
wrong tool. What a declaration buys, and what this gives up, is certainty: a
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

**A third kind arrived, and it is the one MCP was actually for.** A plugin is a
server the crew signs in to, which then publishes what it can do. It settles the
declared-versus-observed argument above by making the question moot: `tools/list`
is a declaration, but it is the vendor's rather than the operator's, it is
current because it was fetched at the moment of connecting, and it arrives as
schemas rather than as prose. The trade is that it only works for services that
run such a server, which is why there are three of them and a text box for
everything else. `docs/PLUGINS.md`.

**The threat the survey's tables do not reach.** Its Operation-phase threats are
about peers. A signed-in agent's larger exposure is the page it is reading:
*BrowseSafe* ([arXiv 2511.20597](https://arxiv.org/abs/2511.20597)) makes the
point that the injections that matter drive actions rather than text, and being
signed in is precisely what makes the attempt worth making, since the payload no
longer needs to obtain access. Guaca takes the architectural half of their
layered defense, which is the half a local app can hold honestly: page content
is labeled where it enters the turn (`runtime::WEB_LABEL`) rather than only in
a system prompt written thousands of tokens earlier, credentials never enter the
model's context, and the prompt names the line a signed-in agent stops at.
Their model-based layers are not reimplemented here and are not claimed.

**All of that is wording, and wording is the layer an injection is written to
beat.** *Design Patterns for Securing LLM Agents against Prompt Injections*
([arXiv 2506.08837](https://arxiv.org/abs/2506.08837)) states the principle this
falls short of: once an agent has ingested untrusted input, it must be
*impossible* for that input to trigger a consequential action, rather than
merely discouraged. Guaca had the mechanism for that and was not using it.
`request_permission` parks a turn on a person and puts two buttons in the
channel they are already reading, and it fired only when the model chose to call
it, which is the one decision an injection is in a position to talk it out of.

`runtime::needs_consent` is the structural version, and it is deliberately
narrow. Three conditions, all of which must hold: the browser action changes
something rather than reading it, this turn has already taken in a page or a
screen, and the browser is standing on a site this agent holds a session for,
in that browser rather than anywhere else. Reading is never gated, because an agent that cannot read cannot report the
attack either, and gating navigation would mean approving a click in order to
reach the click being approved. What is left is the case this paper and
BrowseSafe agree is worth paying for: the payload does not need to obtain
access, it already has the operator's, and the next press is the operator's to
allow. Once, per site, per turn: a question asked again for every press on the
same account is one an operator stops reading, and a defense nobody reads is
wording again. The grant lives on the turn and dies with it, and any page from
off that site takes it back.

The rule is a pure function, separate from the asking, because a security rule
nobody can read in one sitting is a rule nobody can check. Its tests carry the
two lookalike tricks that would defeat a careless version, a host that merely
ends with the signed-in domain and a signed-in domain parked in front of an `@`,
and both must come back as not that session. A gate that matched either would be
worse than no gate, because it would look like it had considered them.

The full version of this is CaMeL, *Defeating Prompt Injections by Design*
([arXiv 2503.18813](https://arxiv.org/abs/2503.18813)), which extracts control
and data flow from the trusted query so untrusted data can never reach the
program flow, and enforces capabilities when tools are called. It is not built
here and should not be: it costs 77% task completion against 84% undefended on
AgentDojo, and it needs an orchestrator this app has no place for. What is taken
from it is the shape of the claim. Provable beats persuaded, and where this app
cannot be provable it asks a person rather than pretending.

**Provenance.** The protocols carry no causality. Guaca's envelope records
`run_id`, `hop`, and `cause`, which is what makes a cascade reconstructable
after the fact. This is the first thing you want when five agents have been
talking and something went wrong.

## Paying for a turn with a subscription, and why only one vendor allows it

Both of the model vendors an operator is most likely to already be paying monthly
publish an OAuth flow that funds inference from a consumer plan instead of from a
metered key. Only one of them permits a third party to use it, and the split is
policy rather than protocol, so it is recorded here rather than being rediscovered
by whoever asks for the other half.

**OpenAI permits it, and says so.** ChatGPT sign-in is a published Codex
authentication method alongside API keys, and OpenAI extended it to third-party
harnesses rather than reserving it for its own clients: Sam Altman announced
ChatGPT-account sign-in for OpenClaw on 2 May 2026, and the same device-code flow
is what `subscription.rs` performs. The credential is a plan, so the calls draw
on the plan's quota and no per-token bill arrives. Guaca sends its own
`User-Agent` and the `originator` the backend expects on that endpoint; it does
not claim to be the CLI.

**Anthropic prohibits it, and enforces it.** Consumer Claude OAuth tokens are for
Claude Code and Claude.ai only. Server-side enforcement landed in January 2026,
the documentation was made explicit on 19 February 2026, and on 4 April 2026
subscription quota stopped covering third-party harnesses altogether. The policy
names the Agent SDK specifically, so there is no sanctioned client library route
either. A Claude subscription therefore cannot fund Guaca, and the flow is not
implemented: it would fail at the server, breach the Consumer Terms the operator
agreed to, and put their account at risk of revocation without notice. Claude
models still reach Guaca the way they always have, through an API key or through
OpenRouter, which remains the default endpoint and the default model.

This is worth restating because the asymmetry is not obvious from the outside.
The two flows look near-identical — OAuth, PKCE, a rotating bearer token, a
plan-scoped claim in an ID token — and a reasonable person assumes that
implementing one means the other is a weekend of work. The blocker is a term of
service and a server-side check, and neither is something this repo can engineer
around.

## Considered and declined

**A verifier agent.** The failure taxonomy above puts task verification at 17.3%
of failures, split between no verification and incorrect verification, and its
own intervention study adds a high-level objective check to one framework for
+15.6% task success. Read alone that is a mandate to build one. It is not.
*MAS-ProVe* ([arXiv 2602.03053](https://arxiv.org/abs/2602.03053)) is the
systematic version of the same question, across three verification paradigms,
two granularities, five verifiers and six frameworks, and finds that
process-level verification does not consistently improve performance and
frequently has high variance. A verifier that is unreliable in a crew this small
would spend a model call per step to produce a second opinion nobody can trust,
and every call it spends is on the operator's bill. The eval and trajectory
suites verify at test time instead, where being slow and thorough is free.

**A judge in the eval suite.** `eval.rs` decides every fault from the envelopes
and scores nothing, which was a taste decision when it was written: a fault that
needs a judgment call is one nobody can act on when it fails. *Gaming the
Judge* ([arXiv 2601.14691](https://arxiv.org/abs/2601.14691)) is the measured
argument for it. Rewriting an agent's reasoning while holding its actions and
observations fixed inflated the false positive rate of state-of-the-art judges
by up to 90% across 800 trajectories, and their conclusion is that judging has
to verify claims against observable evidence. An envelope is observable
evidence. The taxonomy above annotates its own dataset with an LLM judge, which
is the right trade at 1642 traces and the wrong one for a suite that gates a
build.

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
- **ANP**: Agent Network Protocol. Decentralized discovery, most of which this
  app has no use for, and one idea it does.

The survey above is what made comparing them tractable, and its
Creation/Operation/Update/Termination threat framing is used directly in
`guard.rs` and `prompt.rs`.

Four papers outside that literature shaped the parts the protocols do not
reach:

- *Why Do Multi-Agent LLM Systems Fail?*, Mert Cemri and others, UC Berkeley,
  [arXiv 2503.13657](https://arxiv.org/abs/2503.13657). The measurement behind
  the termination argument, and the one that names the cost of overcorrecting.
- *Design Patterns for Securing LLM Agents against Prompt Injections*,
  Beurer-Kellner, Creţu, Debenedetti, Tramèr and others,
  [arXiv 2506.08837](https://arxiv.org/abs/2506.08837). The principle that made
  `needs_consent` structural rather than another paragraph of prompt.
- *Defeating Prompt Injections by Design*, Debenedetti, Shumailov, Carlini,
  Tramèr and others, [arXiv 2503.18813](https://arxiv.org/abs/2503.18813). The
  full version of that idea, and the cost of it.
- *Gaming the Judge: Unfaithful Chain-of-Thought Can Undermine Agent
  Evaluation*, Khalifa and others,
  [arXiv 2601.14691](https://arxiv.org/abs/2601.14691). Why `eval.rs` decides
  rather than scores.

Two more shaped connectors:

- *Beyond Browsing: API-Based Web Agents*, Yueqi Song, Frank Xu, Shuyan Zhou and
  Graham Neubig, [arXiv 2410.16464](https://arxiv.org/abs/2410.16464). The
  measurement that made two kinds worth building instead of one.
- *BrowseSafe: Understanding and Preventing Prompt Injection Within AI Browser
  Agents*, Kaiyuan Zhang, Mark Tenenholtz, Kyle Polley, Jerry Ma, Denis Yarats
  and Ninghui Li, [arXiv 2511.20597](https://arxiv.org/abs/2511.20597). The
  threat model for an agent that is already logged in.

Adopting an idea is not an endorsement by any of these authors, and every
simplification recorded here is this app's own.
