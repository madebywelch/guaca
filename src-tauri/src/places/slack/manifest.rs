use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
use serde_json::{json, Value};

pub const SCOPES: [&str; 6] = [
    "app_mentions:read",
    "chat:write",
    "im:history",
    "channels:history",
    "groups:history",
    "reactions:write",
];

pub const EVENTS: [&str; 4] = ["app_mention", "message.im", "message.channels", "message.groups"];

pub const NEW_APP: &str = "https://api.slack.com/apps?new_app=1&manifest_json=";

pub fn manifest() -> Value {
    json!({
        "display_information": {
            "name": "Guaca",
            "description": "Your Guaca agents, in Slack. Runs on your own Host."
        },
        "features": {
            "app_home": { "messages_tab_enabled": true, "messages_tab_read_only_enabled": false },
            "bot_user": { "display_name": "Guaca", "always_online": true }
        },
        "oauth_config": { "scopes": { "bot": SCOPES } },
        "settings": {
            "event_subscriptions": { "bot_events": EVENTS },
            "interactivity": { "is_enabled": true },
            "socket_mode_enabled": true,
            "org_deploy_enabled": false,
            "token_rotation_enabled": false
        }
    })
}

pub fn create_link() -> String {
    format!("{NEW_APP}{}", utf8_percent_encode(&manifest().to_string(), NON_ALPHANUMERIC))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    fn strings(value: &Value) -> BTreeSet<String> {
        value.as_array().unwrap().iter().map(|item| item.as_str().unwrap().to_string()).collect()
    }

    #[test]
    fn the_app_asks_for_exactly_the_least_scopes_and_events() {
        let manifest = manifest();
        let scopes = strings(&manifest["oauth_config"]["scopes"]["bot"]);
        let least: BTreeSet<String> = [
            "app_mentions:read",
            "chat:write",
            "im:history",
            "channels:history",
            "groups:history",
            "reactions:write",
        ]
        .map(String::from)
        .into();
        assert_eq!(scopes, least);
        assert!(!scopes
            .iter()
            .any(|scope| scope.starts_with("users:") || scope.starts_with("files:")));
        let events = strings(&manifest["settings"]["event_subscriptions"]["bot_events"]);
        let wanted: BTreeSet<String> =
            ["app_mention", "message.im", "message.channels", "message.groups"]
                .map(String::from)
                .into();
        assert_eq!(events, wanted);
        assert_eq!(manifest["settings"]["socket_mode_enabled"], true);
        assert_eq!(manifest["settings"]["interactivity"]["is_enabled"], true);
        assert!(manifest["settings"].get("slash_commands").is_none());
        assert!(manifest["features"].get("slash_commands").is_none());
    }

    #[test]
    fn the_link_carries_the_manifest_whole() {
        let link = create_link();
        let encoded = link.strip_prefix(NEW_APP).unwrap();
        assert!(!encoded.contains(['{', '"', ' ', '&', '#']));
        let decoded = percent_encoding::percent_decode_str(encoded).decode_utf8().unwrap();
        assert_eq!(serde_json::from_str::<Value>(&decoded).unwrap(), manifest());
    }
}
