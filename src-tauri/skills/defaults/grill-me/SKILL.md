---
name: grill-me
description: "Use when the operator asks to be grilled or to stress-test a plan or idea, or before complex or hard-to-reverse work (a migration, a schema, auth, payments, a launch) whose plan is vague or has open decisions. Interviews in rounds, with a recommendation on every question."
license: MIT
author: "Rafael Zendron (rafaumeu) + Matt Pocock (mattpocock/skills, grilling) + Hermes Agent, adapted for Guaca"
---

# Grill me

Stress-tests a plan through structured, adversarial questioning before any work
starts. Model the plan as a design tree, where every decision branches into the
decisions that hang off it, and interview the operator in rounds until every
branch is resolved and nothing is silently assumed.

## When to use it

- The operator says "grill me", "interview my plan", "stress-test this idea".
- Before complex work: auth flows, schema changes, migrations, payments, a
  launch, anything expensive to undo.
- A plan has unresolved decisions or reads as vague.
- Before a plan is split into pieces for crewmates or handed to a coding job.

Not for reviewing work that already exists, and not for simple one-off tasks.

## Frontier rounds

Map the plan as a design tree. The frontier is every decision whose
prerequisites are already settled: the questions you can ask now without
guessing at answers you have not heard yet.

Work in rounds. Ask the whole current frontier in one reply, numbered, each
question carrying your recommended answer, and then end your turn: the
operator's answers arrive as their next message. A question whose answer
depends on another question still open in this round belongs to a later round,
not this one.

Format each round like this, numbering on from the previous round so an answer
like "Q5: yes" is never ambiguous:

```
Q1. <title>: <the question, with options if there are any>
Recommendation: <your answer, and one line on why>

Q2. <title>: <the question>
Recommendation: <your answer, and why>
```

Ask the rounds in your reply, not with `ask_operator`: that tool takes one
question at a time and is for a single fork in the work, not an interview. If
the plan is a crewmate's rather than the operator's, grill them the same way
with `send_message` (intent `work`), and keep the rounds few and full, since
every exchange counts toward the limit on messages between two agents.

Each answer reshapes the tree: settled decisions push the frontier outward and
unblock the questions that hung on them. Recompute the frontier and ask the
next round.

## Facts are your job; decisions are the operator's

When a frontier question needs a fact, find it yourself. Never ask the operator
for something you could look up:

- Attachments: `read_file`.
- A repository, if you have one: `shell` for a quick look (`git log`,
  `git grep`, reading a file), or `code` for an exploration that means reading a
  lot of the code.
- The web, if you have `browse`.
- Your `notebook`, your memory, and the crew's `calendar`.
- A crewmate whose skills cover it: `directory`, then `send_message`.

Do not block on an exploration. `code` and `send_message` answer later, as new
messages, so hold back only the questions downstream of what you asked and put
the rest of the frontier to the operator now.

## What the tree should cover

**Understanding: the real goal and its edges.**
- What is the actual objective? What is explicitly in scope, and what is out?
- What are the constraints: time, money, people, technology? Who is it for?

**Each technical decision.**
- Why this approach and not the obvious alternative? What happens if it fails?
- What is the worst case, and how would it be rolled back?
- If the codebase or the crew already has a pattern for this, name it.

**Edge cases.**
- What happens if a user does the unexpected thing? If a dependency goes down?
- What if volume is a hundred times what is expected? What are the security
  implications?

## Synthesis, when the frontier is empty

1. Summarize every decision as a bullet.
2. List what is still open, and what is explicitly out of scope.
3. Ask: "Aligned? Should I start, or adjust anything?"

Do not act on the plan until the operator confirms. Once they do, the synthesis
is the start of the brief: a `code` job, or a crewmate handed a piece of the
work, cannot see this conversation, so everything settled here has to travel
with the task. If it is long, hand it over as a document with `write_document`.

## Pitfalls

1. **Asking out of dependency order.** A question that depends on an unanswered
   one is a guess wearing a question mark. Keep it for a later round.
2. **Asking for what you could look up.** Read the code, the attachments and
   your notes first.
3. **Accepting "I don't know" as final.** Offer options, explain the
   trade-offs, make a recommendation.
4. **Doing the work during the interview.** Alignment only: no `code` job, no
   edits and no work handed to crewmates until the explicit go-ahead.
5. **Being too agreeable.** Your job is to find problems. If everything looks
   fine, look harder.
6. **Ignoring the operator's language.** Interview in whatever language they
   write in.

## Check before you synthesize

- Every question in a round had all its prerequisites settled.
- Every question carried a recommendation.
- Facts came from looking, not from asking.
- The frontier is empty: no branch was silently assumed.
- The summary lists every decision and every open item.
- The operator confirmed alignment before any work started.
