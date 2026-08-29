# Providers

The two ways a turn is paid for that are not an API key: a ChatGPT sign-in, and
the `claude` program. *A subscription is a second provider* in `AGENTS.md` is
the rule these sit under, `docs/PROTOCOL.md` has the dates and the sources, and
the code is `llm/codex.rs`, `llm/claude.rs` and `subscription.rs`.

- **A ChatGPT access token's `exp` is not when it stops working.** OpenAI mints
  one ten days out and the backend refuses it after about three, with
  `token_expired`. The local claim is the only signal on the machine and it is
  wrong in the direction that strands an operator: refreshing against it alone
  left a dead token in place for a week, refused every turn, and kept reporting
  a healthy sign-in in Settings, because "signed in" was a file existing. So a
  401 from the backend is what triggers a refresh, and the same request goes
  again under the new token. `Subscription::renew`, and *A token's `exp` is a
  floor on its life* in `docs/ARCHITECTURE.md`.
- **A refresh is serialized, and one the service refuses forgets the sign-in.**
  The refresh token rotates, so a crew that all hit the dead token at once would
  race to retire each other's and the losers would hold one the service already
  threw away. And a 4xx from the token endpoint is the sign-in genuinely being
  over: the file goes, so Settings offers signing in rather than signing out. A
  5xx is the service having a bad minute and costs nobody their sign-in.
- **Every isolation flag on the `claude` command line is load-bearing, and the
  measurement is why.** Started the ordinary way the program loads the
  operator's own MCP servers, settings and hooks, and an agent in this app
  inherits all of it: measured at 2.1.247, one trivial reply cost 104,371 input
  tokens and named 200-odd tools, against 783 tokens and none with `--tools ""`,
  `--strict-mcp-config` over an empty `--mcp-config`, and `--setting-sources ""`.
  That is not tidiness. It is the difference between a crew and a crew that can
  send mail from the operator's own inbox because they connected Gmail in a
  terminal last week.
- **A reply on that provider lands whole, and the thinking is what moves.** The
  answer is a JSON document still being written, so streaming it would mean
  drawing a half-decoded escape into a channel, which is worse than a message
  that arrives at once. The thinking and the prose the model writes on its way
  there both go to `Token::Reasoning`, are shown, and are dropped, exactly as
  everywhere else. It is the one way this provider looks different on screen,
  and it is a decision rather than a gap.
- **A refusal from that program is not a failure it had, and it is the one thing
  there worth another draw.** The model's safety check can stop an answer on a
  call that succeeded, and the frame then reads `subtype: "success"` with
  `is_error` true, `api_error_status` null, no `structured_output`, and a
  `result` that opens `API Error:` and closes with the category that fired.
  On the report this was written from that category was `reasoning_extraction`,
  which runs on what the model wrote rather than on what the operator asked for. Told apart by the
  error flag alone it lands in the arm that means a dead sign-in or a spent
  plan: never retried, and answered with a paragraph of the program's own advice
  about rephrasing in a new session and changing `/model`, neither of which
  exists here and the second of which this app passes on purpose. `stop_reason`
  is the field that separates them, `LlmError::ModelRefused` is transient and so
  gets the turn's three attempts, and the sentence after the program's words is
  this app's.
- **The `claude` result frame is snake case and is deliberately not renamed.**
  It mixes conventions — `modelUsage` sits beside `total_cost_usd` — so a
  blanket `rename_all` is right about the fields it was written against and
  silently wrong about the next one. Every field is optional, so wrong is not an
  error: `structured_output` deserializes to absent, and the symptom is replies
  going missing rather than anything failing.
- **A model named on a group running on Claude is kept and never used.** There
  is no third model field and there will not be one: which model runs is the
  program's own setting, and this app passes no `--model` for the reason the
  coding harness passes none. Kept, because an operator who tries Claude for an
  hour and goes back has to find their model where they left it. Both panels say
  so on the row, because a model field that is quietly ignored is the one thing
  nothing else on screen would explain.
