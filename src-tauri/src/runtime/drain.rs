use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{mpsc, Notify};

use super::{CutShort, Intake, RunGuard, Runs, Runtime};
use crate::db::Recovered;
use crate::domain::agent::AgentCard;
use crate::domain::envelope::{Envelope, Intent, NoticeKind, Part, Participant, Trust};
use crate::domain::ids::{AgentId, MessageId, RunId};
use crate::domain::now_ms;

const LOOK: Duration = Duration::from_millis(50);

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Drained {
    pub working: Vec<String>,
}

impl Drained {
    pub fn is_complete(&self) -> bool {
        self.working.is_empty()
    }
}

impl Runtime {
    pub async fn drain(&self, within: Duration) -> Drained {
        let began = tokio::time::Instant::now();
        let live: Vec<RunId> = {
            let mut runs = self.inner.runs.lock();
            runs.draining = true;
            runs.outstanding.keys().copied().collect()
        };
        tracing::info!(
            conversations = live.len(),
            "the host is stopping: putting work down for the next one"
        );

        for run in &live {
            self.release_parked(*run);
        }
        let notifiers: Vec<Arc<Notify>> =
            self.inner.inboxes.lock().values().map(|inbox| inbox.resume.clone()).collect();
        for resume in notifiers {
            resume.notify_waiters();
        }

        let deadline = began + within;
        loop {
            let left = self.inner.runs.lock().outstanding.len();
            if left == 0 {
                tracing::info!(
                    took_ms = began.elapsed().as_millis() as u64,
                    "every conversation is put down"
                );
                return Drained::default();
            }
            if tokio::time::Instant::now() >= deadline {
                let working = self.still_working();
                tracing::warn!(
                    ?working,
                    conversations = left,
                    "the host is stopping with work that did not reach a boundary; the next one \
                     reports it interrupted"
                );
                return Drained { working };
            }
            tokio::time::sleep(LOOK).await;
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

    pub(super) fn put_down(
        &self,
        booked: &[Envelope],
        absorbed: &[Envelope],
        taken: &[Envelope],
        acted_on: usize,
        records: &[MessageId],
    ) -> bool {
        let routine = Envelope::is_routine;
        for envelope in taken.iter().filter(|envelope| routine(envelope)) {
            self.track_inflight(envelope.run_id, -1);
        }
        let absorbed: Vec<&Envelope> = absorbed.iter().chain(taken).collect();
        let routine_absorbed: Vec<MessageId> = absorbed
            .iter()
            .filter(|envelope| routine(envelope))
            .map(|envelope| envelope.id)
            .collect();
        let other_absorbed: Vec<_> = absorbed
            .iter()
            .copied()
            .filter(|envelope| !routine_absorbed.contains(&envelope.id))
            .collect();
        let touched: HashSet<RunId> = booked
            .iter()
            .chain(other_absorbed.iter().copied())
            .map(|envelope| envelope.run_id)
            .collect();
        let limits: HashMap<RunId, _> = booked
            .iter()
            .chain(other_absorbed.iter().copied())
            .filter_map(|envelope| {
                let Participant::Agent { id } = envelope.to else { return None };
                self.inner
                    .store
                    .get_agent(id)
                    .ok()
                    .flatten()
                    .map(|card| (envelope.run_id, self.limits_for(&card)))
            })
            .collect();
        let mut runs = self.inner.runs.lock();
        let called_off: HashSet<RunId> =
            touched.iter().copied().filter(|run| runs.stopped.contains(run)).collect();
        let live = |run: &RunId| !called_off.contains(run);
        let carried = booked.iter().any(|envelope| live(&envelope.run_id));
        let owed: HashSet<RunId> = taken[..acted_on]
            .iter()
            .filter(|envelope| !carried && !routine(envelope) && live(&envelope.run_id))
            .map(|envelope| envelope.run_id)
            .collect();
        let mut ended = Vec::new();
        let mut cut = Vec::new();
        for envelope in booked.iter().chain(taken.iter().filter(|envelope| !routine(envelope))) {
            if owed.contains(&envelope.run_id) {
                runs.absorbed.remove(&envelope.run_id);
                runs.cut_short.insert(envelope.run_id);
                cut.push(envelope);
                continue;
            }
            if live(&envelope.run_id) {
                Self::release_bookings(&mut runs, envelope.run_id, 1);
                continue;
            }
            runs.absorbed.remove(&envelope.run_id);
            ended.push(envelope.run_id);
        }
        let kept: Vec<&Envelope> =
            booked.iter().filter(|envelope| live(&envelope.run_id)).collect();
        let linked: Vec<MessageId> = if carried {
            taken[..acted_on]
                .iter()
                .filter(|envelope| !routine(envelope) && live(&envelope.run_id))
                .map(|envelope| envelope.id)
                .collect()
        } else {
            Vec::new()
        };
        let messages: Vec<(MessageId, &[MessageId])> = kept
            .iter()
            .enumerate()
            .map(|(at, envelope)| (envelope.id, if at == 0 { records } else { &[][..] }))
            .chain(
                other_absorbed
                    .iter()
                    .filter(|envelope| {
                        live(&envelope.run_id)
                            && !owed.contains(&envelope.run_id)
                            && !linked.contains(&envelope.id)
                    })
                    .map(|envelope| (envelope.id, &[][..])),
            )
            .collect();
        let with_carrier: Vec<MessageId> = if carried {
            routine_absorbed.iter().chain(&linked).copied().collect()
        } else {
            Vec::new()
        };
        let touched: HashSet<RunId> =
            touched.iter().filter(|run| live(run) && !owed.contains(run)).copied().collect();
        runs.put_down.extend(touched.iter().copied());
        let mut guard = self.inner.guard.lock();
        let whole: Vec<_> = touched
            .iter()
            .filter(|run| !runs.outstanding.contains_key(run))
            .filter_map(|run| {
                if guard.peek(*run).is_none() {
                    guard.run_within(*run, *limits.get(run)?);
                }
                let state = guard.peek(*run)?;
                Some((*run, state.steps_used(), state.snapshot()))
            })
            .collect();
        drop(guard);
        let written = match self.inner.store.put_down_with_absorbed(
            &runs,
            &messages,
            &whole,
            &with_carrier,
        ) {
            Ok(()) => true,
            Err(err) => {
                tracing::error!(%err, "could not put work down; the next host reports it interrupted");
                false
            }
        };
        drop(runs);
        for envelope in cut {
            if let Participant::Agent { id } = envelope.to {
                self.cut_short_notice(id, envelope, CutShort::Stopped);
            }
            ended.push(envelope.run_id);
        }
        for run in ended {
            self.track_inflight(run, -1);
        }
        written && !kept.is_empty()
    }

    pub(super) fn mark_whole(&self, held: &RunGuard<'_, Runs>, runs: &[RunId]) {
        let guard = self.inner.guard.lock();
        let whole: Vec<_> = runs
            .iter()
            .filter_map(|run| {
                guard.peek(*run).map(|state| (*run, state.steps_used(), state.snapshot()))
            })
            .collect();
        drop(guard);
        if let Err(err) = self.inner.store.put_down(held, &[], &whole) {
            tracing::error!(%err, "could not hand a conversation on; the next host reports it interrupted");
        }
    }

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
            let stopped = envelope.clone();
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
            self.let_go(agent, &stopped, &mut Intake { rx: &mut *rx, carry: &mut *carry, depth });
        }
        if !kept.is_empty() {
            for envelope in kept {
                let (records, absorbed_ids) = self.resumed(std::slice::from_ref(&envelope));
                let absorbed: Option<Vec<_>> = absorbed_ids
                    .iter()
                    .map(|id| self.inner.store.get_message(*id).ok().flatten())
                    .collect();
                let Some(absorbed) = absorbed else {
                    tracing::error!("could not read an absorbed routine while putting work down");
                    continue;
                };
                let (carried, absorbed): (Vec<_>, Vec<_>) =
                    absorbed.into_iter().partition(|envelope| !envelope.is_routine());
                self.put_down(&[envelope], &absorbed, &carried, carried.len(), &records);
            }
        }
        self.idle(agent);
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn put_turn_down(
        &self,
        card: &AgentCard,
        run_id: RunId,
        hop: u16,
        cause: Option<MessageId>,
        batch: &[Envelope],
        absorbed: &[Envelope],
        taken_in: &[Envelope],
        acted_on: usize,
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
        self.put_down(batch, absorbed, taken_in, acted_on, &records);
        self.idle(card.id);
    }

    pub(super) fn resumed(&self, batch: &[Envelope]) -> (Vec<MessageId>, Vec<MessageId>) {
        let mut resuming = self.inner.resuming.lock();
        batch.iter().find_map(|envelope| resuming.remove(&envelope.id)).unwrap_or_default()
    }

    pub fn pick_up(&self, recovered: Recovered) {
        let Recovered { resumed, carried, steps, guards, .. } = recovered;
        let conversations = steps.len();
        let legacy = steps.len().saturating_sub(guards.len());
        if legacy > 0 {
            tracing::warn!(count = legacy, "legacy runs resumed with a fresh guard");
        }
        {
            let mut runs = self.inner.runs.lock();
            for run in resumed.iter().map(|put| put.envelope.run_id).chain(carried) {
                self.book(&mut runs, run);
            }
        }
        let restored: HashSet<_> = guards.keys().copied().collect();
        {
            let mut guard = self.inner.guard.lock();
            for (run, state) in guards {
                guard.restore(run, state);
            }
        }
        for (run, spent) in &steps {
            if restored.contains(run) {
                continue;
            }
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
        let mut envelopes = Vec::with_capacity(messages);
        for put in resumed {
            if !put.records.is_empty() || !put.absorbed.is_empty() {
                self.inner.resuming.lock().insert(put.envelope.id, (put.records, put.absorbed));
            }
            envelopes.push(put.envelope);
        }
        self.queue_all(envelopes);
        if messages > 0 {
            tracing::info!(messages, conversations, "picked up the work the last host put down");
        }
    }
}
