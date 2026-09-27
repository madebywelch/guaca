---
name: grounded-citations
description: "Use when an answer or document rests on facts you read rather than knew: research, comparisons, the current state of something, fact-checks, anything the operator will want to check. Inline [n] citations, a Sources list, verbatim quotes, and [unverified] for the rest."
license: MIT
author: "Hermes Agent + Teknium, adapted for Guaca"
---

# Grounded citations

Every claim taken from an outside source gets an inline numbered citation, and
the answer ends with a `Sources:` list. The numbers come from a list you keep
while you read, never from memory afterward: a URL enters the list only when a
tool has just returned that page, and the prose only ever carries the small
number it was given. A wrong number is then something you can catch (it is not
on the list), and a wrong URL cannot happen, because you never typed one from
memory.

For high-stakes work the same list becomes a fact-checking chain: each source
carries the verbatim sentence that supports the claim, claims from your own
knowledge are marked `[unverified]`, and nothing is delivered citing a source
that has no quote behind it.

This covers replies in the channel, messages to crewmates, and documents you
write with `write_document`.

## When to use it

Use it whenever an answer or a document rests on information you retrieved
rather than knew:

- Research, comparisons, news summaries, "what is the current state of X".
- Any document that quotes, paraphrases or reports outside facts: reports,
  briefs, specs, decks, pages.
- Fact-finding where the operator will want to check your work.
- Synthesis across sources that disagree and must each be attributed.
- Findings you send a crewmate, who may pass the claim on without its source.

Skip inline citations when the lookup is incidental to another task (a version
check in the middle of a coding job), in casual conversation, and in creative
writing. Mention a URL only if the operator would plausibly want the link.

## Where sources come from

Cite only what you actually opened in this task. What you can open depends on
what you were given:

- **Attachments.** `read_file` reopens a file the operator or a crewmate sent.
  Cite it by file name and page or section.
- **A browser.** If you have `browse`, `open` the page and `read` it. Cite the
  page you read, not the search results that led you there.
- **A computer.** If you have one, `run_command` with `curl` fetches a page.
- **Connectors.** A connector's tools return records from the operator's
  accounts; cite each by its link or identifier.
- **A repository.** If you have one, `shell` reads its files and history; cite
  by path and commit.
- **Crewmates.** A crewmate's message is not a source. Ask for the URL and the
  exact sentence they read, and cite that.

If you have none of these for the claim at hand, do not cite from memory. Find
a crewmate in `directory` whose skills cover the lookup and send it to them, ask
the operator for the source, or deliver the claim marked `[unverified]` and say
plainly what you could not check.

## Procedure

1. **Start a list for the task.** One line per source, `[n] title: URL` (or
   file name and page), numbered in the order you first used them. For work
   that spans turns or several documents, keep the list in your `notebook` (for
   example `sources/vendor-pricing.md`, extended with `append`) so the numbers
   survive the turn. Start a new list only for a new task; continuing work
   keeps the list whose numbers are already in a draft.

2. **Register at retrieval time.** When a tool returns a page you will use,
   give it the next number then, copying the URL from that tool result, and do
   it before writing prose about it. The same page keeps the same number for
   the whole task: a trailing slash, a tracking parameter or a `#fragment` does
   not make it a new source. Adding sources afterward, reconstructed from the
   draft, is the failure this skill exists to prevent.

3. **Cite while drafting.** Put the number immediately after each sentence the
   source supports:

   ```
   Ice floats because it is less dense than liquid water.[1][2]
   ```

   - No space before the bracket, and each number in its own brackets: `[1][2]`,
     not `[1, 2]`.
   - At most three per sentence. Past three, a citation stops saying which
     source carries the claim.
   - Cite each sentence as you write it, never as one dump at the end of a
     paragraph. Attribution added afterward finds a source that plausibly
     matches a sentence already written, which is how a citation ends up
     supporting something the page does not say.
   - Only numbers on your list. Never invent one, and never cite a source you
     did not open.
   - Claims from your own knowledge get no number.
   - When sources conflict, give both readings, each with its own number.
   - Take figures, dates and names in the source's own words, not from a
     summary of it: every restatement is a chance for a number to drift. Say
     "no source found for X" rather than smoothing over a gap.

4. **End with the Sources list,** copied from your list rather than retyped:
   only the numbers the text cites, in order.

   ```
   Sources:
   [1] Density of ice: https://example.com/ice
   [2] Water, phase behavior: https://example.com/water
   ```

5. **Check before delivering.** Every `[n]` in the text is on your list. The
   Sources list has exactly the cited numbers, with the URLs from your list. No
   URL appears that you did not open in this task. A source on your list that
   nothing cites usually means a claim lost its citation while you edited; find
   it.

## Where the list goes

- **A reply, or a message to a crewmate:** a plain `Sources:` block at the end.
- **A document** (`write_document`): a `## Sources` section at the end of the
  document itself, because its reader may never see the conversation.
- **A table or spreadsheet:** a `source` column holding the number, and the list
  below the table or on its own sheet. No URLs in data cells.
- **Slides:** `[n]` in the bullet and one Sources slide at the end. Never a URL
  in a body bullet.
- **Several pages from one piece of work:** one list shared across all of them,
  so `[7]` means the same source everywhere, and a Sources section on each page.
- **Code and configuration:** no citations inside generated code. If provenance
  matters, it goes in the commit message, the pull request, or a header in the
  accompanying document.

Never mix two numbering systems in one document.

## Sweeps across kinds of source

"What are people saying about X" is not one search. Cover the kinds of source
you can reach (official documentation and announcements, changelogs and release
notes, forums and community threads, issue trackers and code) and attribute
every claim to where it came from. Keep opinion apart from measurement: a forum
thread is evidence that users report something, not that it is true, so pair it
with a primary source or label it as sentiment. Report coverage gaps ("the forum
search returned nothing newer than March") instead of silently narrowing to what
worked.

## Fact-checking mode

For medical, legal, financial and safety questions, disputed claims, or
whenever the operator asks for a fact-check, upgrade from citations to evidence.

1. **A verbatim quote for each source.** Copy the sentence that carries each
   claim out of the text the tool returned, character for character. Never
   retype it, paraphrase it, or rebuild it from memory. Quote it as a reader
   sees it, without the page's link syntax or markup, and take it from the page
   itself, never from a search result's snippet. Before delivering, check every
   quote against what you actually read in this task: a quote you cannot find
   there is not evidence, and the fix is to find the real sentence or drop the
   claim, not to reword until it sounds right.

2. **Mark model-knowledge claims `[unverified]`.** A load-bearing claim you
   could not source gets the marker in place of a citation:

   ```
   The refactor likely predates the 2.0 release.[unverified]
   ```

   The goal is declared provenance for every factual claim, not a citation on
   every sentence: each one carries either a number or the marker, while
   framing and your own reasoning need neither. If a claim can be checked,
   check it; the marker is for what genuinely cannot be. A fact-check dominated
   by `[unverified]` needed more retrieval, not more markers, and its summary
   should say so.

3. **Cross-check disputed facts against a second, independent source.** One
   source is reporting; two independent sources are corroboration. When they
   disagree, cite both readings with their own quotes and say which you weight
   and why.

4. **Deliver the evidence with the sources.** Each source's quotes go beneath
   it, so the reader can follow claim, source and exact wording without taking
   anything on faith:

   ```
   ## Sources
   [1] Density of ice: https://example.com/ice
       > Ice is about 9% less dense than liquid water.
   ```

   Every cited source has at least one quote. A cited source with none fails
   the check: go back and read it, or drop the claim.

## Pitfalls

- **Registering after writing.** The list is filled from tool results as they
  arrive, never reconstructed from the draft.
- **Renumbering mid-task.** If a draft cites `[4]`, `[4]` stays that source
  until the task is over.
- **Typing a URL from memory into the Sources list.** A hand-typed URL is itself
  an unverified claim.
- **Citing a search snippet as if you read the page.** A result's description
  supports only what it literally says. Open the page when the claim needs its
  body.
- **Over-citing.** Three is the ceiling; a number on every clause hides which
  source carries the load.
- **Merging numbered lists.** When crewmates contribute, ask them for URLs and
  quotes, not their numbers. The agent writing the deliverable owns the one list
  and numbers everything from it.
- **Paraphrasing into a quote.** A quote that is not verbatim fails the check;
  find the sentence.
- **Using `[unverified]` as an escape hatch.** It marks the rare claim that
  cannot be sourced. If most sentences carry it, the task needed more reading.
