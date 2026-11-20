use serde_json::Value;

use super::blocks::parse_action;
use crate::domain::place::{Heard, HeardKey, OutsideUser, PlaceKey, Said, Spot, ThreadKey};

pub fn ts_ms(ts: &str) -> Option<i64> {
    let (seconds, fraction) = ts.split_once('.').unwrap_or((ts, ""));
    let seconds: i64 = seconds.parse().ok()?;
    let millis: String = fraction.chars().chain("000".chars()).take(3).collect();
    seconds.checked_mul(1000)?.checked_add(millis.parse::<i64>().ok()?)
}

pub fn ts_order(ts: &str) -> (i64, i64) {
    let (seconds, fraction) = ts.split_once('.').unwrap_or((ts, ""));
    let micros: String = fraction.chars().chain("000000".chars()).take(6).collect();
    (seconds.parse().unwrap_or_default(), micros.parse().unwrap_or_default())
}

pub fn place(team: &str, channel: &str) -> PlaceKey {
    PlaceKey(format!("{team}/{channel}"))
}

pub fn split_place(place: &PlaceKey) -> Option<(&str, &str)> {
    place.0.split_once('/').filter(|(team, channel)| !team.is_empty() && !channel.is_empty())
}

pub fn split_ref(reference: &str) -> Option<(&str, &str)> {
    let mut parts = reference.split('/');
    let channel = parts.next().filter(|part| !part.is_empty())?;
    let ts = parts.next().filter(|part| !part.is_empty())?;
    Some((channel, ts))
}

pub fn strip_mention(text: &str, bot: Option<&str>) -> String {
    let Some(bot) = bot else { return text.trim().to_string() };
    let mut out = text.replace(&format!("<@{bot}>"), "");
    let labeled = format!("<@{bot}|");
    while let Some(start) = out.find(&labeled) {
        let Some(end) = out[start..].find('>') else { break };
        out.replace_range(start..start + end + 1, "");
    }
    out.trim().to_string()
}

pub fn from_event(team: &str, event: &Value, bot: Option<&str>) -> Option<Heard> {
    let channel = event["channel"].as_str()?;
    let direct = match event["type"].as_str()? {
        "message" => event["channel_type"].as_str() == Some("im"),
        "app_mention" => channel.starts_with('D'),
        _ => return None,
    };
    message(team, channel, direct, event, bot)
}

pub fn from_history(
    team: &str,
    channel: &str,
    message_: &Value,
    bot: Option<&str>,
) -> Option<Heard> {
    if message_["type"].as_str().is_some_and(|kind| kind != "message") {
        return None;
    }
    message(team, channel, channel.starts_with('D'), message_, bot)
}

fn message(
    team: &str,
    channel: &str,
    direct: bool,
    event: &Value,
    bot: Option<&str>,
) -> Option<Heard> {
    if event.get("bot_id").is_some_and(|id| !id.is_null())
        || event["hidden"].as_bool() == Some(true)
    {
        return None;
    }
    if !matches!(event["subtype"].as_str(), None | Some("thread_broadcast")) {
        return None;
    }
    let speaker = event["user"].as_str()?;
    if Some(speaker) == bot {
        return None;
    }
    let ts = event["ts"].as_str()?;
    let text = strip_mention(event["text"].as_str()?, bot);
    if text.is_empty() {
        return None;
    }
    let thread = event["thread_ts"].as_str().or((!direct).then_some(ts));
    Some(Heard {
        key: HeardKey(format!("{channel}/{ts}")),
        spot: Spot {
            place: place(team, channel),
            thread: thread.map(|ts| ThreadKey(ts.to_string())),
        },
        direct,
        speaker: OutsideUser(speaker.to_string()),
        at_ms: ts_ms(ts)?,
        said: Said::Text { text },
    })
}

pub fn from_press(payload: &Value) -> Vec<Heard> {
    let press = || {
        let team = payload["team"]["id"].as_str()?;
        let speaker = payload["user"]["id"].as_str()?;
        let container = &payload["container"];
        let channel = container["channel_id"].as_str().or(payload["channel"]["id"].as_str())?;
        let message_ts = container["message_ts"].as_str().or(payload["message"]["ts"].as_str())?;
        let direct = channel.starts_with('D');
        let thread = container["thread_ts"]
            .as_str()
            .or(payload["message"]["thread_ts"].as_str())
            .or((!direct).then_some(message_ts));
        let heard: Vec<Heard> = payload["actions"]
            .as_array()?
            .iter()
            .filter_map(|action| {
                let (ask, choice) = parse_action(action["action_id"].as_str()?)?;
                let action_ts = action["action_ts"].as_str()?;
                Some(Heard {
                    key: HeardKey(format!("{channel}/{message_ts}/{action_ts}")),
                    spot: Spot {
                        place: place(team, channel),
                        thread: thread.map(|ts| ThreadKey(ts.to_string())),
                    },
                    direct,
                    speaker: OutsideUser(speaker.to_string()),
                    at_ms: ts_ms(action_ts)?,
                    said: Said::Choice { ask, choice },
                })
            })
            .collect();
        Some(heard)
    };
    press().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_slack_timestamp_becomes_milliseconds_and_orders_by_its_micros() {
        assert_eq!(ts_ms("1732118400.000100"), Some(1_732_118_400_000));
        assert_eq!(ts_ms("1732118400.9"), Some(1_732_118_400_900));
        assert_eq!(ts_ms("nonsense"), None);
        assert_eq!(ts_ms("99999999999999999.0"), None);
        assert!(ts_order("1732118400.000100") < ts_order("1732118400.000200"));
        assert!(ts_order("1732118400.9") > ts_order("1732118400.000200"));
    }

    #[test]
    fn only_the_bots_own_mention_is_removed() {
        assert_eq!(strip_mention("<@UBOT> ship it", Some("UBOT")), "ship it");
        assert_eq!(strip_mention("hey <@UBOT|guaca>, ask <@U2>", Some("UBOT")), "hey , ask <@U2>");
        assert_eq!(strip_mention(" <@U2> hi ", None), "<@U2> hi");
        assert_eq!(strip_mention("<@UBOT> first\nsecond", Some("UBOT")), "first\nsecond");
    }

    #[test]
    fn a_dm_has_no_thread_and_a_channel_message_starts_its_own() {
        let dm = json!({"type":"message","channel":"D1","channel_type":"im","user":"U1","text":"hi","ts":"1.000001"});
        let heard = from_event("T1", &dm, Some("UBOT")).unwrap();
        assert!(heard.direct);
        assert_eq!(heard.spot.thread, None);
        assert_eq!(heard.spot.place.0, "T1/D1");
        let channel = json!({"type":"message","channel":"C1","channel_type":"channel","user":"U1","text":"hi","ts":"2.000001"});
        let heard = from_event("T1", &channel, Some("UBOT")).unwrap();
        assert!(!heard.direct);
        assert_eq!(heard.spot.thread, Some(ThreadKey("2.000001".into())));
        let reply = json!({"type":"message","channel":"C1","channel_type":"channel","user":"U1","text":"hi","ts":"3.0","thread_ts":"2.000001"});
        assert_eq!(
            from_event("T1", &reply, None).unwrap().spot.thread,
            Some(ThreadKey("2.000001".into()))
        );
    }

    #[test]
    fn bots_edits_deletions_and_joins_are_not_heard() {
        let base = json!({"type":"message","channel":"C1","channel_type":"channel","user":"U1","text":"hi","ts":"1.0"});
        let with = |field: &str, value: Value| {
            let mut event = base.clone();
            event[field] = value;
            event
        };
        assert!(from_event("T1", &with("bot_id", json!("B1")), None).is_none());
        for subtype in ["message_changed", "message_deleted", "channel_join", "bot_message"] {
            assert!(
                from_event("T1", &with("subtype", json!(subtype)), None).is_none(),
                "{subtype}"
            );
        }
        assert!(from_event("T1", &with("subtype", json!("thread_broadcast")), None).is_some());
        assert!(from_event("T1", &with("user", json!("UBOT")), Some("UBOT")).is_none());
        assert!(from_event("T1", &with("text", json!("<@UBOT>")), Some("UBOT")).is_none());
        assert!(from_event("T1", &with("type", json!("reaction_added")), None).is_none());
    }

    #[test]
    fn a_press_names_the_ask_and_the_choice() {
        let payload = json!({
            "type": "block_actions", "team": {"id": "T1"}, "user": {"id": "U1"},
            "container": {"channel_id": "D1", "message_ts": "5.0"},
            "actions": [{"action_id": "ask:ap_9:deny", "action_ts": "6.5"}, {"action_id": "other", "action_ts": "6.6"}]
        });
        let heard = from_press(&payload);
        assert_eq!(heard.len(), 1);
        assert_eq!(heard[0].said, Said::Choice { ask: "ap_9".into(), choice: "deny".into() });
        assert_eq!(heard[0].key.0, "D1/5.0/6.5");
        assert!(heard[0].direct);
        assert!(from_press(&json!({})).is_empty());
    }
}
