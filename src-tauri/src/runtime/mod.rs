//! The agent runtime.
//!
//! Every agent is a task with an unbounded inbox. Sending is enqueue-and-return,
//! so an agent that fires messages at four peers is not blocked on any of them,
//! and four peers think concurrently. That is the whole reason this lives in
//! Rust rather than in the webview.
//!
//! Locks here are `parking_lot` and every critical section is short and
//! synchronous. Nothing holds a lock across an `.await`; the guard registry in
//! particular is locked, consulted, and released before any inference starts.

pub mod events;
pub mod guard;
pub mod prompt;

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU16, AtomicUsize, Ordering};
use std::sync::Arc;

use parking_lot::{Mutex, RwLock};
use tokio::sync::{mpsc, Notify};

use crate::config::{AppConfig, InferenceConfig};

/// Turns the browser driver's JSON into something a model reads well.
///
/// The whole page and every element would be most of a context window, so this
/// is bounded on purpose: enough text to understand the page, and the numbered
/// controls, which are the part that has to be exact.
fn render_page(raw: &str) -> String {
    let Ok(page) = serde_json::from_str::<serde_json::Value>(raw) else {
        return raw.chars().take(4000).collect();
    };

    let mut out = format!(
        "{}\n{}",
        page["title"].as_str().unwrap_or_default(),
        page["url"].as_str().unwrap_or_default()
    );

    let scrolled = page["scroll"].as_i64().unwrap_or(0);
    let height = page["height"].as_i64().unwrap_or(0);
    if height > 0 {
        out.push_str(&format!("\nscrolled {scrolled} of {height} pixels"));
    }

    if let Some(elements) = page["elements"].as_array() {
        out.push_str("\n\nYou can use these, by number:\n");
        for element in elements.iter().take(60) {
            let text = element["text"].as_str().unwrap_or_default();
            // An unlabelled control is usually an icon, and saying so is more
            // use than an empty pair of quotes.
            let label = if text.is_empty() { "(unlabelled)" } else { text };
            out.push_str(&format!(
                "  [{}] {} {label}\n",
                element["id"].as_i64().unwrap_or_default(),
                element["tag"].as_str().unwrap_or("?")
            ));
        }
        if elements.len() > 60 {
            out.push_str(&format!("  … and {} more\n", elements.len() - 60));
        }
    }

    let text = page["text"].as_str().unwrap_or_default().trim();
    if !text.is_empty() {
        out.push_str("\nWhat the page says:\n");
        out.extend(text.chars().take(4000));
    }
    out
}

/// What one tool call produced: what the model is told, what the transcript
/// records, and a picture when the tool answers with one.
struct ToolResult {
    rendered: String,
    part: Part,
    /// A `data:` URL, present only for a look at the screen. It is the one
    /// answer a model cannot act on as text.
    image: Option<String>,
}
use crate::db::{Store, StoreError};
use crate::domain::agent::{AgentCard, DirectoryEntry, Lifecycle};
use crate::domain::envelope::{
    channel_for, Envelope, NoticeKind, Part, Participant, ToolOutcome, Trust,
};
use crate::domain::ids::{AgentId, MessageId, RunId};
use crate::domain::now_ms;
use crate::llm::openrouter::{ChatMessage, ChatRequest, LlmClient, LlmError, ToolCall};
use crate::llm::tools::{self, Delivery, ToolInvocation};
use crate::workspace::Workspace;
use events::{Activity, EventSink, UiEvent};
use guard::{GuardLimits, GuardRegistry, Refusal, SendRequest, Verdict};
use prompt::{NameTable, ReplyMode};

/// How many times one turn may call tools before it must produce prose.
///
/// Four is enough for `directory` then `send_message` with slack for a retry
/// after a refusal, and low enough that a confused model cannot spend a run's
/// entire budget inside a single turn.
const MAX_TOOL_ROUNDS: usize = 4;

/// How many messages one turn reads at once.
const MAX_BATCH: usize = 12;

/// How much transcript is replayed into a prompt.
const HISTORY_WINDOW: u32 = 40;

#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("no agent with id {0}")]
    UnknownAgent(AgentId),
    #[error("{0} has been deleted")]
    AgentTerminated(String),
}

struct Inbox {
    tx: mpsc::UnboundedSender<Envelope>,
    /// Queue depth, so the sidebar can show a backlog without draining it.
    depth: Arc<AtomicUsize>,
    /// Woken when a paused agent is resumed.
    resume: Arc<Notify>,
}

struct Inner {
    /// Explicit rather than relying on an ambient tokio context: Tauri's setup
    /// hook runs on the main thread outside any runtime, so `tokio::spawn`
    /// would panic there.
    handle: tokio::runtime::Handle,
    store: Store,
    llm: LlmClient,
    config: RwLock<AppConfig>,
    guard: Mutex<GuardRegistry>,
    inboxes: Mutex<HashMap<AgentId, Inbox>>,
    activity: Mutex<HashMap<AgentId, Activity>>,
    /// Outstanding work per run, used to decide when a cascade has settled.
    inflight: Mutex<HashMap<RunId, usize>>,
    /// Per-agent notes on disk.
    workspace: Workspace,
    /// Loopback port of the computer viewer. Zero until it is listening.
    viewer_port: AtomicU16,
    /// Actor tasks currently running. Registration and the task are separate
    /// things, and a leaked task is invisible without counting it.
    live_actors: Arc<AtomicUsize>,
    events: Arc<dyn EventSink>,
}

#[derive(Clone)]
pub struct Runtime {
    inner: Arc<Inner>,
}

impl Runtime {
    /// Uses the ambient tokio runtime. Convenient inside `#[tokio::test]`.
    pub fn new(
        store: Store,
        llm: LlmClient,
        config: AppConfig,
        workspace: Workspace,
        events: Arc<dyn EventSink>,
    ) -> Self {
        Self::with_handle(tokio::runtime::Handle::current(), store, llm, config, workspace, events)
    }

    pub fn with_handle(
        handle: tokio::runtime::Handle,
        store: Store,
        llm: LlmClient,
        config: AppConfig,
        workspace: Workspace,
        events: Arc<dyn EventSink>,
    ) -> Self {
        let limits = config.limits;
        Self {
            inner: Arc::new(Inner {
                handle,
                store,
                llm,
                config: RwLock::new(config),
                guard: Mutex::new(GuardRegistry::new(limits)),
                inboxes: Mutex::new(HashMap::new()),
                activity: Mutex::new(HashMap::new()),
                inflight: Mutex::new(HashMap::new()),
                workspace,
                viewer_port: AtomicU16::new(0),
                live_actors: Arc::new(AtomicUsize::new(0)),
                events,
            }),
        }
    }

    /// The loopback port the computer viewer is listening on, once it is up.
    ///
    /// Stored here because the UI has to build viewer URLs and the runtime is
    /// what the commands already hold.
    pub fn set_viewer_port(&self, port: u16) {
        self.inner.viewer_port.store(port, Ordering::SeqCst);
    }

    pub fn viewer_port(&self) -> u16 {
        self.inner.viewer_port.load(Ordering::SeqCst)
    }

    pub fn store(&self) -> &Store {
        &self.inner.store
    }

    pub fn workspace(&self) -> &Workspace {
        &self.inner.workspace
    }

    pub fn config(&self) -> AppConfig {
        self.inner.config.read().clone()
    }

    pub fn set_config(&self, config: AppConfig) {
        self.inner.guard.lock().set_limits(config.limits);
        *self.inner.config.write() = config;
    }

    pub fn limits(&self) -> GuardLimits {
        self.inner.guard.lock().default_limits()
    }

    // ---- lifecycle -------------------------------------------------------

    /// Brings every non-terminated agent online. Called once at startup.
    pub fn start_all(&self) -> Result<usize, RuntimeError> {
        let agents = self.inner.store.list_agents()?;
        let mut started = 0;
        for card in agents.iter().filter(|c| c.lifecycle != Lifecycle::Terminated) {
            self.start_agent(card.id);
            started += 1;
        }
        Ok(started)
    }

    /// Spawns the actor for one agent. Idempotent.
    pub fn start_agent(&self, id: AgentId) {
        let mut inboxes = self.inner.inboxes.lock();
        if inboxes.contains_key(&id) {
            return;
        }

        let (tx, rx) = mpsc::unbounded_channel();
        let depth = Arc::new(AtomicUsize::new(0));
        let resume = Arc::new(Notify::new());
        inboxes.insert(id, Inbox { tx, depth: depth.clone(), resume: resume.clone() });
        drop(inboxes);

        let runtime = self.clone();
        let live = self.inner.live_actors.clone();
        live.fetch_add(1, Ordering::SeqCst);
        self.inner.handle.spawn(async move {
            actor_loop(runtime, id, rx, depth, resume).await;
            live.fetch_sub(1, Ordering::SeqCst);
        });

        self.set_activity(id, Activity::Idle);
    }

    /// Drops the inbox so the actor task finishes once it has drained.
    ///
    /// Anything still queued is discarded, which is the correct reading of
    /// "delete this agent": undelivered mail to a deleted mailbox has nowhere
    /// to go.
    pub fn stop_agent(&self, id: AgentId) {
        let inbox = { self.inner.inboxes.lock().remove(&id) };
        if let Some(inbox) = inbox {
            // Wake a parked actor so it can notice it has been deleted and
            // exit. Dropping the inbox alone only releases an actor blocked on
            // `recv`; one paused mid-message is waiting on this notifier, and
            // this is the last handle to it.
            inbox.resume.notify_waiters();
        }
        self.inner.activity.lock().remove(&id);
    }

    pub fn resume_agent(&self, id: AgentId) {
        let resume = { self.inner.inboxes.lock().get(&id).map(|inbox| inbox.resume.clone()) };
        if let Some(resume) = resume {
            resume.notify_waiters();
        }
        self.set_activity(id, Activity::Idle);
    }

    /// Marks an agent paused. The actor parks at its next message boundary and
    /// everything sent meanwhile queues rather than being dropped.
    pub fn pause_agent(&self, id: AgentId) {
        self.set_activity(id, Activity::Paused);
    }

    pub fn emit(&self, event: UiEvent) {
        self.inner.events.emit(event);
    }

    /// One cheap round trip to tell a bad key from a bad URL from a bad model.
    ///
    /// Without this, every misconfiguration presents identically as an agent
    /// that says nothing.
    pub async fn probe(&self, config: &AppConfig) -> Result<String, LlmError> {
        let request = ChatRequest {
            model: config.inference.default_model.clone(),
            messages: vec![
                ChatMessage::system("Reply with the single word: ok"),
                ChatMessage::user("ping"),
            ],
            tools: Vec::new(),
            temperature: Some(0.0),
        };
        let completion = self.inner.llm.stream_chat(&config.inference, &request, |_| {}).await?;
        Ok(format!(
            "Connected to {} using {}. Model replied: {}",
            config.inference.base_url,
            config.inference.default_model,
            completion.content.trim().chars().take(80).collect::<String>()
        ))
    }

    /// Messages queued for an agent that it has not yet picked up.
    ///
    /// Observable because "persisted" and "queued" are two different moments:
    /// `deliver` writes to the store before it touches the inbox, so anything
    /// waiting on delivery has to watch the inbox, not the transcript.
    pub fn inbox_depth(&self, id: AgentId) -> usize {
        self.inner
            .inboxes
            .lock()
            .get(&id)
            .map(|inbox| inbox.depth.load(Ordering::SeqCst))
            .unwrap_or(0)
    }

    /// Number of agent actor tasks currently running.
    pub fn live_actors(&self) -> usize {
        self.inner.live_actors.load(Ordering::SeqCst)
    }

    pub fn activity_snapshot(&self) -> HashMap<AgentId, Activity> {
        self.inner.activity.lock().clone()
    }

    fn set_activity(&self, id: AgentId, activity: Activity) {
        let changed = {
            let mut map = self.inner.activity.lock();
            if map.get(&id) == Some(&activity) {
                false
            } else {
                map.insert(id, activity);
                true
            }
        };
        if changed {
            self.inner.events.emit(UiEvent::ActivityChanged { agent_id: id, activity });
        }
    }

    // ---- delivery --------------------------------------------------------

    /// Persists an envelope, tells the UI, and queues it if an agent is the
    /// recipient.
    ///
    /// Persisting before enqueueing is deliberate: the operator sees a message
    /// the moment it is sent, even if the recipient is busy for the next
    /// thirty seconds.
    fn deliver(&self, envelope: Envelope) -> Result<(), RuntimeError> {
        self.inner.store.append(&envelope)?;
        self.inner.events.emit(UiEvent::MessageAppended { message: Box::new(envelope.clone()) });

        if let Participant::Agent { id } = envelope.to {
            let queued = {
                let inboxes = self.inner.inboxes.lock();
                match inboxes.get(&id) {
                    Some(inbox) => {
                        let depth = inbox.depth.fetch_add(1, Ordering::SeqCst) + 1;
                        match inbox.tx.send(envelope) {
                            Ok(()) => Some(depth),
                            Err(_) => {
                                inbox.depth.fetch_sub(1, Ordering::SeqCst);
                                None
                            }
                        }
                    }
                    None => None,
                }
            };

            if let Some(depth) = queued {
                // An agent mid-inference keeps its Thinking badge; the queue
                // depth is only interesting when it is not already working.
                let thinking = { self.inner.activity.lock().get(&id) == Some(&Activity::Thinking) };
                if !thinking {
                    self.set_activity(id, Activity::Queued { depth });
                }
            }
        }
        Ok(())
    }

    /// Operator sends a message to one agent. Returns the run it starts.
    pub fn send_from_human(&self, to: AgentId, text: &str) -> Result<RunId, RuntimeError> {
        let card = self.inner.store.get_agent(to)?.ok_or(RuntimeError::UnknownAgent(to))?;
        if card.lifecycle == Lifecycle::Terminated {
            return Err(RuntimeError::AgentTerminated(card.name));
        }

        let run_id = RunId::new();
        let envelope = Envelope {
            id: MessageId::new(),
            run_id,
            channel_id: to,
            from: Participant::Human,
            to: Participant::Agent { id: to },
            parts: vec![Part::text(text.trim())],
            trust: Trust::Operator,
            hop: 0,
            expects_reply: true,
            cause: None,
            created_at: now_ms(),
        };

        self.track_inflight(run_id, 1);
        self.deliver(envelope)?;
        Ok(run_id)
    }

    fn track_inflight(&self, run: RunId, delta: i64) {
        let settled = {
            let mut map = self.inner.inflight.lock();
            let entry = map.entry(run).or_insert(0);
            if delta >= 0 {
                *entry += delta as usize;
            } else {
                *entry = entry.saturating_sub((-delta) as usize);
            }
            if *entry == 0 {
                map.remove(&run);
                true
            } else {
                false
            }
        };

        if settled {
            let steps = self.inner.guard.lock().peek(run).map(|r| r.steps_used()).unwrap_or(0);
            self.inner.events.emit(UiEvent::RunSettled { run_id: run, steps_used: steps });
        }
    }

    fn notice(
        &self,
        agent: AgentId,
        run_id: RunId,
        cause: Option<MessageId>,
        kind: NoticeKind,
        text: String,
    ) {
        let envelope = Envelope {
            id: MessageId::new(),
            run_id,
            channel_id: agent,
            from: Participant::System,
            to: Participant::Agent { id: agent },
            parts: vec![Part::Notice { kind, text }],
            trust: Trust::System,
            hop: 0,
            expects_reply: false,
            cause,
            created_at: now_ms(),
        };
        // A notice is written straight to the transcript rather than delivered,
        // so it never wakes the agent it is about.
        if let Err(err) = self.inner.store.append(&envelope) {
            tracing::error!(%err, "failed to record notice");
            return;
        }
        self.inner.events.emit(UiEvent::MessageAppended { message: Box::new(envelope) });
    }

    // ---- one agent turn --------------------------------------------------

    async fn run_turn(&self, agent_id: AgentId, batch: Vec<Envelope>) {
        let Some(card) = self.inner.store.get_agent(agent_id).ok().flatten() else {
            return;
        };
        if card.lifecycle == Lifecycle::Terminated {
            return;
        }

        let run_id = batch[0].run_id;
        let inbound_hop = batch.iter().map(|e| e.hop).max().unwrap_or(0);
        let cause = batch.last().map(|e| e.id);

        // The most recent envelope that wants an answer decides where the
        // reply goes. Everything else in the batch is context.
        let reply_target = batch.iter().rev().find(|e| e.expects_reply).map(|e| e.from);
        let mode = match reply_target {
            Some(Participant::Human) => ReplyMode::ToOperator,
            Some(Participant::Agent { .. }) => ReplyMode::ToPeer,
            _ => ReplyMode::NoteOnly,
        };

        // Peek rather than claim: the budget is spent per model call inside the
        // loop below, but there is no point building a prompt or telling the UI
        // a message is coming if the run is already finished.
        let has_budget = { self.inner.guard.lock().run(run_id).has_budget() };
        if !has_budget {
            let limits = self.limits();
            self.notice(
                agent_id,
                run_id,
                cause,
                NoticeKind::GuardStop,
                format!(
                    "{} did not run: this conversation already used its budget of {} model calls. \
                     Raise it in Settings if the work is genuinely this large.",
                    card.name, limits.max_steps_per_run
                ),
            );
            self.finish_turn(agent_id, run_id, batch.len());
            return;
        }

        self.set_activity(agent_id, Activity::Thinking);

        let roster = self.roster_excluding(agent_id);
        let names = self.name_table();
        let history = self
            .inner
            .store
            .channel_messages(agent_id, HISTORY_WINDOW)
            .unwrap_or_default()
            .into_iter()
            // The batch is rendered separately; including it twice would make
            // the model answer itself.
            .filter(|e| !batch.iter().any(|b| b.id == e.id))
            .collect::<Vec<_>>();

        let notes = self.inner.workspace.read(agent_id);
        let mut messages =
            prompt::build_messages(&card, &roster, &names, &notes, &history, &batch, mode);

        // Where the finished message will land, and who it is for. Both are
        // known before the first token, so the UI never has to guess and then
        // correct itself.
        let (out_channel, stream_to) = match (mode, reply_target) {
            (ReplyMode::ToPeer, Some(Participant::Agent { id })) => (id, Participant::Agent { id }),
            _ => (agent_id, Participant::Human),
        };

        let stream_id = MessageId::new();
        self.inner.events.emit(UiEvent::StreamStarted {
            message_id: stream_id,
            channel_id: out_channel,
            agent_id,
            run_id,
            to: stream_to,
        });

        let config = self.config();
        let mut collected_text = String::new();
        // Settings resolve agent over group over app. An agent that names its own
        // model keeps it; otherwise the group's choice applies; otherwise the
        // app default. The endpoint resolves the same way, so one crew can run
        // against a local server while another uses a hosted one.
        let inference = self.inference_for(&card, &config);
        let model = if card.model.trim().is_empty() {
            inference.default_model.clone()
        } else {
            card.model.clone()
        };

        let mut tool_parts: Vec<Part> = Vec::new();
        // Peers written to through `send_message` during this turn.
        let mut addressed: HashSet<AgentId> = HashSet::new();
        let mut failure: Option<LlmError> = None;
        let mut hit_tool_ceiling = false;
        let mut budget_exhausted = false;

        for round in 0..MAX_TOOL_ROUNDS {
            // One claim per model call. Claiming per turn instead would let a
            // tool-looping turn bill MAX_TOOL_ROUNDS times against one unit of
            // budget, which is how a bounded run still runs up a bill.
            let reserved = { self.inner.guard.lock().run(run_id).reserve_step() };
            if !reserved {
                budget_exhausted = true;
                break;
            }

            let request = ChatRequest {
                model: model.clone(),
                messages: messages.clone(),
                tools: tools::specs(),
                temperature: None,
            };

            let events = self.inner.events.clone();
            let completion = self
                .inner
                .llm
                .stream_chat(&inference, &request, |token| {
                    events.emit(UiEvent::StreamDelta {
                        message_id: stream_id,
                        channel_id: out_channel,
                        text: token.to_string(),
                    });
                })
                .await;

            let completion = match completion {
                Ok(completion) => completion,
                Err(err) => {
                    failure = Some(err);
                    break;
                }
            };

            if !completion.content.is_empty() {
                if !collected_text.is_empty() {
                    collected_text.push_str("\n\n");
                }
                collected_text.push_str(&completion.content);
            }

            if completion.tool_calls.is_empty() {
                break;
            }

            messages.push(ChatMessage::Assistant {
                content: (!completion.content.is_empty()).then(|| completion.content.clone()),
                tool_calls: completion.to_wire_tool_calls(),
            });

            for call in &completion.tool_calls {
                let outcome = self
                    .execute_tool(
                        &card,
                        run_id,
                        inbound_hop,
                        cause,
                        reply_target,
                        &mut addressed,
                        call,
                    )
                    .await;
                tool_parts.push(outcome.part);
                messages.push(ChatMessage::Tool {
                    tool_call_id: call.id.clone(),
                    content: outcome.rendered,
                });

                // A picture cannot travel inside a tool result, which is text,
                // so it follows as a turn of its own. This is the whole reason
                // an agent can work a screen rather than only describe one.
                if let Some(image) = outcome.image {
                    messages.push(ChatMessage::user_seeing(
                        "This is what your screen looks like now.",
                        image,
                    ));
                }
            }

            if round == MAX_TOOL_ROUNDS - 1 {
                hit_tool_ceiling = true;
            }
        }

        self.inner
            .events
            .emit(UiEvent::StreamEnded { message_id: stream_id, channel_id: out_channel });

        if hit_tool_ceiling {
            tool_parts.push(Part::Notice {
                kind: NoticeKind::GuardStop,
                text: format!(
                    "{} reached the limit of {MAX_TOOL_ROUNDS} tool calls in one turn.",
                    card.name
                ),
            });
        }
        if budget_exhausted {
            let limits = self.limits();
            tool_parts.push(Part::Notice {
                kind: NoticeKind::GuardStop,
                text: format!(
                    "This conversation hit its budget of {} model calls, so {} stopped early.",
                    limits.max_steps_per_run, card.name
                ),
            });
        }

        if let Some(err) = failure {
            tracing::warn!(agent = %card.name, error = %err, "inference failed");
            self.notice(
                agent_id,
                run_id,
                cause,
                NoticeKind::UpstreamError,
                format!("{} could not reply: {}", card.name, err),
            );
        } else {
            self.emit_reply(
                &card,
                run_id,
                inbound_hop,
                cause,
                mode,
                reply_target,
                &addressed,
                collected_text,
                tool_parts,
            );
        }

        self.finish_turn(agent_id, run_id, batch.len());
    }

    fn finish_turn(&self, agent_id: AgentId, run_id: RunId, consumed: usize) {
        let depth = {
            let inboxes = self.inner.inboxes.lock();
            inboxes.get(&agent_id).map(|i| i.depth.load(Ordering::SeqCst)).unwrap_or(0)
        };
        self.set_activity(
            agent_id,
            if depth == 0 { Activity::Idle } else { Activity::Queued { depth } },
        );
        self.track_inflight(run_id, -(consumed as i64));
    }

    #[allow(clippy::too_many_arguments)]
    fn emit_reply(
        &self,
        card: &AgentCard,
        run_id: RunId,
        inbound_hop: u16,
        cause: Option<MessageId>,
        mode: ReplyMode,
        reply_target: Option<Participant>,
        addressed: &HashSet<AgentId>,
        text: String,
        mut tool_parts: Vec<Part>,
    ) {
        let text = text.trim().to_string();
        let me = Participant::Agent { id: card.id };
        let mut hop = inbound_hop;
        let mut to = Participant::Human;

        // An agent that already answered this peer with `send_message` has said
        // its piece. The text it trails afterwards is commentary on its own
        // turn, and delivering that as a second message is how one turn put two
        // near-identical messages in the peer's channel. It goes to the
        // operator instead, where it is still readable.
        let already_answered = matches!(
            reply_target,
            Some(Participant::Agent { id }) if addressed.contains(&id)
        );

        if mode == ReplyMode::ToPeer && !already_answered {
            if let Some(Participant::Agent { id: peer }) = reply_target {
                // An automatic reply still travels a hop and still counts
                // against the pair budget, otherwise two agents could bounce
                // replies forever without ever calling send_message.
                let peer_name = self
                    .inner
                    .store
                    .get_agent(peer)
                    .ok()
                    .flatten()
                    .map(|c| c.name)
                    .unwrap_or_else(|| "that agent".to_string());

                let verdict = {
                    self.inner.guard.lock().run(run_id).evaluate(&SendRequest {
                        from: card.id,
                        to: peer,
                        to_name: peer_name.clone(),
                        text: text.clone(),
                        inbound_hop,
                    })
                };

                match verdict {
                    Verdict::Allow { hop: next } => {
                        to = Participant::Agent { id: peer };
                        hop = next;
                    }
                    Verdict::Refuse(refusal) => {
                        // Downgrade to a note rather than dropping the answer.
                        tool_parts.push(Part::Notice {
                            kind: NoticeKind::GuardStop,
                            text: format!(
                                "Reply to {peer_name} was not delivered: {}.",
                                refusal.headline()
                            ),
                        });
                    }
                }
            }
        }

        // The record of what this agent did belongs in this agent's own
        // channel, always. Attaching it to the reply meant that a reply to a
        // peer carried the sender's private working notes into the recipient's
        // transcript, so opening one agent's channel showed you every other
        // agent's tool calls.
        if !tool_parts.is_empty() {
            let record = Envelope {
                id: MessageId::new(),
                run_id,
                channel_id: card.id,
                from: me,
                to: Participant::System,
                parts: tool_parts,
                trust: Trust::System,
                hop: inbound_hop,
                expects_reply: false,
                cause,
                created_at: now_ms(),
            };
            if let Err(err) = self.deliver(record) {
                tracing::error!(%err, "failed to record agent activity");
            }
        }

        if text.is_empty() {
            return;
        }

        let Some(channel_id) = channel_for(me, to) else {
            return;
        };

        let envelope = Envelope {
            id: MessageId::new(),
            run_id,
            channel_id,
            from: me,
            to,
            parts: vec![Part::text(text)],
            trust: Trust::Peer,
            hop,
            // An agent's answer never itself demands an answer. This is the
            // single asymmetry that makes cascades terminate.
            expects_reply: false,
            cause,
            created_at: now_ms(),
        };

        if to.is_agent() {
            self.track_inflight(run_id, 1);
        }
        if let Err(err) = self.deliver(envelope) {
            tracing::error!(%err, "failed to deliver reply");
        }
    }

    // ---- tools -----------------------------------------------------------

    #[allow(clippy::too_many_arguments)]
    async fn execute_tool(
        &self,
        card: &AgentCard,
        run_id: RunId,
        inbound_hop: u16,
        cause: Option<MessageId>,
        // Who this turn is answering, so a send aimed at them is recognised as
        // a reply rather than a fresh approach.
        reply_target: Option<Participant>,
        // Peers this turn has already written to. See `emit_reply`.
        addressed: &mut HashSet<AgentId>,
        call: &ToolCall,
    ) -> ToolResult {
        let arguments = call.parsed_arguments().unwrap_or(serde_json::Value::Null);

        let (rendered, part, image) = self
            .dispatch_tool(
                card,
                run_id,
                inbound_hop,
                cause,
                reply_target,
                addressed,
                call,
                arguments,
            )
            .await;
        ToolResult { rendered, part, image }
    }

    /// The body of `execute_tool`, kept separate so every arm can go on
    /// returning a pair while only the screen arm produces a picture.
    #[allow(clippy::too_many_arguments)]
    async fn dispatch_tool(
        &self,
        card: &AgentCard,
        run_id: RunId,
        inbound_hop: u16,
        cause: Option<MessageId>,
        reply_target: Option<Participant>,
        addressed: &mut HashSet<AgentId>,
        call: &ToolCall,
        arguments: serde_json::Value,
    ) -> (String, Part, Option<String>) {
        let invocation = match tools::parse(call) {
            Ok(invocation) => invocation,
            Err(err) => {
                return (
                    err.guidance(),
                    Part::ToolCall {
                        name: call.name.clone(),
                        arguments,
                        outcome: ToolOutcome::Failed { error: err.to_string() },
                    },
                    None,
                );
            }
        };

        if let ToolInvocation::UseScreen { action } = invocation {
            return self.use_screen(card, action, arguments).await;
        }

        let (rendered, part) = match invocation {
            // Handled above, where it can answer with a picture.
            ToolInvocation::UseScreen { .. } => unreachable!("taken by the branch above"),
            ToolInvocation::Directory => {
                let roster = self.roster_excluding(card.id);
                let payload =
                    serde_json::to_string_pretty(&roster).unwrap_or_else(|_| "[]".to_string());
                let summary = if roster.is_empty() {
                    "No other agents exist.".to_string()
                } else {
                    format!(
                        "{} agent(s): {}",
                        roster.len(),
                        roster.iter().map(|e| e.name.as_str()).collect::<Vec<_>>().join(", ")
                    )
                };
                (
                    payload,
                    Part::ToolCall {
                        name: tools::DIRECTORY.to_string(),
                        arguments,
                        outcome: ToolOutcome::Ok { summary },
                    },
                )
            }

            ToolInvocation::UpdateNotes { content } => {
                match self.inner.workspace.write(card.id, &card.name, &content) {
                    Ok(stored) => {
                        let summary = if stored.truncated {
                            format!(
                                "Notes saved, but they were too long and the end was cut. {} \
                                 characters kept. Rewrite them shorter, keeping only what will \
                                 still matter next week.",
                                stored.characters
                            )
                        } else if stored.characters == 0 {
                            "Notes cleared.".to_string()
                        } else {
                            format!("Notes saved ({} characters).", stored.characters)
                        };
                        (
                            summary.clone(),
                            Part::ToolCall {
                                name: tools::UPDATE_NOTES.to_string(),
                                arguments,
                                outcome: ToolOutcome::Ok { summary },
                            },
                        )
                    }
                    Err(err) => (
                        format!("Error: your notes could not be saved ({err})."),
                        Part::ToolCall {
                            name: tools::UPDATE_NOTES.to_string(),
                            arguments,
                            outcome: ToolOutcome::Failed { error: err.to_string() },
                        },
                    ),
                }
            }

            ToolInvocation::RunCommand { command } => {
                let outcome = match self.ensure_computer(card).await {
                    Ok((client, sandbox)) => {
                        client.run(&sandbox.id, &sandbox.envd_token, &command).await
                    }
                    Err(err) => Err(err),
                };
                let (rendered, outcome) = match outcome {
                    Ok(output) => {
                        let summary = format!(
                            "exit {}, {} bytes out",
                            output.exit_code,
                            output.stdout.len() + output.stderr.len()
                        );
                        (output.rendered(), ToolOutcome::Ok { summary })
                    }
                    // Reported to the model rather than raised: a machine that
                    // will not start is something the agent has to work around
                    // and tell the operator about, not a dead turn.
                    Err(err) => (
                        format!("Error: your computer is not available ({err})."),
                        ToolOutcome::Failed { error: err.to_string() },
                    ),
                };
                (
                    rendered,
                    Part::ToolCall { name: tools::RUN_COMMAND.to_string(), arguments, outcome },
                )
            }

            ToolInvocation::Browse { action, args } => {
                let outcome = match self.ensure_computer(card).await {
                    Ok((client, sandbox)) => {
                        client.browse(&sandbox.id, &sandbox.envd_token, &action, &args).await
                    }
                    Err(err) => Err(err),
                };
                let (rendered, outcome) = match outcome {
                    Ok(page) => {
                        let summary = format!("{action} in the browser");
                        (render_page(&page), ToolOutcome::Ok { summary })
                    }
                    Err(err) => {
                        (format!("Error: {err}"), ToolOutcome::Failed { error: err.to_string() })
                    }
                };
                (rendered, Part::ToolCall { name: tools::BROWSE.to_string(), arguments, outcome })
            }

            ToolInvocation::OpenOnDesktop { command } => {
                let outcome = match self.ensure_computer(card).await {
                    Ok((client, sandbox)) => {
                        client.open_on_desktop(&sandbox.id, &sandbox.envd_token, &command).await
                    }
                    Err(err) => Err(err),
                };
                let (rendered, outcome) = match outcome {
                    Ok(_) => (
                        format!(
                            "Opened `{command}` on your screen. The operator can see it. Use \
                             run_command if you need to read anything back from the machine."
                        ),
                        ToolOutcome::Ok { summary: format!("opened {command}") },
                    ),
                    Err(err) => (
                        format!("Error: could not open that on your screen ({err})."),
                        ToolOutcome::Failed { error: err.to_string() },
                    ),
                };
                (
                    rendered,
                    Part::ToolCall { name: tools::OPEN_ON_DESKTOP.to_string(), arguments, outcome },
                )
            }

            ToolInvocation::SendMessage { to, text } => {
                let deliveries = self.send_to_peers(
                    card,
                    run_id,
                    inbound_hop,
                    cause,
                    reply_target,
                    addressed,
                    &to,
                    &text,
                );
                let rendered = tools::render_deliveries(&deliveries);
                let queued =
                    deliveries.iter().filter(|d| matches!(d, Delivery::Queued { .. })).count();
                let outcome = if queued > 0 {
                    ToolOutcome::Ok { summary: format!("queued for {queued} agent(s)") }
                } else {
                    ToolOutcome::Refused {
                        reason: deliveries
                            .iter()
                            .filter_map(|d| match d {
                                Delivery::Refused { reason, .. } => Some(reason.as_str()),
                                _ => None,
                            })
                            .next()
                            .unwrap_or("no recipients")
                            .to_string(),
                    }
                };
                (
                    rendered,
                    Part::ToolCall { name: tools::SEND_MESSAGE.to_string(), arguments, outcome },
                )
            }
        };

        (rendered, part, None)
    }

    /// Looking at, and acting on, the screen.
    ///
    /// Split out because it is the only tool that answers with a picture: a
    /// model cannot act on a screen described to it in prose, so a look comes
    /// back as an image in the conversation rather than as text.
    async fn use_screen(
        &self,
        card: &AgentCard,
        action: tools::ScreenAction,
        arguments: serde_json::Value,
    ) -> (String, Part, Option<String>) {
        let failed = |message: String, err: String, arguments: serde_json::Value| {
            (
                message,
                Part::ToolCall {
                    name: tools::USE_SCREEN.to_string(),
                    arguments,
                    outcome: ToolOutcome::Failed { error: err },
                },
                None,
            )
        };

        let (client, sandbox) = match self.ensure_computer(card).await {
            Ok(pair) => pair,
            Err(err) => {
                return failed(
                    format!("Error: your screen is not available ({err})."),
                    err.to_string(),
                    arguments,
                )
            }
        };

        if matches!(action, tools::ScreenAction::Look) {
            return match client.screenshot(&sandbox.id, &sandbox.envd_token).await {
                Ok((image, geometry)) => (
                    format!(
                        "Here is your screen, {geometry} pixels. Coordinates are measured from \
                         the top left of this picture."
                    ),
                    Part::ToolCall {
                        name: tools::USE_SCREEN.to_string(),
                        arguments,
                        outcome: ToolOutcome::Ok {
                            summary: format!("looked at the screen ({geometry})"),
                        },
                    },
                    Some(image),
                ),
                Err(err) => failed(
                    format!("Error: could not see the screen ({err})."),
                    err.to_string(),
                    arguments,
                ),
            };
        }

        let (desktop, described) = match &action {
            tools::ScreenAction::Look => unreachable!("handled above"),
            tools::ScreenAction::Click { x, y, button, count } => (
                crate::e2b::DesktopAction::Click { x: *x, y: *y, button: *button, count: *count },
                format!("clicked at {x}, {y}"),
            ),
            tools::ScreenAction::Move { x, y } => (
                crate::e2b::DesktopAction::Move { x: *x, y: *y },
                format!("moved the pointer to {x}, {y}"),
            ),
            tools::ScreenAction::Type { text } => (
                crate::e2b::DesktopAction::Type { text: text.clone() },
                format!("typed {} characters", text.chars().count()),
            ),
            tools::ScreenAction::Key { keys } => {
                (crate::e2b::DesktopAction::Key { keys: keys.clone() }, format!("pressed {keys}"))
            }
            tools::ScreenAction::Scroll { down, amount } => (
                crate::e2b::DesktopAction::Scroll { down: *down, amount: *amount },
                format!("scrolled {} {amount}", if *down { "down" } else { "up" }),
            ),
        };

        match client.act_on_desktop(&sandbox.id, &sandbox.envd_token, &desktop).await {
            Ok(_) => (
                format!("{described}. Look again to see what changed."),
                Part::ToolCall {
                    name: tools::USE_SCREEN.to_string(),
                    arguments,
                    outcome: ToolOutcome::Ok { summary: described },
                },
                None,
            ),
            Err(err) => failed(
                format!("Error: that did not reach the screen ({err})."),
                err.to_string(),
                arguments,
            ),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn send_to_peers(
        &self,
        card: &AgentCard,
        run_id: RunId,
        inbound_hop: u16,
        cause: Option<MessageId>,
        reply_target: Option<Participant>,
        addressed: &mut HashSet<AgentId>,
        recipients: &[String],
        text: &str,
    ) -> Vec<Delivery> {
        // Fan-out width is checked before any recipient, so a blast at the
        // whole roster is refused as one thing rather than partly delivered.
        let too_wide = { self.inner.guard.lock().run(run_id).check_fanout(recipients.len()) };
        if let Some(refusal) = too_wide {
            return recipients
                .iter()
                .map(|name| Delivery::Refused { to: name.clone(), reason: refusal.explain() })
                .collect();
        }

        let directory = self.inner.store.list_agents().unwrap_or_default();
        let mut out = Vec::new();

        for name in recipients {
            let trimmed = name.trim();
            // Scoped to the sender's group, exactly like `roster_excluding`. A
            // name belonging to an agent in another group must not resolve, and
            // must not be distinguishable from a name belonging to nobody:
            // confirming that the agent exists would leak the roster across the
            // boundary the group is there to draw.
            let found = directory
                .iter()
                .find(|c| c.group_id == card.group_id && c.name.eq_ignore_ascii_case(trimmed));

            let target = match found {
                None => {
                    if directory.iter().any(|c| c.name.eq_ignore_ascii_case(trimmed)) {
                        // The operator gets to see what the model may not.
                        tracing::debug!(
                            from = %card.name,
                            recipient = trimmed,
                            "refused a send addressed outside the sender's group"
                        );
                    }
                    out.push(Delivery::Refused {
                        to: name.clone(),
                        reason: Refusal::UnknownRecipient { recipient: name.clone() }.explain(),
                    });
                    continue;
                }
                Some(recipient) if recipient.lifecycle == Lifecycle::Terminated => {
                    out.push(Delivery::Refused {
                        to: name.clone(),
                        reason: Refusal::RecipientTerminated { recipient: recipient.name.clone() }
                            .explain(),
                    });
                    continue;
                }
                Some(recipient) => recipient,
            };

            let verdict = {
                self.inner.guard.lock().run(run_id).evaluate(&SendRequest {
                    from: card.id,
                    to: target.id,
                    to_name: target.name.clone(),
                    text: text.to_string(),
                    inbound_hop,
                })
            };

            match verdict {
                Verdict::Refuse(refusal) => {
                    out.push(Delivery::Refused {
                        to: target.name.clone(),
                        reason: refusal.explain(),
                    });
                }
                Verdict::Allow { hop } => {
                    let from = Participant::Agent { id: card.id };
                    let to = Participant::Agent { id: target.id };
                    let Some(channel_id) = channel_for(from, to) else {
                        continue;
                    };

                    // An agent answering its correspondent through this tool is
                    // still answering, so the message must not demand an answer
                    // back. Marking it as a fresh approach re-arms the cascade
                    // that `emit_reply`'s asymmetry exists to end: the peer
                    // replies, this agent replies to that, and the exchange only
                    // stops when the guard's dedup or hop limit fires. Two
                    // agents introducing themselves reached hop 7 of 8 that way.
                    let answering = matches!(
                        reply_target,
                        Some(Participant::Agent { id }) if id == target.id
                    );

                    let envelope = Envelope {
                        id: MessageId::new(),
                        run_id,
                        channel_id,
                        from,
                        to,
                        parts: vec![Part::text(text)],
                        trust: Trust::Peer,
                        hop,
                        expects_reply: !answering,
                        cause,
                        created_at: now_ms(),
                    };

                    self.track_inflight(run_id, 1);
                    match self.deliver(envelope) {
                        Ok(()) => {
                            addressed.insert(target.id);
                            out.push(Delivery::Queued { to: target.name.clone() })
                        }
                        Err(err) => {
                            self.track_inflight(run_id, -1);
                            out.push(Delivery::Refused {
                                to: target.name.clone(),
                                reason: format!("Refused: delivery failed ({err})."),
                            });
                        }
                    }
                }
            }
        }

        out
    }

    // ---- lookups ---------------------------------------------------------

    /// The peers one agent can see: its own group, discoverable, not itself.
    ///
    /// The group filter is the isolation boundary, not a display convenience.
    /// `send_to_peers` resolves names against exactly the same scope, so an
    /// agent cannot address a peer it was never shown. An agent whose own card
    /// has gone sees nobody, which fails closed.
    /// The agent's computer, made or replaced if there is not a live one.
    ///
    /// The single place a sandbox is provisioned, so the agent's tool, the
    /// operator's terminal and the desktop button cannot disagree about which
    /// machine an agent has. Lazy on purpose: there is nothing to switch on,
    /// and an agent that never runs a command never costs a sandbox.
    pub async fn ensure_computer(
        &self,
        card: &AgentCard,
    ) -> Result<(crate::e2b::E2bClient, crate::e2b::Sandbox), crate::e2b::E2bError> {
        let config = self.config();
        let client =
            crate::e2b::E2bClient::new(&config.e2b.api_key).ok_or(crate::e2b::E2bError::NoKey)?;

        // A sandbox recorded without its tokens predates them and cannot be
        // reached, so it counts as absent rather than as something to retry.
        let existing = match (&card.sandbox_id, &card.sandbox_envd_token) {
            (Some(id), Some(envd)) if client.is_alive(id).await.unwrap_or(false) => {
                Some(crate::e2b::Sandbox {
                    id: id.clone(),
                    envd_token: envd.clone(),
                    traffic_token: card.sandbox_traffic_token.clone().unwrap_or_default(),
                })
            }
            _ => None,
        };

        if let Some(sandbox) = existing {
            return Ok((client, sandbox));
        }

        let fresh = client.create(&card.name).await?;

        // A sandbox that cannot be written down is a sandbox nobody can reach
        // and nobody will stop paying for, so it is killed rather than left.
        // Failing to read the create reply once already orphaned three of them.
        if let Err(err) = self
            .inner
            .store
            .set_agent_sandbox(card.id, Some((&fresh.id, &fresh.envd_token, &fresh.traffic_token)))
        {
            tracing::error!(%err, sandbox = %fresh.id, "could not record a sandbox; killing it");
            let _ = client.kill(&fresh.id).await;
            return Err(crate::e2b::E2bError::Protocol(format!(
                "the sandbox could not be recorded and was released ({err})"
            )));
        }

        self.inner.events.emit(UiEvent::AgentsChanged);
        Ok((client, fresh))
    }

    /// Kills every sandbox this app made that no agent still refers to.
    ///
    /// A crash between creating a sandbox and recording it, or an agent deleted
    /// while its machine was up, leaves something running that nothing in the
    /// app can see. Only sandboxes labelled by Guac are touched.
    pub async fn sweep_computers(&self) -> Result<usize, crate::e2b::E2bError> {
        let config = self.config();
        let Some(client) = crate::e2b::E2bClient::new(&config.e2b.api_key) else {
            return Ok(0);
        };

        let known: std::collections::HashSet<String> = self
            .inner
            .store
            .list_agents()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|c| c.sandbox_id)
            .collect();

        let mut swept = 0;
        for sandbox in client.list_ours().await? {
            if known.contains(&sandbox) {
                continue;
            }
            tracing::info!(%sandbox, "releasing a sandbox no agent refers to");
            if client.kill(&sandbox).await.is_ok() {
                swept += 1;
            }
        }
        Ok(swept)
    }

    /// The inference settings one agent's turn should use.
    ///
    /// Layered rather than replaced: a group that overrides only the model
    /// still uses the app's endpoint and key, so setting one field does not
    /// silently blank the others.
    fn inference_for(&self, card: &AgentCard, config: &AppConfig) -> InferenceConfig {
        match self.inner.store.group_inference(card.group_id) {
            Ok(overrides) => overrides.apply(&config.inference),
            Err(err) => {
                // A group that cannot be read must not take its agents offline;
                // the app defaults are a working fallback.
                tracing::warn!(agent = %card.name, %err, "group settings unreadable, using app defaults");
                config.inference.clone()
            }
        }
    }

    fn roster_excluding(&self, me: AgentId) -> Vec<DirectoryEntry> {
        let agents = self.inner.store.list_agents().unwrap_or_default();
        let Some(group) = agents.iter().find(|c| c.id == me).map(|c| c.group_id) else {
            return Vec::new();
        };
        agents
            .into_iter()
            .filter(|c| c.id != me && c.group_id == group && c.lifecycle.is_discoverable())
            .map(|c| c.directory_entry())
            .collect()
    }

    fn name_table(&self) -> NameTable {
        self.inner
            .store
            .list_agents()
            .unwrap_or_default()
            .into_iter()
            .map(|c| (c.id, c.name))
            .collect()
    }
}

async fn actor_loop(
    runtime: Runtime,
    id: AgentId,
    mut rx: mpsc::UnboundedReceiver<Envelope>,
    depth: Arc<AtomicUsize>,
    resume: Arc<Notify>,
) {
    // Carries an envelope that was pulled but does not belong in the current
    // batch, so nothing is lost between iterations.
    let mut carry: Option<Envelope> = None;

    loop {
        let first = match carry.take() {
            Some(envelope) => envelope,
            None => match rx.recv().await {
                Some(envelope) => envelope,
                None => break,
            },
        };
        depth.fetch_sub(1, Ordering::SeqCst);

        // A paused agent holds what it has and lets the rest queue behind it.
        //
        // Deletion has to be distinguished from pausing here. Both stop the
        // agent accepting work, but `stop_agent` drops the inbox, which holds
        // the only other handle to this notifier. Parking on a deleted agent
        // would wait for a wake-up that can never come, leaking the task and
        // the envelope it is holding for the life of the process.
        let mut abandoned = false;
        loop {
            match runtime.inner.store.get_agent(id).ok().flatten() {
                None => {
                    abandoned = true;
                    break;
                }
                Some(card) if card.lifecycle == Lifecycle::Terminated => {
                    abandoned = true;
                    break;
                }
                Some(card) if card.lifecycle.accepts_work() => break,
                Some(_) => {
                    runtime.set_activity(id, Activity::Paused);
                    resume.notified().await;
                }
            }
        }
        if abandoned {
            break;
        }

        let mut batch = vec![first];

        // Messages that do not want an answer are pure context, so reading a
        // burst of them in one turn is both cheaper and less noisy. Messages
        // that do want an answer are handled one at a time, because each
        // produces its own addressed reply.
        if !batch[0].expects_reply {
            while batch.len() < MAX_BATCH {
                match rx.try_recv() {
                    Ok(next) if !next.expects_reply && next.run_id == batch[0].run_id => {
                        depth.fetch_sub(1, Ordering::SeqCst);
                        batch.push(next);
                    }
                    Ok(next) => {
                        carry = Some(next);
                        break;
                    }
                    Err(_) => break,
                }
            }
        }

        runtime.run_turn(id, batch).await;
    }

    tracing::debug!(agent = %id.short(), "actor stopped");
}
