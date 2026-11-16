//! Notification action buttons and the categories that group them.
//!
//! [`Action`] is the public-facing type for defining action buttons.
//! [`ActionCategory`] is an implementation detail; you do not need to use it
//! directly. When you add actions to a [`Notification`] via
//! [`Notification::action`], a category is synthesised and registered
//! automatically when the notification is sent.
//!
//! [`ActionCategory`] is still available for the advanced use case of
//! **server-defined categories**: when a push notification from your server
//! references a `categoryIdentifier` that your app must pre-register at
//! launch (before any push arrives).
//!
//! [`Notification`]: crate::Notification
//! [`Notification::action`]: crate::Notification::action

use std::sync::{Mutex, OnceLock};

use objc2::rc::Retained;
use objc2_foundation::{NSArray, NSSet, NSString};
use objc2_user_notifications::{
    UNNotificationAction, UNNotificationActionOptions, UNNotificationCategory,
    UNNotificationCategoryOptions, UNTextInputNotificationAction, UNUserNotificationCenter,
};

use crate::worker;

static CATEGORIES: OnceLock<Mutex<Vec<ActionCategory>>> = OnceLock::new();

fn categories() -> &'static Mutex<Vec<ActionCategory>> {
    CATEGORIES.get_or_init(|| Mutex::new(Vec::new()))
}

/// A notification action.
///
/// Use [`Action::button`] for a plain button or [`Action::reply`] for a
/// text-input action. Chain [`Action::requires_authentication`] on either
/// to require Touch ID / password before the action fires.
#[derive(Debug, Clone)]
pub enum Action {
    /// A plain button action.
    Button {
        /// Identifier sent back in [`NotificationResponse::action_identifier`](`crate::NotificationResponse::action_identifier`).
        identifier: String,
        /// Button label shown to the user.
        title: String,
        /// Require Touch ID / password before firing.
        requires_authentication: bool,
    },

    /// A text-input action.
    ///
    /// The user's input is delivered via
    /// [`NotificationResponse::reply_text`](`crate::NotificationResponse::reply_text`).
    ///
    /// macOS only renders one `Reply` action correctly per notification.
    /// If multiple are added, only the first is used.
    Reply {
        /// Identifier sent back in [`NotificationResponse::action_identifier`](`crate::NotificationResponse::action_identifier`).
        identifier: String,
        /// Label shown in the Options menu.
        title: String,
        /// Label on the submit button inside the expanded text field.
        button_title: String,
        /// Placeholder text inside the input field.
        placeholder: String,
        /// Require Touch ID / password before opening the text field.
        requires_authentication: bool,
    },
}

impl Action {
    /// Create a plain button action.
    pub fn button(identifier: impl Into<String>, title: impl Into<String>) -> Self {
        Self::Button {
            identifier: identifier.into(),
            title: title.into(),
            requires_authentication: false,
        }
    }

    /// Create a text-input (reply) action.
    ///
    /// - `button_title` is the label on the submit button (e.g. `"Send"`).
    /// - `placeholder` is the hint text inside the empty input field.
    pub fn reply(
        identifier: impl Into<String>,
        title: impl Into<String>,
        button_title: impl Into<String>,
        placeholder: impl Into<String>,
    ) -> Self {
        Self::Reply {
            identifier: identifier.into(),
            title: title.into(),
            button_title: button_title.into(),
            placeholder: placeholder.into(),
            requires_authentication: false,
        }
    }

    /// Require Touch ID / password before the action fires.
    ///
    /// Works for both [`Action::Button`] and [`Action::Reply`].
    pub fn requires_authentication(self) -> Self {
        match self {
            Self::Button {
                identifier, title, ..
            } => Self::Button {
                identifier,
                title,
                requires_authentication: true,
            },
            Self::Reply {
                identifier,
                title,
                button_title,
                placeholder,
                ..
            } => Self::Reply {
                identifier,
                title,
                button_title,
                placeholder,
                requires_authentication: true,
            },
        }
    }

    /// The action's identifier.
    pub(crate) fn identifier(&self) -> &str {
        match self {
            Self::Button { identifier, .. } | Self::Reply { identifier, .. } => identifier,
        }
    }

    /// Build the Objective-C action object.
    pub(crate) fn build(&self) -> Retained<UNNotificationAction> {
        match self {
            Self::Button {
                identifier,
                title,
                requires_authentication,
            } => {
                let mut options = UNNotificationActionOptions::empty();
                if *requires_authentication {
                    options |= UNNotificationActionOptions::AuthenticationRequired;
                }
                UNNotificationAction::actionWithIdentifier_title_options(
                    &NSString::from_str(identifier),
                    &NSString::from_str(title),
                    options,
                )
            }
            Self::Reply {
                identifier,
                title,
                button_title,
                placeholder,
                requires_authentication,
            } => {
                let mut options = UNNotificationActionOptions::empty();
                if *requires_authentication {
                    options |= UNNotificationActionOptions::AuthenticationRequired;
                }
                // UNTextInputNotificationAction is a direct subclass of
                // UNNotificationAction; into_super() performs the safe upcast.
                UNTextInputNotificationAction::actionWithIdentifier_title_options_textInputButtonTitle_textInputPlaceholder(
                    &NSString::from_str(identifier),
                    &NSString::from_str(title),
                    options,
                    &NSString::from_str(button_title),
                    &NSString::from_str(placeholder),
                )
                .into_super()
            }
        }
    }
}

/// A group of actions registered with macOS.
///
/// Pre-register with [`ActionCategory::register`] for server-defined categories,
/// or use [`Notification::action`] for automatic registration.
///
/// [`Notification::action`]: crate::Notification::action
#[derive(Debug, Clone)]
pub struct ActionCategory {
    /// Category identifier.
    pub identifier: String,
    pub(crate) actions: Vec<Action>,
}

fn upsert_category(categories: &mut Vec<ActionCategory>, category: ActionCategory) {
    match categories
        .iter()
        .position(|cat| cat.identifier == category.identifier)
    {
        Some(idx) => categories[idx] = category,
        None => categories.push(category),
    }
}

impl ActionCategory {
    /// Create a new category.
    pub fn new(identifier: impl Into<String>) -> Self {
        Self {
            identifier: identifier.into(),
            actions: Vec::new(),
        }
    }

    /// Create a category from an existing list of actions.
    pub(crate) fn from_actions(identifier: &str, actions: Vec<Action>) -> Self {
        Self {
            identifier: identifier.to_owned(),
            actions,
        }
    }

    /// Add an action to this category.
    pub fn action(mut self, action: Action) -> Self {
        self.actions.push(action);
        self
    }

    /// Build the `UNNotificationCategory` object.
    fn build_category(&self) -> Retained<UNNotificationCategory> {
        let actions: Vec<_> = self.actions.iter().map(|act| act.build()).collect();
        let actions_array = NSArray::from_retained_slice(&actions);
        UNNotificationCategory::categoryWithIdentifier_actions_intentIdentifiers_options(
            &NSString::from_str(&self.identifier),
            &actions_array,
            &NSArray::new(),
            UNNotificationCategoryOptions::CustomDismissAction,
        )
    }

    /// Register this category synchronously, merging with all previously registered categories.
    ///
    /// If a category with the same identifier was registered before, it is replaced.
    /// Must only be called from the worker thread.
    pub(crate) fn register_now(&self) {
        let mut categories = categories().lock().expect("category registry poisoned");

        upsert_category(&mut categories, self.clone());

        let built: Vec<_> = categories.iter().map(|cat| cat.build_category()).collect();
        let full_set = NSSet::from_retained_slice(&built);
        UNUserNotificationCenter::currentNotificationCenter().setNotificationCategories(&full_set);
        log::debug!(
            "registered category '{}' ({} total)",
            self.identifier,
            categories.len()
        );
    }

    /// Register this category with `UNUserNotificationCenter`.
    ///
    /// Only needed for server-defined categories. For local notifications,
    /// use [`Notification::action`] instead.
    ///
    /// [`Notification::action`]: crate::Notification::action
    pub fn register(self) {
        worker::dispatch(move || self.register_now());
    }
}

#[cfg(test)]
mod test_upsert {
    use super::{Action, ActionCategory, upsert_category};

    fn cat(id: &str) -> ActionCategory {
        ActionCategory::new(id)
    }

    fn cat_with_action(id: &str, action_id: &str) -> ActionCategory {
        ActionCategory::new(id).action(Action::button(action_id, action_id))
    }

    #[test]
    fn appends_when_identifier_is_new() {
        let mut cats = vec![cat("a"), cat("b")];
        upsert_category(&mut cats, cat("c"));
        assert_eq!(cats.len(), 3);
        assert_eq!(cats[2].identifier, "c");
    }

    #[test]
    fn replaces_without_changing_length() {
        let mut cats = vec![cat("a"), cat_with_action("b", "old"), cat("c")];
        upsert_category(&mut cats, cat_with_action("b", "new"));
        assert_eq!(cats.len(), 3, "length must not change on replace");
        assert_eq!(cats[1].actions[0].identifier(), "new");
    }

    #[test]
    fn replaces_at_correct_position_leaving_others_undisturbed() {
        let mut cats = vec![cat("a"), cat("b"), cat("c")];
        upsert_category(&mut cats, cat_with_action("a", "x"));
        assert_eq!(cats[0].actions[0].identifier(), "x");
        assert_eq!(cats[1].identifier, "b");
        assert_eq!(cats[2].identifier, "c");
    }

    #[test]
    fn first_insert_into_empty_vec() {
        let mut cats = vec![];
        upsert_category(&mut cats, cat("a"));
        assert_eq!(cats.len(), 1);
        assert_eq!(cats[0].identifier, "a");
    }
}

#[cfg(test)]
mod test_action {
    use super::Action;

    #[test]
    fn button_requires_authentication_sets_flag() {
        let action = Action::button("id", "Confirm").requires_authentication();
        assert!(matches!(
            action,
            Action::Button {
                requires_authentication: true,
                ..
            }
        ));
    }

    #[test]
    fn reply_requires_authentication_sets_flag() {
        let action = Action::reply("id", "Reply", "Send", "hint").requires_authentication();
        assert!(matches!(
            action,
            Action::Reply {
                requires_authentication: true,
                ..
            }
        ));
    }

    #[test]
    fn identifier_accessible_on_both_variants() {
        assert_eq!(Action::button("btn-id", "OK").identifier(), "btn-id");
        assert_eq!(
            Action::reply("reply-id", "Reply", "Send", "hint").identifier(),
            "reply-id"
        );
    }
}
