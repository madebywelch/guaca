mod api;
mod blocks;
mod events;
pub mod manifest;
pub mod mrkdwn;

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use parking_lot::Mutex;
use serde_json::{json, Value};
use tokio_tungstenite::tungstenite::Message;

use self::api::{Api, Body, Failure, Token};
use crate::domain::place::{
    Abilities, AdapterId, Buttons, Card, HeardKey, Mark, OutsideUser, PlaceKey, PostSafety, Posted,
    RelayTag, Spot,
};
use crate::places::{Adapter, AdapterError, Ears, Pending, SendError};
use crate::secrets::Secret;

pub const ORIGIN: &str = "https://slack.com";
pub const ORIGIN_VARIABLE: &str = "GUACA_SLACK_ORIGIN";
pub const RELAY_EVENT: &str = "guaca_relay";

const PAGE: usize = 200;
const FIND_PAGE: usize = 100;
const PAGES: usize = 50;
const LIMITED_TRIES: usize = 6;
const LIMITED_WAIT: Duration = Duration::from_secs(1);
const QUIET: Duration = Duration::from_secs(30);
const RECONNECT_PAUSE: Duration = Duration::from_secs(1);

pub struct SlackAdapter {
    api: Api,
    bot_user: Mutex<Option<String>>,
    quiet: Duration,
}

enum Ended {
    Asked,
    Dropped,
}

pub fn origin(overridden: Option<String>) -> String {
    match overridden {
        Some(origin) if !origin.trim().is_empty() => {
            tracing::warn!("using an overridden Slack origin");
            origin.trim().trim_end_matches('/').to_string()
        }
        _ => ORIGIN.to_string(),
    }
}

fn unready() -> AdapterError {
    AdapterError {
        reason: "The connection to Slack closed before it was ready. Guaca will try again."
            .to_string(),
        retry: true,
    }
}

fn socket_off() -> AdapterError {
    AdapterError {
        reason: "Socket Mode is turned off for this Slack app. Turn it on under Socket Mode in the app's settings, then reconnect."
            .to_string(),
        retry: false,
    }
}

fn too_much() -> AdapterError {
    AdapterError {
        reason: "Slack returned incomplete history. Guaca did not deliver any of it. Check Slack for messages that still need an answer.".into(),
        retry: false,
    }
}

fn not_slack() -> AdapterError {
    AdapterError {
        reason: "That place is not a Slack conversation, so Guaca cannot read it.".to_string(),
        retry: false,
    }
}

fn unrecorded() -> AdapterError {
    AdapterError {
        reason:
            "Guaca could not record a message from Slack. It reads it again on the next connect."
                .to_string(),
        retry: true,
    }
}

fn unknown_post() -> SendError {
    SendError::Refused { reason: "That Slack message reference is not one Guaca made.".to_string() }
}

fn lookup_incomplete() -> SendError {
    SendError::Refused {
        reason: "Slack did not return enough history to verify the post. Check the conversation before sending it again.".into(),
    }
}

fn channel_of(to: &Spot) -> Result<&str, SendError> {
    events::split_place(&to.place).map(|(_, channel)| channel).ok_or_else(|| SendError::Refused {
        reason: "That place is not a Slack conversation.".to_string(),
    })
}

fn emoji(mark: Mark) -> &'static str {
    match mark {
        Mark::Working => "eyes",
        Mark::Done => "white_check_mark",
        Mark::NeedsYou => "raised_hand",
        Mark::Failed => "warning",
    }
}

fn tagged(message: &Value, tag: &RelayTag) -> bool {
    message["metadata"]["event_type"].as_str() == Some(RELAY_EVENT)
        && message["metadata"]["event_payload"]["tag"].as_str() == Some(tag.0.as_str())
}

impl SlackAdapter {
    pub fn new(bot: Secret, app: Secret) -> Self {
        Self::at(bot, app, origin(std::env::var(ORIGIN_VARIABLE).ok()))
    }

    pub fn at(bot: Secret, app: Secret, origin: impl Into<String>) -> Self {
        Self { api: Api::new(origin.into(), bot, app), bot_user: Mutex::new(None), quiet: QUIET }
    }

    pub fn with_quiet(mut self, quiet: Duration) -> Self {
        self.quiet = quiet;
        self
    }

    async fn connection(&self, ears: &Ears) -> Result<Ended, AdapterError> {
        let opened = self
            .api
            .call("apps.connections.open", Token::App, Body::Form(vec![]))
            .await
            .map_err(Failure::into_adapter)?;
        let url = opened["url"].as_str().ok_or_else(|| Failure::Garbled.into_adapter())?;
        let (mut socket, _) = tokio_tungstenite::connect_async(url).await.map_err(|_| unready())?;
        let mut ready = false;
        let mut pinged = false;
        loop {
            let frame = match tokio::time::timeout(self.quiet, socket.next()).await {
                Err(_) if pinged => {
                    tracing::warn!("Slack stopped answering on the socket");
                    break;
                }
                Err(_) => {
                    pinged = true;
                    if socket.send(Message::Ping(Default::default())).await.is_err() {
                        break;
                    }
                    continue;
                }
                Ok(Some(Ok(frame))) => frame,
                Ok(_) => break,
            };
            pinged = false;
            let Message::Text(text) = frame else { continue };
            let Ok(envelope) = serde_json::from_str::<Value>(text.as_str()) else { continue };
            match envelope["type"].as_str() {
                Some("hello") => ready = true,
                Some("disconnect") if envelope["reason"].as_str() == Some("link_disabled") => {
                    return Err(socket_off());
                }
                Some("disconnect") => return Ok(Ended::Asked),
                _ => {
                    let Some(id) = envelope["envelope_id"].as_str() else { continue };
                    if !self.take(&envelope, ears).await {
                        tracing::warn!(
                            "Guaca did not record a Slack event, so it is not acknowledged and Slack delivers it again"
                        );
                        continue;
                    }
                    let ack = json!({ "envelope_id": id }).to_string();
                    if socket.send(Message::text(ack)).await.is_err() {
                        break;
                    }
                }
            }
        }
        if ready {
            Ok(Ended::Dropped)
        } else {
            Err(unready())
        }
    }

    async fn take(&self, envelope: &Value, ears: &Ears) -> bool {
        let payload = &envelope["payload"];
        let heard = match (envelope["type"].as_str(), payload["type"].as_str()) {
            (Some("events_api"), _) => {
                if let Some(user) = payload["authorizations"][0]["user_id"].as_str() {
                    *self.bot_user.lock() = Some(user.to_string());
                }
                let bot = self.bot_user.lock().clone();
                let team = payload["team_id"].as_str().unwrap_or_default();
                events::from_event(team, &payload["event"], bot.as_deref()).into_iter().collect()
            }
            (Some("interactive"), Some("block_actions")) => events::from_press(payload),
            _ => Vec::new(),
        };
        for heard in heard {
            if ears.heard(heard).await.is_err() {
                return false;
            }
        }
        true
    }

    async fn bot_user(&self) -> Result<String, Failure> {
        if let Some(user) = self.bot_user.lock().clone() {
            return Ok(user);
        }
        let found = self.api.call("auth.test", Token::Bot, Body::Form(vec![])).await?;
        let user = found["user_id"].as_str().ok_or(Failure::Garbled)?.to_string();
        *self.bot_user.lock() = Some(user.clone());
        Ok(user)
    }

    async fn patient(
        &self,
        method: &str,
        form: Vec<(&'static str, String)>,
    ) -> Result<Value, Failure> {
        let mut last = Failure::Limited(None);
        for attempt in 1..=LIMITED_TRIES {
            match self.api.call(method, Token::Bot, Body::Form(form.clone())).await {
                Err(Failure::Limited(after)) => {
                    last = Failure::Limited(after);
                    if attempt < LIMITED_TRIES {
                        tokio::time::sleep(after.unwrap_or(LIMITED_WAIT)).await;
                    }
                }
                other => return other,
            }
        }
        Err(last)
    }

    async fn pages(
        &self,
        method: &str,
        form: Vec<(&'static str, String)>,
        patient: bool,
    ) -> Result<(Vec<Value>, bool), Failure> {
        let mut messages = Vec::new();
        let mut cursor: Option<String> = None;
        for _ in 0..PAGES {
            let mut page_form = form.clone();
            if let Some(cursor) = cursor.take() {
                page_form.push(("cursor", cursor));
            }
            let page = if patient {
                self.patient(method, page_form).await?
            } else {
                self.api.call(method, Token::Bot, Body::Form(page_form)).await?
            };
            messages.extend(page["messages"].as_array().ok_or(Failure::Garbled)?.iter().cloned());
            cursor = page["response_metadata"]["next_cursor"]
                .as_str()
                .filter(|next| !next.is_empty())
                .map(str::to_string);
            if cursor.is_none() {
                return Ok((messages, page["has_more"].as_bool() != Some(true)));
            }
        }
        Ok((messages, false))
    }

    async fn react(
        &self,
        method: &str,
        channel: &str,
        ts: &str,
        name: &str,
        fine: &str,
    ) -> Result<(), SendError> {
        let body = json!({ "channel": channel, "timestamp": ts, "name": name });
        match self.api.call(method, Token::Bot, Body::Json(body)).await {
            Ok(_) => Ok(()),
            Err(failure) if failure.is(fine) => Ok(()),
            Err(failure) => Err(failure.into_send()),
        }
    }
}

impl Adapter for SlackAdapter {
    fn id(&self) -> AdapterId {
        AdapterId("slack".to_string())
    }

    fn abilities(&self) -> Abilities {
        Abilities {
            buttons: Some(Buttons { max: blocks::BUTTONS, label_chars: blocks::LABEL_CHARS }),
            edits: true,
            reactions: true,
            max_text: blocks::SECTION_CHARS,
            posts: PostSafety::Findable,
        }
    }

    fn run<'a>(&'a self, ears: Ears) -> Pending<'a, Result<(), AdapterError>> {
        Box::pin(async move {
            loop {
                match self.connection(&ears).await? {
                    Ended::Asked => tracing::info!("Slack asked for a new connection"),
                    Ended::Dropped => {
                        tracing::warn!("the connection to Slack dropped, reconnecting");
                        tokio::time::sleep(RECONNECT_PAUSE).await;
                    }
                }
            }
        })
    }

    fn catch_up<'a>(
        &'a self,
        place: &'a PlaceKey,
        after: Option<&'a HeardKey>,
        ears: &'a Ears,
    ) -> Pending<'a, Result<(), AdapterError>> {
        Box::pin(async move {
            let (team, channel) = events::split_place(place).ok_or_else(not_slack)?;
            let oldest =
                after.and_then(|key| events::split_ref(&key.0)).map(|(_, ts)| ts.to_string());
            let bot = self.bot_user().await.map_err(Failure::into_adapter)?;
            let mut form = vec![("channel", channel.to_string()), ("limit", PAGE.to_string())];
            if let Some(oldest) = &oldest {
                form.push(("oldest", oldest.clone()));
                form.push(("inclusive", "false".to_string()));
            }
            let (mut messages, whole) = self
                .pages("conversations.history", form.clone(), true)
                .await
                .map_err(Failure::into_adapter)?;
            if !whole {
                return Err(too_much());
            }
            let parents: Vec<String> = messages
                .iter()
                .filter(|message| message["reply_count"].as_u64().unwrap_or_default() > 0)
                .filter_map(|message| message["ts"].as_str().map(str::to_string))
                .collect();
            for parent in parents {
                let mut replies_form = form.clone();
                replies_form.push(("ts", parent.clone()));
                let (replies, whole) = self
                    .pages("conversations.replies", replies_form, true)
                    .await
                    .map_err(Failure::into_adapter)?;
                if !whole {
                    return Err(too_much());
                }
                messages.extend(
                    replies
                        .into_iter()
                        .filter(|reply| reply["ts"].as_str() != Some(parent.as_str())),
                );
            }
            let floor = oldest.as_deref().map(events::ts_order);
            let mut heard: Vec<_> = messages
                .iter()
                .filter(|message| {
                    let ts = message["ts"].as_str().map(events::ts_order);
                    floor.is_none_or(|floor| ts.is_some_and(|ts| ts > floor))
                })
                .filter_map(|message| events::from_history(team, channel, message, Some(&bot)))
                .collect();
            heard.sort_by_key(|heard| {
                events::split_ref(&heard.key.0)
                    .map(|(_, ts)| events::ts_order(ts))
                    .unwrap_or_default()
            });
            heard.dedup_by(|a, b| a.key == b.key);
            for heard in heard {
                ears.heard(heard).await.map_err(|_| unrecorded())?;
            }
            Ok(())
        })
    }

    fn post<'a>(
        &'a self,
        to: &'a Spot,
        card: &'a Card,
        tag: &'a RelayTag,
    ) -> Pending<'a, Result<Posted, SendError>> {
        Box::pin(async move {
            let channel = channel_of(to)?;
            let (blocks, text) = blocks::render(card);
            let mut body = json!({
                "channel": channel,
                "text": text,
                "blocks": blocks,
                "unfurl_links": false,
                "metadata": { "event_type": RELAY_EVENT, "event_payload": { "tag": tag.0 } },
            });
            if let Some(thread) = &to.thread {
                body["thread_ts"] = json!(thread.0);
            }
            let posted = self
                .api
                .call("chat.postMessage", Token::Bot, Body::Json(body))
                .await
                .map_err(Failure::into_send)?;
            match (posted["channel"].as_str(), posted["ts"].as_str()) {
                (Some(channel), Some(ts)) => Ok(Posted(format!("{channel}/{ts}"))),
                _ => Err(Failure::Garbled.into_send()),
            }
        })
    }

    fn edit<'a>(
        &'a self,
        _to: &'a Spot,
        posted: &'a Posted,
        card: &'a Card,
    ) -> Pending<'a, Result<(), SendError>> {
        Box::pin(async move {
            let (channel, ts) = events::split_ref(&posted.0).ok_or_else(unknown_post)?;
            let (blocks, text) = blocks::render(card);
            let body = json!({ "channel": channel, "ts": ts, "text": text, "blocks": blocks });
            self.api
                .call("chat.update", Token::Bot, Body::Json(body))
                .await
                .map_err(Failure::into_send)?;
            Ok(())
        })
    }

    fn find<'a>(
        &'a self,
        to: &'a Spot,
        tag: &'a RelayTag,
    ) -> Pending<'a, Result<Option<Posted>, SendError>> {
        Box::pin(async move {
            let channel = channel_of(to)?;
            let mut form = vec![
                ("channel", channel.to_string()),
                ("include_all_metadata", "true".to_string()),
                ("limit", FIND_PAGE.to_string()),
            ];
            let (messages, whole) = match &to.thread {
                Some(thread) => {
                    form.push(("ts", thread.0.clone()));
                    self.pages("conversations.replies", form, false).await
                }
                None => self
                    .api
                    .call("conversations.history", Token::Bot, Body::Form(form))
                    .await
                    .and_then(|page| {
                        let messages = page["messages"].as_array().ok_or(Failure::Garbled)?;
                        let more = page["has_more"].as_bool() == Some(true)
                            || page["response_metadata"]["next_cursor"]
                                .as_str()
                                .is_some_and(|cursor| !cursor.is_empty());
                        Ok((messages.clone(), !more))
                    }),
            }
            .map_err(Failure::into_send)?;
            if let Some(message) = messages.iter().find(|message| tagged(message, tag)) {
                let ts = message["ts"]
                    .as_str()
                    .filter(|ts| events::ts_ms(ts).is_some())
                    .ok_or_else(lookup_incomplete)?;
                return Ok(Some(Posted(format!("{channel}/{ts}"))));
            }
            if !whole
                || messages.iter().any(|message| {
                    message["metadata"]["event_type"].as_str() == Some(RELAY_EVENT)
                        && message["metadata"]["event_payload"]["tag"].as_str().is_none()
                })
            {
                return Err(lookup_incomplete());
            }
            Ok(None)
        })
    }

    fn mark<'a>(
        &'a self,
        heard: &'a HeardKey,
        _to: &'a Spot,
        mark: Mark,
    ) -> Pending<'a, Result<(), SendError>> {
        Box::pin(async move {
            let (channel, ts) = events::split_ref(&heard.0).ok_or_else(unknown_post)?;
            self.react("reactions.add", channel, ts, emoji(mark), "already_reacted").await?;
            if mark != Mark::Working {
                self.react("reactions.remove", channel, ts, emoji(Mark::Working), "no_reaction")
                    .await?;
            }
            Ok(())
        })
    }

    fn tell<'a>(
        &'a self,
        to: &'a Spot,
        who: &'a OutsideUser,
        text: &'a str,
    ) -> Pending<'a, Result<(), SendError>> {
        Box::pin(async move {
            let channel = channel_of(to)?;
            let mut body =
                json!({ "channel": channel, "user": who.0, "text": mrkdwn::escape(text) });
            if let Some(thread) = &to.thread {
                body["thread_ts"] = json!(thread.0);
            }
            self.api
                .call("chat.postEphemeral", Token::Bot, Body::Json(body))
                .await
                .map_err(Failure::into_send)?;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_origin_is_slack_unless_the_seam_names_another() {
        assert_eq!(origin(None), ORIGIN);
        assert_eq!(origin(Some("  ".into())), ORIGIN);
        assert_eq!(origin(Some(" http://127.0.0.1:9/ ".into())), "http://127.0.0.1:9");
    }

    #[test]
    fn slack_declares_what_it_can_do() {
        let slack = SlackAdapter::at(
            Secret::new("xoxb-test-not-a-token"),
            Secret::new("xapp-test-not-a-token"),
            ORIGIN,
        );
        assert_eq!(
            slack.abilities(),
            Abilities {
                buttons: Some(Buttons { max: 5, label_chars: 75 }),
                edits: true,
                reactions: true,
                max_text: 3000,
                posts: PostSafety::Findable,
            }
        );
        assert_eq!(slack.id(), AdapterId("slack".into()));
    }
}
