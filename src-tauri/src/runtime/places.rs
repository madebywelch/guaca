use std::sync::atomic::Ordering;
use std::sync::Arc;

use parking_lot::Mutex;
use sha2::{Digest, Sha256};
use tokio::task::AbortHandle;

use super::relay::backoff;
use super::{Runtime, RuntimeError};
use crate::db::{Landing, NewRelay, RelayState, StoreError};
use crate::domain::approval::{ApprovalState, Request};
use crate::domain::envelope::{Part, Trust};
use crate::domain::ids::{AgentId, ApprovalId};
use crate::domain::now_ms;
use crate::domain::place::{self, AdapterId, Heard, Outgoing, Said};
use crate::places::{Adapter, AdapterStatus, Ears, Unheard};

pub(super) struct Attached {
    adapter: Arc<dyn Adapter>,
    status: Arc<Mutex<AdapterStatus>>,
    connection: AbortHandle,
    relay: Option<AbortHandle>,
}

impl Drop for Attached {
    fn drop(&mut self) {
        self.connection.abort();
        if let Some(relay) = &self.relay {
            relay.abort();
        }
    }
}

pub(super) struct LinkCode {
    code: String,
    agent: AgentId,
    expires_at: i64,
}

fn unheard(err: impl std::fmt::Display) -> Unheard {
    tracing::warn!(%err, "a message from outside could not be recorded; the provider will deliver it again");
    Unheard { reason: "Guaca could not record that message yet.".into() }
}

impl Runtime {
    pub(super) fn place_gate(&self, adapter: &AdapterId) -> Arc<tokio::sync::RwLock<()>> {
        self.inner.place_gates.lock().entry(adapter.clone()).or_default().clone()
    }

    pub fn attach_adapter(&self, adapter: Arc<dyn Adapter>) {
        let id = adapter.id();
        let mut adapters = self.inner.adapters.lock();
        drop(adapters.remove(&id));
        let status = Arc::new(Mutex::new(AdapterStatus::default()));
        let holding = self.inner.handle.spawn(self.clone().hold(adapter.clone(), status.clone()));
        let mut attached =
            Attached { adapter, status, connection: holding.abort_handle(), relay: None };
        if self.inner.relay_started.load(Ordering::Acquire) {
            self.start_relay(&mut attached);
        }
        adapters.insert(id, attached);
    }

    pub(super) fn start_relays(&self) {
        let mut adapters = self.inner.adapters.lock();
        self.inner.relay_started.store(true, Ordering::Release);
        for attached in adapters.values_mut() {
            self.start_relay(attached);
        }
    }

    fn start_relay(&self, attached: &mut Attached) {
        if attached.relay.is_none() {
            let task = self.clone().relay(attached.adapter.clone(), attached.status.clone());
            attached.relay = Some(self.inner.handle.spawn(task).abort_handle());
        }
    }

    pub fn detach_adapter(&self, id: &AdapterId) -> bool {
        self.inner.adapters.lock().remove(id).is_some()
    }

    pub fn adapter_status(&self, id: &AdapterId) -> Option<AdapterStatus> {
        let mut status = self.inner.adapters.lock().get(id)?.status.lock().clone();
        status.last_relay_error = self.inner.store.last_relay_error(id).ok().flatten();
        Some(status)
    }

    pub fn link_code(&self, adapter: &AdapterId, agent: AgentId) -> Result<String, RuntimeError> {
        self.inner.store.get_agent(agent)?.ok_or(RuntimeError::UnknownAgent(agent))?;
        let drawn = uuid::Uuid::new_v4().as_u128() % 1_000_000;
        let code = format!("{drawn:06}");
        self.inner.link_codes.lock().insert(
            adapter.clone(),
            LinkCode { code: code.clone(), agent, expires_at: now_ms() + place::LINK_CODE_MS },
        );
        Ok(code)
    }

    async fn hold(self, adapter: Arc<dyn Adapter>, status: Arc<Mutex<AdapterStatus>>) {
        let mut failures = 0u32;
        loop {
            let ears = Ears::new(self.clone(), adapter.clone(), status.clone());
            let ran = adapter.run(ears).await;
            let connected = status.lock().connected_since.take();
            if connected.is_some_and(|began| now_ms().saturating_sub(began) >= 60_000) {
                failures = 0;
            }
            match ran {
                Ok(()) => failures = 0,
                Err(err) => {
                    let mut now = status.lock();
                    now.last_error = Some(err.reason);
                    if !err.retry {
                        now.stopped = true;
                        self.wake_relay();
                        return;
                    }
                    failures += 1;
                }
            }
            tokio::time::sleep(backoff(failures.max(1))).await;
        }
    }

    async fn tell(&self, adapter: &Arc<dyn Adapter>, heard: &Heard, text: &str) {
        if let Err(err) = adapter.tell(&heard.spot, &heard.speaker, text).await {
            tracing::warn!(?err, "could not tell a person outside why their message was not taken");
        }
    }

    fn use_code(
        &self,
        adapter: &AdapterId,
        heard: &Heard,
        said: &str,
    ) -> Result<Option<String>, StoreError> {
        let mut codes = self.inner.link_codes.lock();
        if self.inner.store.link_was_heard(adapter, &heard.key)? {
            return Ok(Some(String::new()));
        }
        let Some(code) = codes.get(adapter).filter(|code| code.code == said) else {
            return Ok(None);
        };
        if heard.at_ms < code.expires_at - place::LINK_CODE_MS {
            return Ok(Some(String::new()));
        }
        if code.expires_at <= now_ms() {
            return Ok(Some(
                "This link code expired. Ask for a new one in Guaca's Settings.".into(),
            ));
        }
        if !heard.direct {
            return Ok(Some(place::CODE_IN_CHANNEL.into()));
        }
        let store = &self.inner.store;
        let Some(card) =
            store.get_agent(code.agent)?.filter(|card| card.lifecycle.is_discoverable())
        else {
            return Ok(Some("This agent is unavailable. Pick another in Guaca's Settings.".into()));
        };
        store.link_person(adapter, heard, card.group_id, card.id)?;
        codes.remove(adapter);
        Ok(Some(place::linked(&card.name)))
    }

    pub(crate) async fn hear(
        &self,
        adapter: &Arc<dyn Adapter>,
        heard: Heard,
        history: bool,
    ) -> Result<(), Unheard> {
        let id = adapter.id();
        let store = &self.inner.store;
        let gate = self.place_gate(&id);
        let pairing = match &heard.said {
            Said::Text { text }
                if text.len() == 6 && text.bytes().all(|byte| byte.is_ascii_digit()) =>
            {
                Some(text)
            }
            _ => None,
        };
        if let Some(text) = pairing {
            if history {
                return Ok(());
            }
            let guard = gate.write().await;
            if let Some(line) = self.use_code(&id, &heard, text).map_err(unheard)? {
                drop(guard);
                self.wake_relay();
                if !line.is_empty() {
                    self.tell(adapter, &heard, &line).await;
                }
                return Ok(());
            }
        }
        let _guard = gate.read().await;
        let linked = store.linked(&id).map_err(unheard)?;
        if history
            && !linked.as_ref().is_some_and(|linked| {
                heard.at_ms >= linked.since && (!heard.direct || heard.spot.place == linked.place)
            })
        {
            return Ok(());
        }
        let Some(trust) = place::trust(linked.as_ref().map(|it| &it.person), &heard.speaker) else {
            if history {
                return Ok(());
            }
            self.tell(adapter, &heard, place::STRANGER).await;
            return Ok(());
        };
        if heard.direct && !linked.as_ref().is_some_and(|linked| linked.place == heard.spot.place) {
            self.tell(adapter, &heard, place::UNBOUND).await;
            return Ok(());
        }
        if history
            && !store
                .bound(&id, &heard.spot.place)
                .map_err(unheard)?
                .is_some_and(|bound| bound.direct == heard.direct && heard.at_ms >= bound.since)
        {
            return Ok(());
        }
        match &heard.said {
            Said::Choice { .. } if history => Ok(()),
            Said::Choice { ask, choice } => self.press(adapter, &heard, ask, choice).await,
            Said::Text { text } => self.take_text(adapter, &heard, text, trust).await,
        }
    }

    async fn press(
        &self,
        adapter: &Arc<dyn Adapter>,
        heard: &Heard,
        ask: &str,
        choice: &str,
    ) -> Result<(), Unheard> {
        let store = &self.inner.store;
        let approval = match ask.parse::<ApprovalId>() {
            Ok(id) => store.get_approval(id).map_err(unheard)?,
            Err(_) => None,
        };
        let id = adapter.id();
        let posted = store.relay_row(&format!("ask:{ask}")).map_err(unheard)?;
        let target = posted.filter(|row| row.adapter == id && row.spot == heard.spot);
        let bound = store.bound(&id, &heard.spot.place).map_err(unheard)?;
        let Some((approval, target)) = approval.zip(target).filter(|(approval, row)| {
            row.run == Some(approval.run_id)
                && bound.as_ref().is_some_and(|bound| bound.direct == heard.direct)
                && matches!(&row.item, Outgoing::Post { card } if card.choices.iter().any(|c| c.id == choice))
        }) else {
            self.tell(adapter, heard, place::NOT_FROM_HERE).await;
            return Ok(());
        };
        if matches!(approval.request, Request::Permission { .. })
            && (!heard.direct
                || !store
                    .linked(&id)
                    .map_err(unheard)?
                    .is_some_and(|linked| linked.place == heard.spot.place))
        {
            self.tell(adapter, heard, place::NOT_FROM_HERE).await;
            return Ok(());
        }
        if target.state == RelayState::Sending {
            return Err(Unheard { reason: "Guaca has not confirmed that post yet.".into() });
        }
        if target.state != RelayState::Sent || target.posted.is_none() {
            self.tell(adapter, heard, place::NOT_FROM_HERE).await;
            return Ok(());
        }
        let answer = match &approval.request {
            Request::Permission { .. } => place::decision_for(choice).map(|_| choice.to_string()),
            Request::Question { options } => place::option_for(choice, options).map(str::to_string),
        };
        if let Some(answer) = answer {
            self.give(approval.id, &answer).map_err(unheard)?;
        }
        self.wake_relay();
        Ok(())
    }

    fn give(&self, id: ApprovalId, answer: &str) -> Result<(), StoreError> {
        let Some(approval) = self.inner.store.get_approval(id)? else { return Ok(()) };
        if approval.state != ApprovalState::Pending {
            return Ok(());
        }
        let given = match approval.request {
            Request::Permission { .. } => match place::decision_for(answer) {
                Some(decision) => self.decide_approval(id, decision),
                None => return Ok(()),
            },
            Request::Question { .. } => self.answer_question(id, answer),
        };
        match given {
            Ok(_) | Err(RuntimeError::Store(StoreError::ApprovalSettled { .. })) => Ok(()),
            Err(RuntimeError::Store(err)) => Err(err),
            Err(err) => {
                tracing::warn!(%err, "an answer from outside was not taken");
                Ok(())
            }
        }
    }

    fn reading(
        &self,
        ask: ApprovalId,
        text: &str,
        no_buttons: bool,
        heard_at: i64,
    ) -> Result<Option<String>, StoreError> {
        let Some(approval) = self.inner.store.get_approval(ask)? else { return Ok(None) };
        if approval.state != ApprovalState::Pending || heard_at < approval.created_at {
            return Ok(None);
        }
        Ok(match &approval.request {
            Request::Question { options } => {
                let card = place::question("", "", "", options);
                let chosen = no_buttons
                    .then(|| place::read_choice(text, &card.choices))
                    .flatten()
                    .map(|choice| choice.label.clone());
                Some(chosen.unwrap_or_else(|| text.trim().to_string())).filter(|it| !it.is_empty())
            }
            Request::Permission { action } if no_buttons => {
                let card = place::permission("", "", "", Vec::new(), *action);
                place::read_choice(text, &card.choices).map(|choice| choice.id.clone())
            }
            Request::Permission { .. } => None,
        })
    }

    async fn take_text(
        &self,
        adapter: &Arc<dyn Adapter>,
        heard: &Heard,
        text: &str,
        trust: Trust,
    ) -> Result<(), Unheard> {
        let id = adapter.id();
        let store = &self.inner.store;
        let key = format!("outside:{}:{}", id.0, heard.key.0);
        let stale_key = format!("notice:{}/{}:stale", id.0, heard.key.0);
        if store.heard_before(&key).map_err(unheard)?
            || store.heard_before(&stale_key).map_err(unheard)?
        {
            return Ok(());
        }
        if let Some((ask, answer)) = store.place_answer(&id, &heard.key).map_err(unheard)? {
            return self.give(ask, &answer).map_err(unheard);
        }
        let Some(bound) = store
            .bound(&id, &heard.spot.place)
            .map_err(unheard)?
            .filter(|bound| bound.direct == heard.direct)
        else {
            self.tell(adapter, heard, place::UNBOUND).await;
            return Ok(());
        };
        if place::stale(heard.at_ms, now_ms()) {
            let item = Outgoing::Post { card: place::stale_notice(text) };
            let notice = NewRelay {
                key: &stale_key,
                adapter: &id,
                spot: &heard.spot,
                run: None,
                state: RelayState::Queued,
                item: &item,
            };
            store.queue_place_notice(&notice, &heard.key).map_err(unheard)?;
            self.wake_relay();
            return Ok(());
        }
        if let Some((ask, no_buttons)) = store.latest_ask(&id, &heard.spot).map_err(unheard)? {
            if let Some(answer) =
                self.reading(ask, text, no_buttons, heard.at_ms).map_err(unheard)?
            {
                store
                    .record_place_answer(&id, &heard.spot.place, &heard.key, ask, &answer)
                    .map_err(unheard)?;
                return self.give(ask, &answer).map_err(unheard);
            }
        }
        let (agent, body) = match store.thread_agent(&id, &heard.spot).map_err(unheard)? {
            Some(agent) => (agent, text.trim().to_string()),
            None => {
                let crew: Vec<_> = store
                    .list_agents()
                    .map_err(unheard)?
                    .into_iter()
                    .filter(|card| card.group_id == bound.group)
                    .collect();
                place::route(text, &crew, bound.agent)
            }
        };
        if !store
            .get_agent(agent)
            .map_err(unheard)?
            .is_some_and(|card| card.group_id == bound.group && card.lifecycle.is_discoverable())
        {
            self.tell(adapter, heard, place::AGENT_UNAVAILABLE).await;
            return store.raise_high_water(&id, &heard.spot.place, &heard.key).map_err(unheard);
        }
        if body.is_empty() {
            return store.raise_high_water(&id, &heard.spot.place, &heard.key).map_err(unheard);
        }
        let landing = Landing {
            adapter: id.clone(),
            spot: heard.spot.clone(),
            heard: heard.key.clone(),
            heard_at: heard.at_ms,
            direct: heard.direct,
            agent,
        };
        let fingerprint = format!("{:x}", Sha256::digest(body.as_bytes()));
        let parts = vec![Part::Text { text: body }];
        match self.send_keyed(agent, parts, &key, &fingerprint, trust, Some(&landing)) {
            Ok(_) => {
                self.wake_relay();
                Ok(())
            }
            Err(RuntimeError::Store(err @ (StoreError::Sqlite(_) | StoreError::Pool(_)))) => {
                Err(unheard(err))
            }
            Err(RuntimeError::UnknownAgent(_) | RuntimeError::AgentTerminated(_)) => {
                self.tell(adapter, heard, place::AGENT_UNAVAILABLE).await;
                store.raise_high_water(&id, &heard.spot.place, &heard.key).map_err(unheard)
            }
            Err(err) => Err(unheard(err)),
        }
    }
}
