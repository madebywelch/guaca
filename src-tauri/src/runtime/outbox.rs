//! The dispatcher: the one place on the Host that decides what reaches a
//! person and offers it. See `domain/outbox.rs` for the rule and
//! `db/outbox.rs` for the rows.
//!
//! It offers rather than sends. A window learns of a row from the
//! `delivery` event, or from `pending_deliveries` when it connects, and shows
//! it only if its `claim_delivery` wins: one banner across every window,
//! however many are open, and none lost because none was open when it was
//! offered.

use std::collections::HashSet;
use std::time::Duration;

use super::{Runtime, RuntimeError, UiEvent};
use crate::db::{Due, StoreError};
use crate::domain::now_ms;
use crate::domain::outbox::{decide, gather, Attention, Delivery, Gathered, Look, Verdict};
use crate::domain::view::Presence;

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
        let attention = self.config().attention;
        let presence = self.inner.clients.lock().presence(now);
        let mut offered = 0;
        let mut closed = HashSet::new();
        for row in due {
            // A row in a gathering is only ever due because the gathering is
            // over, and then all of it is decided at once.
            if let Some(batch) = &row.batch_id {
                if closed.insert(batch.clone()) {
                    match self.close_gathering(batch, &attention, &presence, now) {
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
                        self.offer(&row.id, None, &row.gathered(), now).map(|sent| {
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
        Ok(usize::from(self.offer(&kept[0].id, Some(&carried), &item, now)?))
    }

    /// Offers one item to every window. False when it was answered or seen in
    /// the moment since it was read, and so not offered.
    fn offer(
        &self,
        id: &str,
        carried: Option<&[String]>,
        item: &Gathered,
        now: i64,
    ) -> Result<bool, StoreError> {
        let Some(delivery) = self.inner.store.offer_delivery(id, carried, item, now)? else {
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
}
