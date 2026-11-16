use mac_usernotifications::{Action, Notification, block_on_current, request_auth};

mod common;
use common::notify_back;

const ACTION_REPLY: &str = "action.reply";
const ACTION_LIKE: &str = "action.like";
const ACTION_TRASH: &str = "action.trash";
const ACTION_QUICK_REPLY: &str = "action.quick_reply";

fn main() {
    if !common::setup(file!()) {
        return;
    }
    block_on_current(run()).unwrap();
}

async fn run() {
    match request_auth().await {
        Ok(true) => log::warn!("permission granted"),
        Ok(false) => {
            log::error!("permission denied; allow in System Settings -> Notifications");
            return;
        }
        Err(error) => {
            log::error!("auth error: {error}");
            return;
        }
    }

    notification_1().await.unwrap();
    notification_2().await.unwrap();

    log::info!("done");
}

/// Incoming chat message: reply, like, or delete.
async fn notification_1() -> Result<(), Box<dyn std::error::Error>> {
    log::info!("sending notification 1 (chat message)…");

    let response = Notification::new()
        .title("New message from Hendrik")
        .message("Hey, are you coming tonight?")
        .action(Action::reply(
            ACTION_REPLY,
            "📝 Reply",
            "Send",
            "Type a reply…",
        ))
        .action(Action::button(ACTION_LIKE, "👍 Like"))
        .action(Action::button(ACTION_TRASH, "🗑️ Delete").requires_authentication())
        .timeout(std::time::Duration::from_secs(60))
        .send()
        .await?
        .response()
        .await;

    match response {
        Ok(response) if response.is_default_action() => {
            log::info!("opened");
            notify_back("Opened", "You clicked the notification body.").await;
        }
        Ok(response) if response.is_dismiss_action() => {
            log::warn!("dismissed");
            notify_back("Dismissed", "You dismissed without choosing an action.").await;
        }
        Ok(response) if response.action_identifier == ACTION_REPLY => {
            let text = response.reply_text.as_deref().unwrap_or("<empty>");
            log::info!("replied: {text:?}");
            notify_back("Message sent ✉️", &format!("You replied: \"{text}\"")).await;
        }
        Ok(response) if response.action_identifier == ACTION_LIKE => {
            log::info!("liked");
            notify_back("Liked 👍", "Hendrik's message was liked.").await;
        }
        Ok(response) if response.action_identifier == ACTION_TRASH => {
            log::info!("deleted");
            notify_back("Deleted 🗑", "Message deleted.").await;
        }
        Ok(response) if response.is_timed_out() => log::warn!("notification 1 timed out"),
        Ok(response) => log::warn!("unknown action: {}", response.action_identifier),
        Err(error) => log::error!("notification 1 error: {error}"),
    }
    Ok(())
}

/// Reminder that prompts for a quick text response.
async fn notification_2() -> Result<(), Box<dyn std::error::Error>> {
    log::info!("sending notification 2 (reminder with reply)…");

    let response = Notification::new()
        .title("Daily standup in 5 minutes")
        .message("Anything blocking you today?")
        .action(Action::reply(
            ACTION_QUICK_REPLY,
            "Add blocker",
            "Submit",
            "e.g. Waiting on review…",
        ))
        .timeout(std::time::Duration::from_secs(60))
        .send()
        .await?
        .response()
        .await;

    match response {
        Ok(response) if response.action_identifier == ACTION_QUICK_REPLY => {
            let text = response.reply_text.as_deref().unwrap_or("<empty>");
            log::info!("blocker noted: {text:?}");
            notify_back("Noted 📝", &format!("Blocker added: \"{text}\"")).await;
        }
        Ok(response) if response.is_dismiss_action() => {
            log::info!("dismissed standup reminder");
            notify_back("All clear!", "No blockers noted.").await;
        }
        Ok(response) if response.is_default_action() => {
            log::info!("opened standup notification");
        }
        Ok(response) if response.is_timed_out() => log::info!("notification 2 timed out"),
        Ok(response) => log::info!("unknown action: {}", response.action_identifier),
        Err(error) => log::error!("notification 2 error: {error}"),
    }
    Ok(())
}
