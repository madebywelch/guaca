# What OpenRouter says about a model

Two reads off one vendor catalog: what a model is good at, which is a
suggestion, and whether it can be shown a picture, which is a refusal.
`llm/catalog.rs`, `llm/modality.rs` and `src/lib/roles.ts`.

- **A model that cannot be shown a picture is one the endpoint said so about,
  and nothing else.** An endpoint that publishes no modalities, and a model that
  is not on its list, both mean what they always meant: send the picture. The
  two errors are not equal. A wrong *it can see* is what every endpoint got
  before this existed and costs one turn, refused with a message naming the
  model; a wrong *it cannot see* takes `use_screen` off an agent that was using
  it and stops delivering attachments, with nothing on screen saying why. So a
  local server with no `architecture` on its model list changes nothing at all,
  and only `input_modalities` without `image` in it subtracts anything.
  `llm/modality.rs`.
- **One value, settled once, spent in four places.** `Modalities` is resolved
  at the top of `run_turn`: the prompt says what reaches this agent, `specs`
  decides whether `use_screen` is offered, `deliver_files` decides what an
  attached picture becomes, and `not_given` refuses a screen a model asked for
  anyway. Three of four agreeing is worse than none: an agent told it is blind
  and handed a screenshot concludes the delivery failed, and one served
  `use_screen` gets a picture thrown away, which reads as a screen that came
  back blank. The fourth is not belt and braces: a model naming a tool it was
  never offered is ordinary, which is the same reason `Store::plugin_reach`
  asks again on the call path.
- **A model suggestion is ranked by capability inside a use case, never by
  OpenRouter's default order.** That order is tokens routed, which is bulk
  traffic: the same cheap high-throughput model tops eleven of the twelve use
  cases, so three suggestions built on it are the same three under every agent
  with a different sentence above them each time. The category picks the pool
  and `sort=intelligence-high-to-low` picks the order inside it. The price on
  each row is not decoration either: capability ordering ignores price, so
  without it the button is a one-click way to make every turn forty times
  dearer.
- **An unknown category is refused in `catalog.rs`, not by OpenRouter.**
  OpenRouter answers one with 200 and an empty list, so a slug it has renamed is
  indistinguishable from a use case nobody sends work to, and the dialog would
  draw nothing for exactly the agents it was built for. `ipc.contract.test.ts`
  compares the twelve in `CATEGORIES` against the twelve in `ROLES`, and the
  `#[ignore]`d test in `catalog.rs` asks the live service whether it still
  ranks all of them, which is the failure no offline suite can see.
- **`roleFor` returning nothing is the common answer, and a tie returns
  nothing too.** Most agents are a Manager or an Inbox and OpenRouter has no
  category for either. A scorer that always names its best guess puts a legal
  model under a scheduling agent, and one bad suggestion is what teaches an
  operator to ignore the good ones. Sales is the single deliberate bend: nothing
  ranks it, so its vocabulary scores into marketing.
