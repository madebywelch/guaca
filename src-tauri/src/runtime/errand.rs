//! Running errands: the agent's own model, once per brief, inside the turn
//! that sent them.
//!
//! What an errand is and what bounds it is `domain/errand.rs`. This is the
//! loop, and the one decision it makes is where it runs: inside the sending
//! turn's tool call, on that turn's run, rather than as a job of its own. That
//! is what lets everything the runtime already does for a run cover an errand
//! with no second copy of it:
//!
//! - **The budget.** Every model call an errand makes is a step claimed from
//!   the run, so `max_steps_per_run` is still the one ceiling on spend and the
//!   trajectory suite still reads `steps == calls + failures` off the same
//!   ledger.
//! - **The stop.** The operator stops a conversation, not an agent, and an
//!   errand is part of the conversation that sent it. It looks for the stop at
//!   the boundaries a turn does, and its model calls are raced against it the
//!   same way.
//! - **Settling.** The sending turn is still holding its envelope while its
//!   errands work, so the run cannot be reported finished under them.
//! - **The answer.** It lands in the context that asked for it, as the result
//!   of the call, rather than as a message on a run of its own. `code` needs
//!   the other shape because a job outlives the turn by forty minutes; an
//!   errand is bounded to a few, and a turn that had to be woken again to read
//!   its answer would lose everything it knew when it asked.
//!
//! An errand acts as the agent: the same card, so the same tools, the same
//! gates and the same places, and every call it makes is drawn in the agent's
//! own trail. What it does not share is state a turn keeps for itself (what it
//! has read, what it has attached), which each errand keeps apart and hands
//! back.

use std::collections::HashSet;
use std::sync::Arc;

use super::{forget_old_screens, Reading, Runtime, ToolResult, SCREEN_NOW};
use crate::config::InferenceConfig;
use crate::domain::agent::AgentCard;
use crate::domain::attachment::Attachment;
use crate::domain::envelope::{NoticeKind, Part, ToolOutcome};
use crate::domain::errand::{self, Ending, Errand};
use crate::domain::ids::{MessageId, RunId};
use crate::domain::plugin::PluginKind;
use crate::domain::promise;
use crate::llm::modality::Modalities;
use crate::llm::openrouter::{ChatMessage, ChatRequest, ToolCall, ToolSpec};
use crate::llm::tools::{self, ToolInvocation};
use crate::runtime::events::UiEvent;
use crate::runtime::guard::GuardLimits;

/// What an errand borrows from the turn that sent it, read once by that turn.
///
/// Every field is something the turn already decided before its first call,
/// and the errand is held to the same answer rather than reading any of it
/// again: a model or a tool list resolved twice can disagree with itself.
pub(super) struct Sender<'a> {
    pub card: &'a AgentCard,
    pub run_id: RunId,
    /// The placeholder the sending turn is writing into. An errand's calls are
    /// reported against it, so the operator watches them happen in the turn
    /// that sent it rather than nowhere.
    pub stream_id: MessageId,
    pub inbound_hop: u16,
    pub cause: Option<MessageId>,
    pub settled: bool,
    pub limits: GuardLimits,
    pub inference: &'a InferenceConfig,
    pub model: &'a str,
    pub modalities: Modalities,
    /// The tools the sending turn was offered, in the order it was offered
    /// them. An errand is offered these less the three it is never given.
    pub offered: &'a [ToolSpec],
    pub named: &'a [PluginKind],
}

impl Runtime {
    /// Sends the errands one call asked for and waits for all of them.
    ///
    /// Returns the call's result for the sending turn and, separately, every
    /// call the errands made, which belong in that turn's trail before the
    /// errand call itself: they are what it did.
    pub(super) async fn send_errands(
        &self,
        from: &Sender<'_>,
        call: &ToolCall,
        attached: &mut Vec<Attachment>,
    ) -> (ToolResult, Vec<Part>) {
        let arguments = call.parsed_arguments().unwrap_or(serde_json::Value::Null);
        self.inner.events.emit(UiEvent::ToolStarted {
            message_id: from.stream_id,
            call_id: call.id.clone(),
            name: tools::ERRAND.to_string(),
            arguments: arguments.clone(),
        });

        let (rendered, outcome, trail) = match tools::parse(call, from.named) {
            Err(err) => {
                (err.guidance(), ToolOutcome::Failed { error: err.to_string() }, Vec::new())
            }
            Ok(ToolInvocation::Errand { briefs }) if briefs.len() > errand::MOST_AT_ONCE => {
                let reason = format!(
                    "Refused: you asked for {} errands and at most {} run at once, so none was \
                     sent and nothing was spent. Send the {} that matter most, or fewer with \
                     wider briefs.",
                    briefs.len(),
                    errand::MOST_AT_ONCE,
                    errand::MOST_AT_ONCE,
                );
                (reason.clone(), ToolOutcome::Refused { reason }, Vec::new())
            }
            Ok(ToolInvocation::Errand { briefs }) => {
                let offered = tools::for_errands(from.offered.iter().cloned());
                // Held by whichever errand first reaches for a computer, a
                // browser or a terminal, for as long as that errand lasts.
                // Each of the three is one thing, and two errands working one
                // page or one work tree at once each undo what the other did.
                let places = Arc::new(tokio::sync::Mutex::new(()));
                let ran = futures_util::future::join_all(briefs.iter().enumerate().map(
                    |(index, brief)| self.run_errand(from, &offered, &places, call, index, brief),
                ))
                .await;

                let mut errands = Vec::with_capacity(ran.len());
                let mut trail = Vec::new();
                for (done, parts, files) in ran {
                    errands.push(done);
                    trail.extend(parts);
                    // Handed on to the answer the sending turn is composing,
                    // which is where a document an errand made belongs: the
                    // errand has no answer of its own that anybody reads.
                    for file in files {
                        if !attached.iter().any(|held| held.digest == file.digest) {
                            attached.push(file);
                        }
                    }
                }
                tracing::info!(
                    agent = %from.card.name,
                    run = %from.run_id,
                    errands = errands.len(),
                    model_calls = errands.iter().map(|e| e.model_calls).sum::<u32>(),
                    endings = ?errands.iter().map(|e| &e.ending).collect::<Vec<_>>(),
                    "errands came back"
                );
                let outcome = if errand::all_failed(&errands) {
                    ToolOutcome::Failed { error: errand::summary(&errands) }
                } else {
                    ToolOutcome::Ok { summary: errand::summary(&errands) }
                };
                (errand::for_caller(&errands), outcome, trail)
            }
            // The turn only sends a call here once it has parsed as an errand,
            // so this is a defect rather than a model's mistake, and it says
            // so rather than dispatching something nothing checked.
            Ok(_) => {
                let error = "a call that was not an errand reached the errand path".to_string();
                tracing::error!(agent = %from.card.name, tool = %call.name, "{error}");
                (
                    format!("Error: {error}. Nothing was sent."),
                    ToolOutcome::Failed { error },
                    Vec::new(),
                )
            }
        };

        let part = Part::tool_call(tools::ERRAND, arguments, outcome);
        self.inner.events.emit(UiEvent::ToolFinished {
            message_id: from.stream_id,
            call_id: call.id.clone(),
            part: part.clone(),
        });
        (ToolResult { rendered, part, image: None }, trail)
    }

    /// One errand, from its brief to its answer.
    ///
    /// The round loop of `run_turn` with everything a turn does for the
    /// operator taken out: no placeholder, no intake, no reply. What is left is
    /// the budget, the stop and the tools, each exactly as the turn has them.
    async fn run_errand(
        &self,
        from: &Sender<'_>,
        offered: &[ToolSpec],
        places: &Arc<tokio::sync::Mutex<()>>,
        call: &ToolCall,
        index: usize,
        brief: &str,
    ) -> (Errand, Vec<Part>, Vec<Attachment>) {
        let card = from.card;
        let rounds = errand::ROUNDS.min(from.limits.max_tool_rounds);
        let mut messages = vec![
            ChatMessage::system(errand::instructions(&card.name)),
            ChatMessage::user(errand::brief(brief)),
        ];

        // A turn's own state, one of each per errand. Sharing `reading` would
        // let a page one errand read gate a click another errand chose for
        // reasons of its own; sharing `attached` would hand one errand's file
        // to another's answer.
        let mut reading = Reading::default();
        let mut addressed = HashSet::new();
        let mut attached: Vec<Attachment> = Vec::new();

        let mut trail: Vec<Part> = Vec::new();
        let mut said = String::new();
        let mut model_calls = 0u32;
        let mut holding: Option<tokio::sync::OwnedMutexGuard<()>> = None;
        let mut ending = Ending::OutOfRounds { rounds };

        'rounds: for _ in 0..rounds {
            if self.stopped(from.run_id) {
                ending = Ending::Stopped;
                break;
            }
            let claimed = {
                self.inner
                    .guard
                    .lock()
                    .run_within(from.run_id, from.limits)
                    .reserve_step_leaving(errand::HEADROOM)
            };
            if !claimed {
                ending = Ending::OutOfBudget;
                break;
            }

            let request = ChatRequest {
                model: from.model.to_string(),
                messages: messages.clone(),
                tools: offered.to_vec(),
                temperature: None,
            };
            let completion = match self
                .stream_with_retries(from.inference, &request, from.run_id, card.id, None)
                .await
            {
                Ok(Some(completion)) => completion,
                // Stopped mid-call. The step goes back for the reason the
                // turn's does: nothing was answered and nothing failed, so a
                // step left claimed is a bill naming a call that never was.
                Ok(None) => {
                    self.inner.guard.lock().run_within(from.run_id, from.limits).release_step();
                    ending = Ending::Stopped;
                    break;
                }
                Err(err) => {
                    // Written where the turn writes its own, and for the same
                    // two readers: the operator, who should know the provider
                    // refused something even though the turn carried on, and
                    // the run's bill, which counts a failed call by its notice.
                    self.notice(
                        card.id,
                        from.run_id,
                        from.cause,
                        NoticeKind::UpstreamError,
                        format!("An errand {} sent could not reach the model: {err}", card.name),
                    );
                    ending = Ending::Failed { error: err.to_string() };
                    break;
                }
            };
            model_calls += 1;
            self.count_tokens(card, from.run_id, from.model, completion.usage);

            if !completion.content.trim().is_empty() {
                said = completion.content.clone();
            }

            if completion.tool_calls.is_empty() {
                // The answer, and the same test the turn applies to its own
                // last sentence. A turn that closes on a promise is given a
                // round to keep it; an errand is not, because the turn that
                // sent it is still there to read the promise and decide.
                ending = if completion.content.trim().is_empty()
                    || promise::promises_work(&completion.content).is_some()
                {
                    Ending::Unfinished
                } else {
                    Ending::Finished
                };
                said = completion.content;
                break;
            }

            messages.push(ChatMessage::Assistant {
                content: (!completion.content.is_empty()).then(|| completion.content.clone()),
                tool_calls: completion.to_wire_tool_calls(),
            });

            for inner in &completion.tool_calls {
                if self.stopped(from.run_id) {
                    ending = Ending::Stopped;
                    break 'rounds;
                }
                // Unique across the turn's whole trail. The model numbers its
                // calls from one in every conversation, and three errands each
                // calling `call_0` would be three chips the UI files as one.
                let shown = ToolCall {
                    id: format!("{}:{}:{}", call.id, index + 1, inner.id),
                    ..inner.clone()
                };

                let invocation = tools::parse(inner, from.named).ok();
                let outcome = match invocation.as_ref().and_then(tools::withheld_from_errands) {
                    Some(tool) => self.refuse_in_errand(from, &shown, tool),
                    None => {
                        if invocation.as_ref().is_some_and(tools::reaches_a_place)
                            && holding.is_none()
                        {
                            match self
                                .until_stopped(from.run_id, card.id, places.clone().lock_owned())
                                .await
                            {
                                Some(guard) => holding = Some(guard),
                                None => {
                                    ending = Ending::Stopped;
                                    break 'rounds;
                                }
                            }
                        }
                        self.execute_tool(
                            card,
                            from.run_id,
                            from.stream_id,
                            from.inbound_hop,
                            from.cause,
                            from.settled,
                            &mut addressed,
                            &mut reading,
                            &mut attached,
                            &shown,
                            from.modalities,
                            from.named,
                        )
                        .await
                    }
                };

                let file_image = matches!(&outcome.part,
                    Part::ToolCall { name, .. } if name == tools::READ_FILE);
                trail.push(outcome.part);
                messages.push(ChatMessage::Tool {
                    tool_call_id: inner.id.clone(),
                    content: outcome.rendered,
                });
                if let Some(image) = outcome.image {
                    if file_image {
                        messages.push(ChatMessage::user_seeing(
                            "The saved attachment looks like this.",
                            image,
                        ));
                    } else {
                        forget_old_screens(&mut messages);
                        messages.push(ChatMessage::user_seeing(SCREEN_NOW, image));
                    }
                }
            }
        }
        drop(holding);

        let calls = trail.len();
        (
            Errand { brief: brief.to_string(), ending, answer: said, calls, model_calls },
            trail,
            attached,
        )
    }

    /// A call an errand is never given, refused where the operator can see it.
    ///
    /// Drawn like any other call, with its reason, for the reason a refused
    /// send keeps its own chip: it is the runtime stopping something, and that
    /// is the part of the trail somebody may have to act on.
    fn refuse_in_errand(
        &self,
        from: &Sender<'_>,
        call: &ToolCall,
        tool: &'static str,
    ) -> ToolResult {
        let arguments = call.parsed_arguments().unwrap_or(serde_json::Value::Null);
        self.inner.events.emit(UiEvent::ToolStarted {
            message_id: from.stream_id,
            call_id: call.id.clone(),
            name: tool.to_string(),
            arguments: arguments.clone(),
        });
        let reason = match tool {
            tools::SEND_MESSAGE => format!(
                "Refused: an errand cannot message anyone, because the answer would arrive after \
                 you are gone. Put what should be said, and to whom, in your answer, and {} will \
                 decide whether to send it.",
                from.card.name
            ),
            tools::UPDATE_MEMORY => format!(
                "Refused: an errand cannot rewrite {name}'s memory, because other errands may be \
                 writing it at the same moment and each write replaces the whole file. Put what \
                 is worth remembering in your answer, and {name} will decide whether to keep it.",
                name = from.card.name
            ),
            _ => "Refused: an errand cannot send errands of its own. Do this part yourself, or \
                  say in your answer that it needs doing and the agent that sent you will decide."
                .to_string(),
        };
        let part =
            Part::tool_call(tool, arguments, ToolOutcome::Refused { reason: reason.clone() });
        self.inner.events.emit(UiEvent::ToolFinished {
            message_id: from.stream_id,
            call_id: call.id.clone(),
            part: part.clone(),
        });
        ToolResult { rendered: reason, part, image: None }
    }
}
