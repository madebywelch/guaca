//! Stopping a host without ending the work on it, and picking that work up on
//! the next one.
//!
//! An update replaces the process every agent runs in, and on a box that
//! follows `main` that is several times a day. Ending every conversation in
//! flight each time, and asking the operator to send each one again, was the
//! price of one rule: nothing interrupted in the middle of an action is ever
//! repeated, because nobody can say whether it finished.
//!
//! A drain keeps the rule and stops paying the price. A host asked to stop
//! stops starting things, and every turn in flight halts at the next boundary
//! a stop already uses, where each call it made has come back and been written
//! down. A model call in the air is dropped rather than waited out: it has no
//! effect outside this process, and waiting would hold the update for as long
//! as the model thinks. What the turn did is recorded in its channel, its
//! messages are put down with that record, and the next host delivers them
//! again with the record in the prompt. The work carries on from a point
//! nobody has to vouch for. See `prompt::resumed` for what the turn is told.
//!
//! What does not reach a boundary in time is not put down, and the next host
//! reports it interrupted exactly as a crash is reported: a turn still inside
//! a tool call when the process ends is the case the rule exists for.
//!
//! Coding jobs are stopped the way the operator's own Stop stops them, through
//! each program's protocol, which ends the program's turn and keeps its
//! session. The next host continues the session. The program keeps its own
//! record of every step, which is what makes that safe, and is told its last
//! step may not have finished.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{mpsc, Notify};

use super::{Launch, Origin, Runtime, RuntimeError};
use crate::db::{PutDownJob, Recovered};
use crate::domain::agent::{AgentCard, Lifecycle};
use crate::domain::envelope::{Envelope, Intent, NoticeKind, Part, Participant, Trust};
use crate::domain::ids::{AgentId, MessageId, RunId};
use crate::domain::now_ms;
use crate::domain::terminal::Harness;

/// How often a drain looks to see whether everything is down.
///
/// A look is two uncontended locks. Every way work gets put down would need a
/// wake-up of its own otherwise, and a missed one would hold the host up for
/// the whole window.
const LOOK: Duration = Duration::from_millis(50);

/// What a resumed coding job is told, in place of a brief.
const RESUMED_JOB: &str = "The host you run on restarted for an update and stopped you partway \
     through. Carry on with the same task from where you stopped. Your last step may not have \
     finished: check the working tree, and anything you were in the middle of, before you go on. \
     Do not repeat a push, a pull request or anything else outside this directory that already \
     happened.";

/// What a drain came to, for the host to log.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Drained {
    /// Agents still in the middle of something when the time ran out. The
    /// next host reports their conversations interrupted.
    pub working: Vec<String>,
    /// Coding jobs that had not ended.
    pub jobs: usize,
}

impl Drained {
    pub fn is_complete(&self) -> bool {
        self.working.is_empty() && self.jobs == 0
    }
}

impl Runtime {
    /// Brings every agent to a halt at its next boundary and puts its work down
    /// for the next host, waiting at most `within`.
    ///
    /// Nothing starts after this is called: no turn, no routine, no coding job.
    /// Messages still arrive and are kept, put down for the next host rather
    /// than queued here.
    ///
    /// `within` has to be shorter than what the host is given to stop in, or
    /// the process is killed with the drain half done. That is still safe, only
    /// wasteful: every put-down is its own commit, and whatever had not been
    /// put down is reported interrupted.
    pub async fn drain(&self, within: Duration) -> Drained {
        let began = tokio::time::Instant::now();
        let live: Vec<RunId> = {
            let mut runs = self.inner.runs.lock();
            runs.draining = true;
            runs.outstanding.keys().copied().collect()
        };
        let jobs = self.inner.coding.lock().len();
        tracing::info!(
            conversations = live.len(),
            jobs,
            "the host is stopping: putting work down for the next one"
        );

        // A turn parked on the operator is waiting for an answer that cannot
        // come in time. Expired rather than refused, as a stop does it: nobody
        // said no, and the turn picked up later asks again.
        for run in &live {
            self.release_parked(*run);
        }
        // Every model call in flight is raced against this, and every paused
        // actor parks on it.
        let notifiers: Vec<Arc<Notify>> =
            self.inner.inboxes.lock().values().map(|inbox| inbox.resume.clone()).collect();
        for resume in notifiers {
            resume.notify_waiters();
        }

        let deadline = began + within;
        loop {
            self.stop_jobs();
            let left = self.inner.runs.lock().outstanding.len();
            let jobs = self.inner.coding.lock().len();
            if left == 0 && jobs == 0 {
                tracing::info!(
                    took_ms = began.elapsed().as_millis() as u64,
                    "every conversation and coding job is put down"
                );
                return Drained::default();
            }
            if tokio::time::Instant::now() >= deadline {
                let working = self.still_working();
                tracing::warn!(
                    ?working,
                    conversations = left,
                    jobs,
                    "the host is stopping with work that did not reach a boundary; the next one \
                     reports it interrupted"
                );
                return Drained { working, jobs };
            }
            tokio::time::sleep(LOOK).await;
        }
    }

    /// Asks every coding job nobody has stopped yet to stop, for the next host.
    ///
    /// Asked again on every look rather than once, so a job whose launch was
    /// already past its own check when the drain began is still reached.
    fn stop_jobs(&self) {
        let mut coding = self.inner.coding.lock();
        for job in coding.values_mut() {
            if job.stopped_by.is_none() && !job.put_down {
                job.put_down = true;
                let _ = job.controls.try_send(crate::coding::Control::Stop);
            }
        }
    }

    fn still_working(&self) -> Vec<String> {
        let busy: Vec<AgentId> = self
            .inner
            .activity
            .lock()
            .iter()
            .filter(|(_, activity)| activity.is_working())
            .map(|(agent, _)| *agent)
            .collect();
        let mut names: Vec<String> = busy.into_iter().map(|agent| self.agent_name(agent)).collect();
        names.sort();
        names
    }

    /// What a run has spent, as the guard counts it.
    pub(super) fn steps_of(&self, run: RunId) -> u32 {
        self.inner.guard.lock().peek(run).map(|state| state.steps_used()).unwrap_or(0)
    }

    /// Puts envelopes down for the next host.
    ///
    /// `booked` are still counted against their runs, and are released here
    /// without settling them: a run whose last booking is put down is handed on
    /// whole, not finished, and emits nothing. `absorbed` were taken into a
    /// running turn and released when it read them, so only their rows are
    /// written. `records` go with the first of `booked`, which is the message
    /// the turn was answering.
    pub(super) fn put_down(
        &self,
        booked: &[Envelope],
        absorbed: &[Envelope],
        records: &[MessageId],
    ) {
        let touched: HashSet<RunId> =
            booked.iter().chain(absorbed).map(|envelope| envelope.run_id).collect();
        let steps: HashMap<RunId, u32> =
            touched.iter().map(|run| (*run, self.steps_of(*run))).collect();
        let messages: Vec<(MessageId, &[MessageId])> = booked
            .iter()
            .enumerate()
            .map(|(at, envelope)| (envelope.id, if at == 0 { records } else { &[][..] }))
            .chain(absorbed.iter().map(|envelope| (envelope.id, &[][..])))
            .collect();

        let mut runs = self.inner.runs.lock();
        for envelope in booked {
            if let Some(count) = runs.outstanding.get_mut(&envelope.run_id) {
                *count = count.saturating_sub(1);
                if *count == 0 {
                    runs.outstanding.remove(&envelope.run_id);
                    runs.refused.remove(&envelope.run_id);
                }
            }
        }
        runs.put_down.extend(touched.iter().copied());
        let whole: Vec<(RunId, u32)> = touched
            .iter()
            .filter(|run| !runs.outstanding.contains_key(run))
            .map(|run| (*run, steps[run]))
            .collect();
        // Under the lock, so `whole` is still true when it is written.
        if let Err(err) = self.inner.store.put_down(&messages, &whole) {
            tracing::error!(%err, "could not put work down; the next host reports it interrupted");
        }
    }

    /// Hands a run on whole: the last of it that was running has ended, and
    /// the rest is put down.
    ///
    /// Called with the run lock held, by `track_inflight`, so it takes no lock
    /// of its own but the guard's.
    pub(super) fn mark_whole(&self, runs: &[RunId]) {
        let whole: Vec<(RunId, u32)> = runs.iter().map(|run| (*run, self.steps_of(*run))).collect();
        if let Err(err) = self.inner.store.put_down(&[], &whole) {
            tracing::error!(%err, "could not hand a conversation on; the next host reports it interrupted");
        }
    }

    /// Puts down everything an actor is holding: the envelope it has just
    /// taken, its holding queue, and whatever is still in its inbox.
    ///
    /// What the operator stopped is ended as a stop ends it, with the notice,
    /// because work called off is never picked up.
    pub(super) fn put_down_queue(
        &self,
        agent: AgentId,
        first: Envelope,
        carry: &mut VecDeque<Envelope>,
        rx: &mut mpsc::UnboundedReceiver<Envelope>,
        depth: &AtomicUsize,
    ) {
        let mut held = vec![first];
        for envelope in carry.drain(..) {
            depth.fetch_sub(1, Ordering::SeqCst);
            held.push(envelope);
        }
        while let Ok(envelope) = rx.try_recv() {
            depth.fetch_sub(1, Ordering::SeqCst);
            held.push(envelope);
        }
        let (called_off, kept): (Vec<Envelope>, Vec<Envelope>) =
            held.into_iter().partition(|envelope| self.called_off(envelope.run_id));
        for envelope in called_off {
            self.notice(
                agent,
                envelope.run_id,
                Some(envelope.id),
                NoticeKind::GuardStop,
                format!(
                    "You stopped this conversation, so {} never started this. Nothing was sent \
                     on. Send it again if you want it done.",
                    self.agent_name(agent)
                ),
            );
            self.finish_turn(agent, envelope.run_id, 1);
        }
        if !kept.is_empty() {
            self.put_down(&kept, &[], &[]);
        }
        self.idle(agent);
    }

    /// Ends a turn the host stopped at a boundary: writes down what it did, in
    /// its own channel, and puts its messages down with that record.
    ///
    /// The record is the only thing the next turn learns the calls from, so a
    /// turn whose record cannot be written is not put down at all. It stays
    /// counted, the drain runs out of time on it, and the next host reports
    /// it interrupted: repeating its calls blind is the one outcome worse
    /// than that.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn put_turn_down(
        &self,
        card: &AgentCard,
        run_id: RunId,
        hop: u16,
        cause: Option<MessageId>,
        batch: &[Envelope],
        taken_in: &[Envelope],
        text: String,
        mut parts: Vec<Part>,
        mut records: Vec<MessageId>,
    ) {
        let text = text.trim().to_string();
        let did = !text.is_empty() || !parts.is_empty();
        if !text.is_empty() {
            parts.push(Part::Text { text });
        }
        parts.push(Part::Notice {
            kind: NoticeKind::Lifecycle,
            text: format!(
                "{} stopped here because the host is restarting, with everything above finished, \
                 and carries on from this point once it is back.",
                card.name
            ),
        });
        let record = Envelope {
            id: MessageId::new(),
            run_id,
            channel_id: card.id,
            from: Participant::Agent { id: card.id },
            to: Participant::System,
            parts,
            trust: Trust::System,
            hop,
            expects_reply: false,
            intent: Intent::Courtesy,
            cause,
            created_at: now_ms(),
        };
        let id = record.id;
        match self.deliver(record) {
            Ok(()) => {
                if did {
                    records.push(id);
                }
            }
            Err(err) if did => {
                tracing::error!(
                    %err,
                    agent = %card.name,
                    "could not write down what a stopped turn did; it is left to be reported \
                     interrupted rather than picked up blind"
                );
                return;
            }
            Err(err) => tracing::warn!(%err, "could not note a stopped turn in its channel"),
        }
        self.put_down(batch, taken_in, &records);
        self.idle(card.id);
    }

    /// The records of what a turn picked up after a restart had already done,
    /// taken so no other turn reads them.
    pub(super) fn resumed(&self, batch: &[Envelope]) -> Vec<MessageId> {
        let mut resuming = self.inner.resuming.lock();
        batch.iter().find_map(|envelope| resuming.remove(&envelope.id)).unwrap_or_default()
    }

    /// Picks up what the last host put down. Called once at boot, after the
    /// actors start and before the scheduler does, so an agent with work to
    /// carry on reads as busy to the first sweep.
    ///
    /// Each conversation carries on against the budget it had spent, not a
    /// fresh one: an update every few hours must not be a way for a cascade to
    /// outlive its limits.
    pub fn pick_up(&self, recovered: Recovered) {
        let Recovered { resumed, steps, jobs, .. } = recovered;
        let conversations = steps.len();
        for (run, spent) in &steps {
            let card = resumed.iter().find(|put| put.envelope.run_id == *run).and_then(|put| {
                match put.envelope.to {
                    Participant::Agent { id } => self.inner.store.get_agent(id).ok().flatten(),
                    _ => None,
                }
            });
            if let Some(card) = card {
                let limits = self.limits_for(&card);
                self.inner.guard.lock().run_within(*run, limits).resume_at(*spent);
            }
        }
        let messages = resumed.len();
        for put in resumed {
            if !put.records.is_empty() {
                self.inner.resuming.lock().insert(put.envelope.id, put.records);
            }
            *self.inner.runs.lock().outstanding.entry(put.envelope.run_id).or_insert(0) += 1;
            self.queue(put.envelope);
        }
        let carried = jobs.len();
        for job in jobs {
            self.resume_job(job);
        }
        if messages > 0 || carried > 0 {
            tracing::info!(
                messages,
                conversations,
                jobs = carried,
                "picked up the work the last host put down"
            );
        }
    }

    /// Carries a coding job the last host stopped on in the same session, or
    /// starts it over when it had no session yet.
    fn resume_job(&self, job: PutDownJob) {
        let card = match self.inner.store.get_agent(job.agent) {
            Ok(Some(card)) if card.lifecycle != Lifecycle::Terminated => card,
            _ => return,
        };
        let origin = if job.by_operator { Origin::Operator } else { Origin::Agent };
        let fresh = job.session.is_empty();
        let started = card
            .has_terminal
            .then_some(())
            .ok_or_else(|| RuntimeError::NoTerminal(card.name.clone()))
            .and_then(|()| {
                self.inner
                    .terminals
                    .directory(card.id, Some(&job.directory))
                    .map_err(RuntimeError::from)
            })
            .and_then(|(working, shown)| {
                self.launch(
                    &card,
                    Launch {
                        working,
                        shown,
                        brief: if fresh { job.task.clone() } else { RESUMED_JOB.to_string() },
                        session: match (fresh, job.harness) {
                            (false, _) => job.session.clone(),
                            (true, Harness::Codex) => String::new(),
                            (true, Harness::Pi | Harness::Claude) => {
                                uuid::Uuid::new_v4().to_string()
                            }
                        },
                        resume: !fresh,
                        harness: job.harness,
                        origin,
                    },
                )
            });
        let Err(err) = started else { return };
        tracing::warn!(%err, agent = %card.name, "could not carry a coding job on after the restart");
        let text = format!(
            "The host restarted for an update while your coding agent was working in `{}`, and it \
             could not be started again afterwards: {err}. Whatever it had already committed is \
             still there. Say so, and carry the work on with `code` and action `continue` if it \
             still needs doing.",
            job.directory
        );
        let envelope = Envelope {
            id: MessageId::new(),
            run_id: RunId::new(),
            channel_id: card.id,
            from: Participant::System,
            to: Participant::Agent { id: card.id },
            parts: vec![Part::Text { text }],
            trust: Trust::System,
            hop: 0,
            expects_reply: true,
            intent: Intent::Work,
            cause: None,
            created_at: now_ms(),
        };
        if let Err(err) = self.deliver(envelope) {
            tracing::error!(%err, agent = %card.name, "a coding job the restart ended reached nobody");
        }
    }
}
