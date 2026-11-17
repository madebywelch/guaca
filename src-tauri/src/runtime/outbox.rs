use std::collections::HashSet;
use std::sync::OnceLock;
use std::time::Duration;

use super::{Runtime, RuntimeError, UiEvent};
use crate::db::{Due, StoreError};
use crate::domain::now_ms;
use crate::domain::outbox::{
    decide, gather, Attention, Delivery, Gathered, Kind, Look, Verdict, Why, EXPIRE_MS,
};
use crate::domain::view::Presence;
use crate::push::{self, Answer, Content, Format, Next, Unreachable};

const TICK: Duration = Duration::from_secs(60);

const REST: Duration = Duration::from_secs(1);

pub const RECLAIM_MS: i64 = 15_000;

impl Runtime {
    pub fn start_dispatcher(&self) {
        let runtime = self.clone();
        let written = self.inner.store.outbox_written();
        self.inner.handle.spawn(async move {
            loop {
                runtime.dispatch_deliveries(now_ms());
                runtime.push_deliveries(now_ms()).await;
                let wait = match runtime.inner.store.next_delivery_at() {
                    Ok(Some(at)) => Duration::from_millis((at - now_ms()).max(0) as u64),
                    Ok(None) => TICK,
                    Err(err) => {
                        tracing::warn!(%err, "could not read when the outbox is next due");
                        TICK
                    }
                };
                tokio::select! {
                    _ = written.notified() => {}
                    _ = tokio::time::sleep(wait.clamp(REST, TICK)) => {}
                }
            }
        });
    }

    pub fn dispatch_deliveries(&self, now: i64) -> usize {
        let store = &self.inner.store;
        if let Err(err) = store.expire_deliveries(now) {
            tracing::warn!(%err, "could not expire unclaimed deliveries");
        }
        let due = match store.due_deliveries(now) {
            Ok(due) => due,
            Err(err) => {
                tracing::warn!(%err, "could not read the outbox; trying again shortly");
                return 0;
            }
        };
        let config = self.config();
        let attention = config.attention;
        let presence = self.inner.clients.lock().presence(now);
        let pushing = (matches!(presence, Presence::Away) && config.delivery.is_on())
            .then_some(config.delivery.content);
        let mut offered = 0;
        let mut closed = HashSet::new();
        for row in due {
            if let Some(batch) = &row.batch_id {
                if closed.insert(batch.clone()) {
                    match self.close_gathering(batch, &attention, &presence, pushing, now) {
                        Ok(more) => offered += more,
                        Err(err) => tracing::warn!(%err, "could not close a gathering"),
                    }
                }
                continue;
            }
            let look = if row.attempts == 0 { Look::New } else { Look::Held };
            let applied =
                match decide(row.kind, row.agent_id, row.ended, look, &attention, &presence) {
                    Verdict::Deliver => {
                        let item = row.gathered();
                        self.offer(&row.id, None, &item, row.kind, pushing, now).map(|sent| {
                            offered += usize::from(sent);
                        })
                    }
                    Verdict::Hold { until } => store.hold_delivery(&row.id, until),
                    Verdict::Gather => store.gather_delivery(&row.id, now).map(drop),
                    Verdict::Suppress(why) => {
                        store.suppress_deliveries(std::slice::from_ref(&row.id), why)
                    }
                };
            if let Err(err) = applied {
                tracing::warn!(%err, id = %row.id, "could not act on a delivery");
            }
        }
        self.settle_releases(&config.delivery, now);
        offered
    }

    fn settle_releases(&self, delivery: &crate::config::DeliveryConfig, now: i64) {
        let store = &self.inner.store;
        let due = match store.due_releases(now) {
            Ok(due) => due,
            Err(err) => {
                tracing::warn!(%err, "could not read the deliveries given back");
                return;
            }
        };
        for (row, count) in due {
            let said = delivery
                .is_on()
                .then(|| push::say(row.kind, &row.gathered(), count, delivery.content));
            let settled = store.settle_release(
                &row.id,
                said.as_ref(),
                count == 1,
                row.created_at + EXPIRE_MS,
                now,
            );
            if let Err(err) = settled {
                tracing::warn!(%err, id = %row.id, "could not settle a delivery given back");
            }
        }
    }

    fn close_gathering(
        &self,
        batch: &str,
        attention: &Attention,
        presence: &Presence,
        pushing: Option<Content>,
        now: i64,
    ) -> Result<usize, StoreError> {
        let store = &self.inner.store;
        let mut kept: Vec<Due> = Vec::new();
        for row in store.gathering(batch)? {
            match decide(row.kind, row.agent_id, row.ended, Look::Closing, attention, presence) {
                Verdict::Suppress(why) => {
                    store.suppress_deliveries(std::slice::from_ref(&row.id), why)?
                }
                _ => kept.push(row),
            }
        }
        let Some(item) = gather(&kept.iter().map(Due::gathered).collect::<Vec<_>>()) else {
            return Ok(0);
        };
        let carried: Vec<String> = kept[1..].iter().map(|row| row.id.clone()).collect();
        let offered = self.offer(&kept[0].id, Some(&carried), &item, kept[0].kind, pushing, now)?;
        Ok(usize::from(offered))
    }

    fn offer(
        &self,
        id: &str,
        carried: Option<&[String]>,
        item: &Gathered,
        kind: Kind,
        pushing: Option<Content>,
        now: i64,
    ) -> Result<bool, StoreError> {
        let count = carried.map_or(1, |carried| carried.len() + 1);
        let said = pushing.map(|content| push::say(kind, item, count, content));
        let store = &self.inner.store;
        let Some(delivery) = store.offer_delivery(id, carried, item, said.as_ref(), now)? else {
            return Ok(false);
        };
        let Delivery { id, kind, title, body, target, created_at } = delivery;
        self.emit(UiEvent::Delivery { id, kind, title, body, target, created_at });
        Ok(true)
    }

    pub fn pending_deliveries(&self, page: Option<&str>) -> Result<Vec<Delivery>, RuntimeError> {
        Ok(self.inner.store.pending_deliveries(page)?)
    }

    pub fn claim_delivery(&self, id: &str, page: Option<&str>) -> Result<bool, RuntimeError> {
        Ok(self.inner.store.claim_delivery(id, page, now_ms())?)
    }

    pub fn release_delivery(&self, id: &str, page: &str) -> Result<bool, RuntimeError> {
        let store = &self.inner.store;
        let Some((given, releases)) = store.release_delivery(id, page, RECLAIM_MS, now_ms())?
        else {
            return Ok(false);
        };
        if releases == 1 {
            let Delivery { id, kind, title, body, target, created_at } = given;
            self.emit(UiEvent::Delivery { id, kind, title, body, target, created_at });
        }
        store.outbox_written().notify_one();
        Ok(true)
    }

    pub async fn push_deliveries(&self, now: i64) -> usize {
        let store = &self.inner.store;
        let due = match store.due_pushes(now) {
            Ok(due) => due,
            Err(err) => {
                tracing::warn!(%err, "could not read the pushes that are due");
                return 0;
            }
        };
        if due.is_empty() {
            return 0;
        }
        let delivery = self.config().delivery;
        let here = !matches!(self.inner.clients.lock().presence(now), Presence::Away);
        let (due, dropped): (Vec<_>, Vec<_>) = due
            .into_iter()
            .partition(|(_, _, given_back)| delivery.is_on() && (!here || *given_back));
        if !dropped.is_empty() {
            let why = if delivery.is_on() { Why::Present } else { Why::Setting };
            let ids: Vec<String> = dropped.into_iter().map(|(push, _, _)| push.id).collect();
            if let Err(err) = store.suppress_deliveries(&ids, why) {
                tracing::warn!(%err, "could not drop the pushes that are due");
            }
        }
        let mut sent = 0;
        for (mut push, attempts, _) in due {
            if delivery.content == Content::Generic {
                push.agent = None;
                push.group = None;
            }
            let request = push::request(
                delivery.format,
                &push,
                Some(&delivery.token),
                Some(&delivery.link_base),
            );
            let next =
                push::next(send(&delivery.url, request).await, attempts + 1, push.created_at, now);
            match &next {
                Next::Sent => sent += 1,
                Next::Again { error, .. } | Next::Failed { error } => {
                    tracing::warn!(id = %push.id, %error, "the push address did not take a push");
                }
            }
            if let Err(err) = store.record_push(&push.id, &next, now) {
                tracing::warn!(%err, id = %push.id, "could not record a push attempt");
            }
        }
        sent
    }

    pub async fn test_push(&self) -> Result<String, String> {
        let delivery = self.config().delivery;
        if !delivery.is_on() {
            return Err("No address is saved yet.".into());
        }
        let test = push::Push {
            id: format!("test-{}", uuid::Uuid::new_v4()),
            kind: Kind::Outcome,
            title: "Guaca".into(),
            body: "This is what a push from Guaca looks like.".into(),
            agent: None,
            group: None,
            status: None,
            created_at: now_ms(),
        };
        let request =
            push::request(delivery.format, &test, Some(&delivery.token), Some(&delivery.link_base));
        match send(&delivery.url, request).await {
            Answer::Accepted if delivery.format == Format::Ntfy => Ok("Sent, and the server took \
                it. If your phone shows nothing, check that the ntfy app is subscribed to this \
                topic."
                .into()),
            Answer::Accepted => Ok("Sent, and the address took it.".into()),
            Answer::Refused { status, .. } => Err(push::refused(status)),
            Answer::Unreachable(why) => Err(push::unreachable(why)),
        }
    }

    pub fn push_report(&self) -> Result<push::Report, RuntimeError> {
        Ok(self.inner.store.push_report()?)
    }
}

fn client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(push::TIMEOUT_SECS))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap_or_default()
    })
}

async fn send(url: &str, request: push::Request) -> Answer {
    let mut post = client().post(url);
    for (name, value) in &request.headers {
        post = post.header(*name, value);
    }
    let response = match post.body(request.body).send().await {
        Ok(response) => response,
        Err(err) if err.is_builder() => return Answer::Unreachable(Unreachable::Invalid),
        Err(err) if err.is_timeout() => return Answer::Unreachable(Unreachable::Timeout),
        Err(err) if err.is_connect() => return Answer::Unreachable(Unreachable::Connect),
        Err(_) => return Answer::Unreachable(Unreachable::Other),
    };
    let status = response.status();
    if status.is_success() {
        return Answer::Accepted;
    }
    let retry_after_ms = response
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<i64>().ok())
        .map(|seconds| seconds.saturating_mul(1000));
    Answer::Refused { status: status.as_u16(), retry_after_ms }
}
