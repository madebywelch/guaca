use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::watch;

use super::events::{EventSink, UiEvent};
use super::Runtime;
use crate::db::{NewRelay, PlaceRun, RelayRow, RelayState, StoreError};
use crate::domain::approval::Request;
use crate::domain::envelope::Part;
use crate::domain::ids::RunId;
use crate::domain::now_ms;
use crate::domain::place::{self, Abilities, Card, CardKind, Outgoing, PostSafety, RelayTag, Spot};
use crate::places::{Adapter, SendError};

const STEPS: [u64; 4] = [2, 10, 60, 600];
const HOURLY: u64 = 3600;
const GIVE_UP_MS: i64 = 24 * 60 * 60 * 1000;
const TICK: Duration = Duration::from_secs(60);
const WAITED: &str = "The provider asked Guaca to wait before sending this.";
const GAVE_UP: &str = "Guaca gave up after trying to send this for a day.";
const UNCERTAIN: &str = "This may have reached the conversation. Check there before trying again.";

pub(super) fn backoff(attempt: u32) -> Duration {
    let step = STEPS.get(attempt.saturating_sub(1) as usize).copied().unwrap_or(HOURLY);
    Duration::from_secs(step)
}

pub(super) struct Waking {
    pub(super) sink: Arc<dyn EventSink>,
    pub(super) wake: watch::Sender<u64>,
}

impl EventSink for Waking {
    fn emit(&self, event: UiEvent) {
        let relayable = matches!(
            event,
            UiEvent::MessageAppended { .. }
                | UiEvent::ApprovalRequested { .. }
                | UiEvent::ApprovalSettled { .. }
                | UiEvent::OutcomeRecorded { .. }
                | UiEvent::RunSettled { .. }
                | UiEvent::ActivityChanged { .. }
        );
        self.sink.emit(event);
        if relayable {
            self.wake.send_modify(|count| *count = count.wrapping_add(1));
        }
    }
}

struct Want {
    key: String,
    spot: Spot,
    run: Option<RunId>,
    at: i64,
    rank: u8,
    sub: usize,
    item: Outgoing,
}

impl Want {
    fn new(key: String, spot: &Spot, run: RunId, at: (i64, u8, usize), item: Outgoing) -> Self {
        Want { key, spot: spot.clone(), run: Some(run), at: at.0, rank: at.1, sub: at.2, item }
    }
}

fn texts(parts: &[Part]) -> String {
    parts
        .iter()
        .filter_map(|part| match part {
            Part::Text { text } => Some(text.trim()),
            _ => None,
        })
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

impl Runtime {
    pub(super) fn wake_relay(&self) {
        self.inner.relay_wake.send_modify(|count| *count = count.wrapping_add(1));
    }

    pub(super) async fn relay(self, adapter: Arc<dyn Adapter>) {
        let id = adapter.id();
        let mut woken = self.inner.relay_wake.subscribe();
        loop {
            woken.borrow_and_update();
            loop {
                match self.relay_round(&adapter).await {
                    Ok(true) => continue,
                    Ok(false) => break,
                    Err(err) => {
                        tracing::warn!(%err, adapter = %id.0, "the relay could not read or record what it sends");
                        break;
                    }
                }
            }
            let wait = match self.inner.store.relay_next_at(&id) {
                Ok(Some(at)) => Duration::from_millis((at - now_ms()).max(0) as u64),
                _ => TICK,
            };
            tokio::select! {
                changed = woken.changed() => if changed.is_err() { return },
                _ = tokio::time::sleep(wait.min(TICK)) => {}
            }
        }
    }

    async fn relay_round(&self, adapter: &Arc<dyn Adapter>) -> Result<bool, StoreError> {
        let id = adapter.id();
        let abilities = adapter.abilities();
        let store = &self.inner.store;
        let now = now_ms();
        let rows = store.relay_rows(&id)?;
        let known: HashMap<&str, &RelayRow> =
            rows.iter().map(|row| (row.key.as_str(), row)).collect();
        let runs = store.open_place_runs(&id)?;
        let mut wants = Vec::new();
        for run in &runs {
            let before = wants.len();
            let finished = self.wants_for(run, &abilities, &known, now, &mut wants)?;
            let open = rows.iter().any(|row| row.run == Some(run.run) && row.state.open());
            if finished && wants.len() == before && !open {
                store.finish_place_run(run.run)?;
            }
        }
        wants.sort_by(|a, b| (a.at, a.rank, a.sub, &a.key).cmp(&(b.at, b.rank, b.sub, &b.key)));

        let mut spots: Vec<&Spot> = Vec::new();
        for spot in rows
            .iter()
            .filter(|row| row.state.open())
            .map(|row| &row.spot)
            .chain(wants.iter().map(|w| &w.spot))
        {
            if !spots.contains(&spot) {
                spots.push(spot);
            }
        }
        let mut progressed = false;
        for spot in spots {
            if let Some(head) = rows.iter().find(|row| row.state.open() && &row.spot == spot) {
                if head.next_at > now {
                    continue;
                }
                match head.state {
                    RelayState::Sending => self.recover(adapter, &abilities, head).await?,
                    _ => {
                        store.relay_to(&head.key, RelayState::Sending)?;
                        self.send_out(adapter, head).await?;
                    }
                }
                progressed = true;
                continue;
            }
            let Some(want) = wants.iter().find(|want| &want.spot == spot) else { continue };
            let skip = matches!(want.item, Outgoing::Skip);
            let state = if skip { RelayState::Skipped } else { RelayState::Sending };
            store.insert_relay(&NewRelay {
                key: &want.key,
                adapter: &id,
                spot: &want.spot,
                run: want.run,
                state,
                item: &want.item,
            })?;
            if !skip {
                let row = store.relay_row(&want.key)?.ok_or_else(|| {
                    StoreError::Corrupt(format!("relay row {} vanished after its insert", want.key))
                })?;
                self.send_out(adapter, &row).await?;
            }
            progressed = true;
        }
        Ok(progressed)
    }

    async fn recover(
        &self,
        adapter: &Arc<dyn Adapter>,
        abilities: &Abilities,
        row: &RelayRow,
    ) -> Result<(), StoreError> {
        let store = &self.inner.store;
        if !matches!(row.item, Outgoing::Post { .. }) {
            return self.send_out(adapter, row).await;
        }
        match abilities.posts {
            PostSafety::Idempotent => self.send_out(adapter, row).await,
            PostSafety::Neither => store.relay_doubt(&row.key, UNCERTAIN),
            PostSafety::Findable => {
                match adapter.find(&row.spot, &RelayTag(row.key.clone())).await {
                    Ok(Some(posted)) => store.relay_sent(&row.key, Some(&posted)),
                    Ok(None) => store.relay_doubt(&row.key, UNCERTAIN),
                    Err(SendError::Retry { after }) => {
                        let wait = after.unwrap_or_else(|| backoff(row.attempts + 1));
                        store.relay_hold(&row.key, now_ms() + wait.as_millis() as i64, WAITED)
                    }
                    Err(SendError::Refused { reason } | SendError::Gone { reason }) => {
                        store.relay_doubt(&row.key, &reason)
                    }
                }
            }
        }
    }

    async fn send_out(&self, adapter: &Arc<dyn Adapter>, row: &RelayRow) -> Result<(), StoreError> {
        let store = &self.inner.store;
        let tag = RelayTag(row.key.clone());
        let sent = match &row.item {
            Outgoing::Post { card } => adapter.post(&row.spot, card, &tag).await.map(Some),
            Outgoing::Edit { posted, card } => {
                adapter.edit(&row.spot, posted, card).await.map(|()| None)
            }
            Outgoing::Mark { heard, mark } => {
                adapter.mark(heard, &row.spot, *mark).await.map(|()| None)
            }
            Outgoing::Skip => Ok(None),
        };
        let now = now_ms();
        match sent {
            Ok(posted) => store.relay_sent(&row.key, posted.as_ref()),
            Err(SendError::Retry { .. }) if now - row.created_at > GIVE_UP_MS => {
                store.relay_failed(&row.key, GAVE_UP)
            }
            Err(SendError::Retry { after }) => {
                let wait = after.unwrap_or_else(|| backoff(row.attempts + 1));
                store.relay_later(&row.key, now + wait.as_millis() as i64, WAITED)
            }
            Err(SendError::Refused { reason } | SendError::Gone { reason }) => {
                store.relay_failed(&row.key, &reason)
            }
        }
    }

    fn wants_for(
        &self,
        run: &PlaceRun,
        abilities: &Abilities,
        known: &HashMap<&str, &RelayRow>,
        now: i64,
        wants: &mut Vec<Want>,
    ) -> Result<bool, StoreError> {
        let store = &self.inner.store;
        let id = run.run;
        let fresh = |key: &str, wants: &Vec<Want>| {
            !known.contains_key(key) && wants.iter().all(|w| w.key != key)
        };
        let outcome = store.outcome(id)?;
        let mark = |mark: place::Mark| {
            if abilities.reactions {
                Outgoing::Mark { heard: run.heard.clone(), mark }
            } else {
                Outgoing::Skip
            }
        };
        let mut posts: Vec<Want> = Vec::new();

        if run.started_at.is_some() || outcome.is_some() {
            let key = format!("mark:{id}:{}", place::Mark::Working.as_str());
            if fresh(&key, wants) {
                let at = run.started_at.unwrap_or(run.created_at);
                wants.push(Want::new(key, &run.spot, id, (at, 1, 0), mark(place::Mark::Working)));
            }
        }

        for reply in store.replies_in_run(id)? {
            let text = texts(&reply.parts);
            let base = format!("message:{}", reply.id);
            if text.is_empty() {
                if fresh(&base, wants) {
                    wants.push(Want::new(
                        base,
                        &run.spot,
                        id,
                        (reply.created_at, 2, 0),
                        Outgoing::Skip,
                    ));
                }
                continue;
            }
            let name = match reply.from {
                crate::domain::envelope::Participant::Agent { id } => {
                    store.get_agent(id)?.map(|card| card.name).unwrap_or_default()
                }
                _ => String::new(),
            };
            for (at, card) in
                place::fit(place::reply(&name, &text), abilities).into_iter().enumerate()
            {
                let key = if at == 0 { base.clone() } else { format!("{base}:{}", at + 1) };
                if fresh(&key, &posts) && fresh(&key, wants) {
                    posts.push(Want::new(
                        key,
                        &run.spot,
                        id,
                        (reply.created_at, 2, at),
                        Outgoing::Post { card },
                    ));
                }
            }
        }

        let direct =
            store.linked(&run.adapter)?.map(|linked| Spot { place: linked.place, thread: None });
        for approval_id in store.approvals_in_run(id)? {
            let Some(approval) = store.get_approval(approval_id)? else { continue };
            let name =
                store.get_agent(approval.agent_id)?.map(|card| card.name).unwrap_or_default();
            let ask_key = format!("ask:{approval_id}");
            let settle_key = format!("settle:{approval_id}");
            let ask = approval_id.to_string();
            let (card, spot) = match &approval.request {
                Request::Question { options } => {
                    (place::question(&name, &ask, &approval.summary, options), run.spot.clone())
                }
                Request::Permission { action } => {
                    let Some(spot) = direct.clone() else { continue };
                    let fields = approval
                        .detail
                        .iter()
                        .map(|f| (f.label.clone(), f.value.clone()))
                        .collect();
                    let card = place::permission(&name, &ask, &approval.summary, fields, *action);
                    (card, spot)
                }
            };
            let settled = place::settled(approval.state, approval.answer.as_deref(), &name);
            if card.kind == CardKind::Permission && !run.direct {
                let key = format!("notice:{id}:waiting");
                if fresh(&key, &posts) && fresh(&key, wants) {
                    let item =
                        Outgoing::Post { card: place::notice(place::waiting_for_owner(&name)) };
                    posts.push(Want::new(key, &run.spot, id, (approval.created_at, 3, 0), item));
                }
            }
            match known.get(ask_key.as_str()) {
                None => {
                    let card = match &settled {
                        Some(said) => place::settle(card, said.clone()),
                        None => card,
                    };
                    let pieces = place::fit(card, abilities);
                    let last = pieces.len() - 1;
                    for (at, card) in pieces.into_iter().enumerate() {
                        let key = if at == last {
                            ask_key.clone()
                        } else {
                            format!("askpart:{approval_id}:{}", at + 1)
                        };
                        if fresh(&key, &posts) && fresh(&key, wants) {
                            posts.push(Want::new(
                                key,
                                &spot,
                                id,
                                (approval.created_at, 3, at + 1),
                                Outgoing::Post { card },
                            ));
                        }
                    }
                    if settled.is_some() && fresh(&settle_key, wants) {
                        wants.push(Want::new(
                            settle_key,
                            &spot,
                            id,
                            (approval.created_at, 3, last + 2),
                            Outgoing::Skip,
                        ));
                    }
                }
                Some(asked) => {
                    let Some(said) = settled.filter(|_| !asked.state.open()) else { continue };
                    if !fresh(&settle_key, wants) {
                        continue;
                    }
                    let edit = match (&asked.item, &asked.posted, asked.state) {
                        (Outgoing::Post { card }, Some(posted), RelayState::Sent)
                            if abilities.edits =>
                        {
                            Some(Outgoing::Edit {
                                posted: posted.clone(),
                                card: place::settle(card.clone(), said.clone()),
                            })
                        }
                        _ => None,
                    };
                    let item = edit.unwrap_or_else(|| Outgoing::Post {
                        card: Card {
                            kind: CardKind::Notice,
                            from: Some(name.clone()),
                            text: said.clone(),
                            fields: Vec::new(),
                            ask: None,
                            choices: Vec::new(),
                            settled: Some(said.clone()),
                        },
                    });
                    let at = approval.decided_at.unwrap_or(now);
                    wants.push(Want::new(settle_key, &asked.spot, id, (at, 4, 0), item));
                }
            }
        }

        if let Some(outcome) = &outcome {
            let ended = place::mark_for(outcome.status);
            let key = format!("mark:{id}:{}", ended.as_str());
            if fresh(&key, wants) {
                wants.push(Want::new(key, &run.spot, id, (outcome.finished_at, 5, 0), mark(ended)));
            }
            if ended == place::Mark::Failed {
                let key = format!("notice:{id}:failed");
                if fresh(&key, &posts) && fresh(&key, wants) {
                    let item = Outgoing::Post { card: place::failed_notice(&outcome.headline) };
                    posts.push(Want::new(key, &run.spot, id, (outcome.finished_at, 6, 0), item));
                }
            }
        }

        let offline = format!("notice:{id}:offline");
        let posted_at = known
            .values()
            .filter(|row| {
                row.run == Some(id)
                    && matches!(row.item, Outgoing::Post { .. })
                    && row.key != offline
            })
            .map(|row| row.created_at)
            .min();
        let in_spot = posts.iter().any(|want| want.spot == run.spot);
        if fresh(&offline, wants)
            && in_spot
            && posted_at.is_none()
            && place::late(run.heard_at, now)
        {
            let text = store.get_message(run.message)?.map(|m| texts(&m.parts)).unwrap_or_default();
            let item = Outgoing::Post { card: place::offline_notice(&text) };
            wants.push(Want::new(offline, &run.spot, id, (run.created_at, 0, 0), item));
        }
        wants.extend(posts);
        Ok(outcome.is_some())
    }
}
