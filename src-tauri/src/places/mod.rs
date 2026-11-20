use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use serde::Serialize;

use crate::domain::now_ms;
use crate::domain::place::{
    Abilities, AdapterId, Card, Heard, HeardKey, Mark, OutsideUser, PlaceKey, Posted, RelayTag,
    Spot, ThreadKey, STALE_MS,
};
use crate::runtime::Runtime;

pub type Pending<'a, T> = futures_util::future::BoxFuture<'a, T>;

pub trait Adapter: Send + Sync + 'static {
    fn id(&self) -> AdapterId;
    fn abilities(&self) -> Abilities;
    fn run<'a>(&'a self, ears: Ears) -> Pending<'a, Result<(), AdapterError>>;
    fn catch_up<'a>(
        &'a self,
        place: &'a PlaceKey,
        recent_threads: &'a [ThreadKey],
        since_ms: i64,
        ears: &'a Ears,
    ) -> Pending<'a, Result<(), AdapterError>>;
    fn post<'a>(
        &'a self,
        to: &'a Spot,
        card: &'a Card,
        tag: &'a RelayTag,
    ) -> Pending<'a, Result<Posted, SendError>>;
    fn edit<'a>(
        &'a self,
        to: &'a Spot,
        posted: &'a Posted,
        card: &'a Card,
    ) -> Pending<'a, Result<(), SendError>>;
    fn find<'a>(
        &'a self,
        to: &'a Spot,
        tag: &'a RelayTag,
    ) -> Pending<'a, Result<Option<Posted>, SendError>>;
    fn mark<'a>(
        &'a self,
        heard: &'a HeardKey,
        to: &'a Spot,
        mark: Mark,
    ) -> Pending<'a, Result<(), SendError>>;
    fn tell<'a>(
        &'a self,
        to: &'a Spot,
        who: &'a OutsideUser,
        text: &'a str,
    ) -> Pending<'a, Result<(), SendError>>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendError {
    Retry { after: Option<Duration> },
    Refused { reason: String },
    Gone { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdapterError {
    pub reason: String,
    pub retry: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unheard {
    pub reason: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdapterStatus {
    pub connected_since: Option<i64>,
    pub last_error: Option<String>,
    pub last_relay_error: Option<String>,
    pub stopped: bool,
}

#[derive(Clone)]
pub struct Ears {
    runtime: Runtime,
    adapter: Arc<dyn Adapter>,
    status: Arc<Mutex<AdapterStatus>>,
}

impl Ears {
    pub(crate) fn new(
        runtime: Runtime,
        adapter: Arc<dyn Adapter>,
        status: Arc<Mutex<AdapterStatus>>,
    ) -> Self {
        Self { runtime, adapter, status }
    }

    pub async fn connected(&self) -> Result<(), AdapterError> {
        let now = now_ms();
        self.status.lock().connected_since = Some(now);
        let since = now - STALE_MS;
        let places =
            self.runtime.store().catch_up_places(&self.adapter.id(), since).map_err(|_| {
                AdapterError {
                    reason: "Guaca could not read the places to catch up on.".into(),
                    retry: true,
                }
            })?;
        for (place, threads) in places {
            self.adapter.catch_up(&place, &threads, since, self).await?;
        }
        Ok(())
    }

    pub async fn heard(&self, heard: Heard) -> Result<(), Unheard> {
        self.runtime.hear(&self.adapter, heard).await
    }
}
