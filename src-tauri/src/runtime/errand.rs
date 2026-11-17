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

pub(super) struct Sender<'a> {
    pub card: &'a AgentCard,
    pub run_id: RunId,
    pub stream_id: MessageId,
    pub inbound_hop: u16,
    pub cause: Option<MessageId>,
    pub settled: bool,
    pub limits: GuardLimits,
    pub inference: &'a InferenceConfig,
    pub model: &'a str,
    pub modalities: Modalities,
    pub offered: &'a [ToolSpec],
    pub named: &'a [PluginKind],
}

struct Place {
    url: Option<String>,
    used: bool,
}

impl Runtime {
    pub(super) async fn send_errands(
        &self,
        from: &Sender<'_>,
        call: &ToolCall,
        reading: &mut Reading,
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
                let places = Arc::new(tokio::sync::Mutex::new(Place {
                    url: reading.url.clone(),
                    used: false,
                }));
                let sent: &Reading = reading;
                let ran = futures_util::future::join_all(briefs.iter().enumerate().map(
                    |(index, brief)| {
                        self.run_errand(from, &offered, &places, sent, call, index, brief)
                    },
                ))
                .await;

                let mut errands = Vec::with_capacity(ran.len());
                let mut trail = Vec::new();
                for (done, parts, files, read) in ran {
                    errands.push(done);
                    trail.extend(parts);
                    reading.ingested |= read;
                    for file in files {
                        if !attached.iter().any(|held| held.digest == file.digest) {
                            attached.push(file);
                        }
                    }
                }
                let place = places.lock().await;
                if place.used {
                    reading.allowed = None;
                    reading.url = place.url.clone();
                }
                drop(place);
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

    #[allow(clippy::too_many_arguments)]
    async fn run_errand(
        &self,
        from: &Sender<'_>,
        offered: &[ToolSpec],
        places: &Arc<tokio::sync::Mutex<Place>>,
        sent: &Reading,
        call: &ToolCall,
        index: usize,
        brief: &str,
    ) -> (Errand, Vec<Part>, Vec<Attachment>, bool) {
        let card = from.card;
        let rounds = errand::ROUNDS.min(from.limits.max_tool_rounds);
        let mut messages = vec![
            ChatMessage::system(errand::instructions(&card.name)),
            ChatMessage::user(errand::brief(brief)),
        ];

        let mut reading = sent.for_errand();
        let mut addressed = HashSet::new();
        let mut attached: Vec<Attachment> = Vec::new();

        let mut trail: Vec<Part> = Vec::new();
        let mut said = String::new();
        let mut model_calls = 0u32;
        let mut holding: Option<tokio::sync::OwnedMutexGuard<Place>> = None;
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
                Ok(None) => {
                    self.inner.guard.lock().run_within(from.run_id, from.limits).release_step();
                    ending = Ending::Stopped;
                    break;
                }
                Err(err) => {
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
                                Some(mut place) => {
                                    place.used = true;
                                    reading.url = place.url.clone();
                                    holding = Some(place);
                                }
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
        if let Some(mut place) = holding {
            place.url = reading.url.clone();
        }

        let calls = trail.len();
        (
            Errand { brief: brief.to_string(), ending, answer: said, calls, model_calls },
            trail,
            attached,
            reading.ingested,
        )
    }

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
