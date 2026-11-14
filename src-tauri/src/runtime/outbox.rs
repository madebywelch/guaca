//! The dispatcher: the one place on the Host that decides what reaches a
//! person and offers it. See `domain/outbox.rs` for the rule and
//! `db/outbox.rs` for the rows.
//!
//! It offers rather than sends. A window learns of a row from the
//! `delivery` event, or from `pending_deliveries` when it connects, and shows
//! it only if its `claim_delivery` wins: one banner across every window,
//! however many are open, and none lost because none was open when it was
//! offered.
//!
//! The one thing it sends is a push. While the operator is away, an offer is
//! made with a second row beside it for the address in `delivery`, if there
//! is one, and `push_deliveries` sends those: a POST each, tried again on a
//! schedule, and recorded. What it is told back is turned into words from the
//! status code alone, since a client's own error names the address.

use std::collections::HashSet;
use std::sync::OnceLock;
use std::time::Duration;

use super::{Runtime, RuntimeError, UiEvent};
use crate::db::{Due, StoreError};
use crate::domain::now_ms;
use crate::domain::outbox::{
    decide, gather, Attention, Delivery, Gathered, Kind, Look, Verdict, Why,
};
use crate::domain::view::Presence;
use crate::push::{self, Answer, Content, Format, Next, Unreachable};

/// The longest the dispatcher sleeps with nothing due, which is how often a
/// row nobody claimed is looked at to see whether it has expired.
const TICK: Duration = Duration::from_secs(60);

/// The shortest it sleeps, so a row that cannot be read cannot spin it.
const REST: Duration = Duration::from_secs(1);

impl Runtime {
    /// Starts the dispatcher. Once, at boot, after the drain's work is picked
    /// up, so what recovery wrote is the first thing it reads.
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

    /// One pass: lets go of what nobody claimed in a day, then decides every
    /// row whose time has come. Returns how many items it offered.
    ///
    /// Public so a suite can step time rather than wait for it.
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
        // Only while nobody is at a window, and only to an address the
        // operator gave: anyone at a window is shown it there.
        let pushing = (matches!(presence, Presence::Away) && config.delivery.is_on())
            .then_some(config.delivery.content);
        let mut offered = 0;
        let mut closed = HashSet::new();
        for row in due {
            // A row in a gathering is only ever due because the gathering is
            // over, and then all of it is decided at once.
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
        offered
    }

    /// Decides a gathering that is over: what is still worth saying is offered
    /// as one item, carried by its oldest row.
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

    /// Offers one item to every window, and with `pushing` writes the push
    /// that goes with it, saying as much as the operator allows. False when it
    /// was answered or seen in the moment since it was read, and so not
    /// offered.
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

    /// Everything offered and not yet claimed, for a window that has just
    /// connected and heard none of the offers.
    pub fn pending_deliveries(&self) -> Result<Vec<Delivery>, RuntimeError> {
        Ok(self.inner.store.pending_deliveries()?)
    }

    /// True for the one window that may show this, and false for every other.
    pub fn claim_delivery(&self, id: &str) -> Result<bool, RuntimeError> {
        Ok(self.inner.store.claim_delivery(id, now_ms())?)
    }

    /// One pass over the address: sends every push whose time has come, and
    /// records how each attempt ended. Returns how many the address took.
    ///
    /// Public so a suite can step time rather than wait for it.
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
        // A push waiting to be tried again is dropped once there is no address
        // to try, and once the operator is back: a window shows them what is
        // waiting, and nothing goes to a phone while a Guaca window has focus.
        let dropped = match (delivery.is_on(), here) {
            (false, _) => Some(Why::Setting),
            (true, true) => Some(Why::Present),
            (true, false) => None,
        };
        if let Some(why) = dropped {
            let ids: Vec<String> = due.into_iter().map(|(push, _)| push.id).collect();
            if let Err(err) = store.suppress_deliveries(&ids, why) {
                tracing::warn!(%err, "could not drop the pushes that are due");
            }
            return 0;
        }
        let mut sent = 0;
        for (mut push, attempts) in due {
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
            let next = push::next(send(&delivery.url, request).await, attempts + 1, push.created_at, now);
            match &next {
                Next::Sent => sent += 1,
                // The words are the status's, never the address's.
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

    /// Sends one push to the saved address now, whatever the operator's
    /// presence, since the button that asks for it is pressed at a window.
    /// Writes no row: it says what came back, and that is all it is for.
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

    /// When a push last went out, and how the last attempt ended.
    pub fn push_report(&self) -> Result<push::Report, RuntimeError> {
        Ok(self.inner.store.push_report()?)
    }
}

/// One client for every push: no redirects, because following one sends the
/// token wherever it points, and a timeout, because the dispatcher waits.
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

/// One POST, and what it got. The error a client formats names the address,
/// so only its kind is kept.
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
