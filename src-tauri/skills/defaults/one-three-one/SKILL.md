---
name: one-three-one
description: "Use when the operator asks for a 1-3-1 or for their options, or a decision has several viable approaches with real trade-offs and needs a recommendation they can act on or forward. One problem, three options, one pick, then a definition of done and a plan."
license: MIT
author: "Willard Moore, adapted for Guaca"
---

# 1-3-1

A structured format for a task with several viable approaches, when the
operator needs a clear recommendation: one sentence framing the problem, three
options with their trade-offs, one recommended option, and a definition of done
and a plan for that option.

## When to use it

- The operator asks for a "1-3-1".
- The operator asks "what are my options" or "give me choices" on a decision.
- A task has several viable approaches with meaningful trade-offs: architecture,
  tooling, a vendor, a migration strategy, a process.
- The operator needs a proposal they can forward to a team or a stakeholder.

Not for simple questions with one obvious answer, for debugging, or where the
operator has already chosen an approach.

## Procedure

1. **Problem, in one sentence.**
   - State the decision or the desired outcome in a single sentence.
   - The what, not the how: no implementation details, tool names or specific
     technologies.
   - If it needs an "and", it is two problems.

2. **Options, exactly three.**
   - Three distinct, viable approaches, labeled A, B and C.
   - Each gets a short description, its pros and its cons.
   - They are genuinely different strategies, not three variations on one.

3. **Recommendation, one option.**
   - Say which you recommend and why, from the operator's context and
     priorities as you know them.
   - Be direct: this is your judgment, not a hedge.

4. **Definition of done.**
   - The specific success criteria for the recommended option: concrete,
     checkable outcomes, not aspirations.

5. **Implementation plan.**
   - Concrete steps to carry out the recommended option, with the specific
     commands, tools or actions where they apply.
   - Say who does each step: you, a named crewmate, or the operator.

If the operator picks a different option, rewrite the recommendation, the
definition of done and the plan to match it.

## Where it goes

- **The operator is in the conversation:** the 1-3-1 is your reply.
- **They are not, and the choice is theirs:** file it with `decision` so it
  waits for them: the problem as the question, the three options as short
  labeled choices, your pick as the recommendation, and the pros, cons and plan
  as context. Carry on with work that does not depend on the answer.
- **They want to forward it:** write it as a document with `write_document`.

## Check

- Exactly one problem sentence.
- Exactly three options, A, B and C, each with pros and cons.
- One recommendation that picks one option and says why.
- The definition of done and the plan both belong to the recommended option.
- If the operator chose differently, all three of those were revised to match.

## Example

Operator: "Give me a 1-3-1 for adding retry logic to our API client."

**Problem:** Calls to external services fail intermittently under load and need
graceful retry handling.

**Options:**

- **Option A: Exponential backoff with jitter in a shared utility.**
  Pros: simple and well understood; one implementation shared by every caller.
  Cons: no circuit breaking; callers still block while retrying.

- **Option B: A retry library (tenacity) as a decorator on each endpoint call.**
  Pros: configurable per endpoint; built-in circuit breakers and rate limits.
  Cons: adds a dependency; decorators can obscure control flow.

- **Option C: Queue-based asynchronous retry with dead-letter handling.**
  Pros: non-blocking; failed calls are tracked and replayable; scales well.
  Cons: much more complexity; needs a queue backend; overkill at low volume.

**Recommendation:** Option A. The call volume does not justify queue
infrastructure, and a shared utility with backoff and jitter covers the failure
cleanly without a new dependency.

**Definition of done:**
- `api_retry()` handles transient HTTP errors (429, 502, 503, 504).
- It retries up to three times with exponential backoff and jitter.
- Every external API call goes through it.
- Each retry is logged with the endpoint and the attempt number.
- Tests cover exhausted retries, immediate success, and a transient failure
  followed by success.

**Implementation plan:**
1. Create `utils/api_retry.py` with configurable maximum retries, base delay and
   retryable status codes.
2. Add jitter with `random.uniform(0, base_delay)` so clients do not retry in
   lockstep.
3. Route the existing calls in `api_client.py` through the utility.
4. Add unit tests that mock HTTP responses for each retry scenario.
5. Check it under load with a stress test against a flaky mock endpoint.
