mod common;

use mac_usernotifications::*;

fn main() {
    if !common::setup(file!()) {
        return;
    }
    block_on_main(async {
        let response = Notification::default()
            .title("I'll need some information first")
            .message("Just the basic facts")
            .interruption_level(InterruptionLevel::Critical)
            .action(Action::reply(
                "action.reply.1",
                "favorite animal",
                "pick",
                "dog",
            ))
            .send()
            .await
            .unwrap()
            .response()
            .await
            .unwrap();

        if response.is_default_action() {
            log::info!("ℹ️ notification closed via default action");
        }
        if response.is_dismiss_action() {
            log::info!("ℹ️ notification dismissed");
        }
        if let Some(text) = &response.reply_text {
            log::info!("ℹ️ reply: {text:?}");
        }
        log::info!("ℹ️ response {response:#?}");
    });
}
