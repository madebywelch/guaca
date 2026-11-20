use std::sync::Arc;

use crate::domain::place::{
    Abilities, AdapterId, Card, Heard, HeardKey, Mark, OutsideUser, PlaceKey, Posted, RelayTag,
    Spot, ThreadKey,
};

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
    Retry { after: Option<std::time::Duration> },
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

type Hear = dyn Fn(Heard) -> Pending<'static, Result<(), Unheard>> + Send + Sync;
type Connect = dyn Fn(Ears) -> Pending<'static, Result<(), AdapterError>> + Send + Sync;

#[derive(Clone)]
pub struct Ears {
    hear: Arc<Hear>,
    connect: Arc<Connect>,
}

impl Ears {
    pub fn new(
        hear: impl Fn(Heard) -> Pending<'static, Result<(), Unheard>> + Send + Sync + 'static,
    ) -> Self {
        Self { hear: Arc::new(hear), connect: Arc::new(|_| Box::pin(async { Ok(()) })) }
    }

    pub fn on_connected(
        mut self,
        connect: impl Fn(Ears) -> Pending<'static, Result<(), AdapterError>> + Send + Sync + 'static,
    ) -> Self {
        self.connect = Arc::new(connect);
        self
    }

    pub async fn connected(&self) -> Result<(), AdapterError> {
        (self.connect)(self.clone()).await
    }

    pub async fn heard(&self, heard: Heard) -> Result<(), Unheard> {
        (self.hear)(heard).await
    }
}
pub mod slack;
