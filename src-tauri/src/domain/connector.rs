//! Credentials a crew can use.
//!
//! One kind, deliberately. A connector is an API token the operator pastes in
//! once, held for a whole group, and put into the environment of every command
//! that group's machines run. The *other* way an agent reaches an account, a
//! browser that is already logged in, is not recorded here and is not recorded
//! anywhere: it is detected from the machine itself. See [`super::signin`].
//!
//! That split is the design. Anything the browser can be asked, Guaca asks the
//! browser; a token is stored only because there is nowhere else for it to live.
//!
//! Two kinds of access are still offered to the agent in one list, because
//! *Beyond Browsing: API-Based Web Agents* (Song, Xu, Zhou and Neubig,
//! [arXiv 2410.16464](https://arxiv.org/abs/2410.16464)) measured API-calling
//! agents against browsing agents on the same WebArena tasks: APIs beat
//! browsing, and a hybrid that could choose beat both by 24 points absolute
//! over browsing alone. What changed is where each half comes from, not that
//! there are two.
//!
//! **The secret never reaches the model.** A value lives in SQLite and is put
//! into the environment of commands the agent runs. It is never rendered into a
//! prompt, never returned over IPC, and never written to the sandbox's disk.
//! The agent is told the variable's name and told to use it by name. This is
//! the boundary `commands.rs` draws around the API key, moved one layer in: the
//! webview never holds a credential, and neither does the model.

use serde::{Deserialize, Serialize};

use super::ids::{ConnectorId, GroupId};

/// A service name longer than this is a paragraph, not a label.
pub const MAX_SERVICE_LEN: usize = 48;
pub const MAX_ACCOUNT_LEN: usize = 120;
/// Notes are read by a model on every turn, so they are one line, not a page.
pub const MAX_NOTE_LEN: usize = 240;

/// A credential the whole group's machines are given.
///
/// Serializable in full: there is no secret on it. The value is held in the
/// store and only ever leaves it into a sandbox's process environment.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Connector {
    pub id: ConnectorId,
    /// Scoped to a group, like everything else an agent can see. A crew cannot
    /// use another crew's credentials any more than it can message its agents.
    pub group_id: GroupId,
    /// What the credential is for: `GitHub`, `Linear`, `Stripe`.
    pub service: String,
    /// Who it acts as, when the operator said. Often blank: picking GitHub from
    /// a list needs a token and nothing else, and a box demanding the account
    /// name of a token you already hold is a question with no purpose.
    pub account: String,
    /// The environment variable the agent will find it in.
    pub env_var: String,
    /// One line for the agent, in the operator's words: `read-only`,
    /// `production, do not write`.
    pub note: String,
    /// Whether a value is stored. False is a connector that would hand the
    /// machine an empty variable, which is worth showing as broken.
    pub secret_set: bool,
    /// Last four characters, so an operator can tell two tokens apart without
    /// either of them being sent to the webview.
    pub secret_hint: String,
    pub created_at: i64,
    pub updated_at: i64,
}

impl Connector {
    /// The line an agent reads about a credential it holds.
    pub fn own_line(&self) -> String {
        let mut line = format!("- {}", self.service);
        if !self.account.is_empty() {
            line.push_str(&format!(" as {}", self.account));
        }
        line.push_str(&format!(" — the credential is in ${}", self.env_var));
        if !self.note.is_empty() {
            line.push_str(&format!(" ({})", self.note));
        }
        line
    }
}

/// Fields an operator can set. Separate from [`Connector`] so ids and
/// timestamps cannot be forged across IPC, and so the secret can be carried
/// inward without ever being carried back.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorDraft {
    pub group_id: GroupId,
    pub service: String,
    pub account: String,
    pub env_var: String,
    #[serde(default)]
    pub note: String,
    /// There is no edit path, so this is the only moment a value can arrive: a
    /// connector is forgotten and re-added rather than rewritten, which keeps
    /// the one command that can carry a secret to a command that also creates
    /// the row it belongs to.
    #[serde(default)]
    pub secret: Option<String>,
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ConnectorError {
    #[error("say what service this credential is for, for example GitHub")]
    BlankService,
    #[error("service must be {max} characters or fewer")]
    ServiceTooLong { max: usize },
    #[error("account must be {max} characters or fewer")]
    AccountTooLong { max: usize },
    #[error("note must be {max} characters or fewer")]
    NoteTooLong { max: usize },
    #[error(
        "an environment variable name must be letters, digits and underscores, and must not \
         start with a digit; got {got:?}"
    )]
    BadEnvVar { got: String },
    #[error("a credential needs a value")]
    BlankSecret,
}

impl ConnectorDraft {
    /// Normalizes and validates operator input.
    pub fn validate(&self) -> Result<CleanConnector, ConnectorError> {
        let service = self.service.trim();
        if service.is_empty() {
            return Err(ConnectorError::BlankService);
        }
        if service.chars().count() > MAX_SERVICE_LEN {
            return Err(ConnectorError::ServiceTooLong { max: MAX_SERVICE_LEN });
        }

        // Blank is fine and usual. A token identifies its own account to the
        // service it belongs to, so demanding one here is a box the operator
        // has to invent an answer for.
        let account = self.account.trim();
        if account.chars().count() > MAX_ACCOUNT_LEN {
            return Err(ConnectorError::AccountTooLong { max: MAX_ACCOUNT_LEN });
        }

        let note = self.note.trim();
        if note.chars().count() > MAX_NOTE_LEN {
            return Err(ConnectorError::NoteTooLong { max: MAX_NOTE_LEN });
        }

        let env_var = self.env_var.trim().to_ascii_uppercase();
        if !is_env_name(&env_var) {
            return Err(ConnectorError::BadEnvVar { got: env_var });
        }

        // A credential with no value would put an empty variable on the
        // machine, and the agent would read the resulting 401 as a revoked
        // token rather than as a connector nobody finished setting up. There is
        // no edit path to supply it later, so it is required here.
        let secret = self.secret.as_deref().unwrap_or_default().trim().to_string();
        if secret.is_empty() {
            return Err(ConnectorError::BlankSecret);
        }

        Ok(CleanConnector {
            group_id: self.group_id,
            service: service.to_string(),
            account: account.to_string(),
            env_var,
            note: note.to_string(),
            secret,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CleanConnector {
    pub group_id: GroupId,
    pub service: String,
    pub account: String,
    pub env_var: String,
    pub note: String,
    pub secret: String,
}

/// Whether a string is safe to use as a shell environment variable name.
///
/// The name goes into a process environment beside a secret. Anything outside
/// this set is either ignored by the shell or, worse, is not a variable name at
/// all and turns the surrounding command into something else.
fn is_env_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with(|c: char| c.is_ascii_digit())
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft() -> ConnectorDraft {
        ConnectorDraft {
            group_id: GroupId::new(),
            service: "  GitHub  ".into(),
            account: "  madebywelch ".into(),
            env_var: " github_token ".into(),
            note: "  read-only  ".into(),
            secret: Some("  ghp_secret  ".into()),
        }
    }

    #[test]
    fn a_draft_is_trimmed_and_the_variable_upper_cased() {
        let clean = draft().validate().unwrap();
        assert_eq!(clean.service, "GitHub");
        assert_eq!(clean.account, "madebywelch");
        assert_eq!(clean.note, "read-only");
        assert_eq!(clean.env_var, "GITHUB_TOKEN");
        assert_eq!(clean.secret, "ghp_secret", "the value is trimmed, not dropped");
    }

    #[test]
    fn an_environment_variable_name_is_checked_before_it_reaches_a_shell() {
        // The name is written into a process environment next to a secret. A
        // name with a space or a semicolon in it is not a variable at all.
        for bad in ["TOKEN;rm -rf /", "MY TOKEN", "2TOKEN", "", "TOKEN$X", "TOKEN-A"] {
            let mut d = draft();
            d.env_var = bad.into();
            assert!(
                matches!(d.validate(), Err(ConnectorError::BadEnvVar { .. })),
                "{bad:?} must be refused"
            );
        }
    }

    #[test]
    fn a_credential_without_a_value_is_refused_rather_than_stored_empty() {
        // Stored empty, the machine gets `GITHUB_TOKEN=` and every call comes
        // back unauthorized, which reads to the agent as a revoked token rather
        // than as a connector nobody finished setting up. There is no edit path
        // to supply it later, so this is the only moment to insist.
        for missing in [Some("   ".to_string()), None] {
            let mut d = draft();
            d.secret = missing;
            assert_eq!(d.validate().unwrap_err(), ConnectorError::BlankSecret);
        }
    }

    #[test]
    fn a_service_is_required_and_an_account_is_not() {
        let mut d = draft();
        d.service = "  ".into();
        assert_eq!(d.validate().unwrap_err(), ConnectorError::BlankService);

        // Picking GitHub from a list needs a token and nothing else. A box
        // demanding the account name of a token you already hold is a question
        // with no purpose, so a blank one is the ordinary case.
        let mut d = draft();
        d.account = "".into();
        assert_eq!(d.validate().unwrap().account, "");
    }

    #[test]
    fn oversized_fields_are_refused_by_character_count() {
        let mut d = draft();
        d.service = "\u{1f951}".repeat(MAX_SERVICE_LEN + 1);
        assert!(matches!(d.validate(), Err(ConnectorError::ServiceTooLong { .. })));

        let mut d = draft();
        d.note = "x".repeat(MAX_NOTE_LEN + 1);
        assert!(matches!(d.validate(), Err(ConnectorError::NoteTooLong { .. })));
    }

    fn connector(note: &str) -> Connector {
        Connector {
            id: ConnectorId::new(),
            group_id: GroupId::new(),
            service: "GitHub".into(),
            account: "madebywelch".into(),
            env_var: "GITHUB_TOKEN".into(),
            note: note.into(),
            secret_set: true,
            secret_hint: "...cret".into(),
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn a_credential_names_the_variable_and_never_the_value() {
        assert_eq!(
            connector("").own_line(),
            "- GitHub as madebywelch — the credential is in $GITHUB_TOKEN"
        );
        assert!(connector("read-only").own_line().ends_with("(read-only)"));

        // The usual case, where the operator picked a service and pasted a
        // token: no account to name, and no dangling "as".
        let anonymous = Connector { account: String::new(), ..connector("") };
        assert_eq!(anonymous.own_line(), "- GitHub — the credential is in $GITHUB_TOKEN");
    }

    #[test]
    fn a_connector_carries_no_secret_anywhere_it_could_be_serialized() {
        // The whole point of the split between Connector and ConnectorDraft. If
        // a secret field ever lands here it goes to the webview, into the
        // prompt, and into the transcript, all at once.
        let held = connector("");
        let json = serde_json::to_value(&held).unwrap();
        assert_eq!(json["secretSet"], true);
        assert!(
            !json.as_object().unwrap().contains_key("secret"),
            "there must be no field a value could arrive in: {json}"
        );
    }
}
