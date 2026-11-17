use serde::{Deserialize, Serialize};

use super::ids::{AgentId, ConnectorId, GroupId};

pub const MAX_SERVICE_LEN: usize = 48;
pub const MAX_ACCOUNT_LEN: usize = 120;
pub const MAX_NOTE_LEN: usize = 240;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Connector {
    pub id: ConnectorId,
    pub group_id: GroupId,
    pub service: String,
    pub account: String,
    pub env_var: String,
    pub note: String,
    pub secret_set: bool,
    #[serde(default)]
    pub agents: Vec<AgentId>,
    pub secret_hint: String,
    pub created_at: i64,
    pub updated_at: i64,
}

impl Connector {
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

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorDraft {
    pub group_id: GroupId,
    pub service: String,
    pub account: String,
    pub env_var: String,
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub secret: Option<String>,
    #[serde(default)]
    pub agents: Vec<AgentId>,
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
    #[error(
        "this name controls the process runtime; use the service's API token variable instead"
    )]
    ReservedEnvVar,
    #[error("a secret must be at most 64 KiB and cannot contain NUL bytes")]
    InvalidSecret,
}

impl ConnectorDraft {
    pub fn validate(&self) -> Result<CleanConnector, ConnectorError> {
        let service = self.service.trim();
        if service.is_empty() {
            return Err(ConnectorError::BlankService);
        }
        if service.chars().count() > MAX_SERVICE_LEN {
            return Err(ConnectorError::ServiceTooLong { max: MAX_SERVICE_LEN });
        }

        let account = self.account.trim();
        if account.chars().count() > MAX_ACCOUNT_LEN {
            return Err(ConnectorError::AccountTooLong { max: MAX_ACCOUNT_LEN });
        }

        let note = self.note.trim();
        if note.chars().count() > MAX_NOTE_LEN {
            return Err(ConnectorError::NoteTooLong { max: MAX_NOTE_LEN });
        }

        let env_var = self.env_var.trim().to_ascii_uppercase();
        if reserved(&env_var) {
            return Err(ConnectorError::ReservedEnvVar);
        }
        if !is_env_name(&env_var) {
            return Err(ConnectorError::BadEnvVar { got: env_var });
        }

        let secret = self.secret.as_deref().unwrap_or_default().trim().to_string();
        validate_secret(&secret)?;

        Ok(CleanConnector {
            group_id: self.group_id,
            service: service.to_string(),
            account: account.to_string(),
            env_var,
            note: note.to_string(),
            secret,
            agents: self.agents.clone(),
        })
    }
}

#[derive(Clone, PartialEq)]
pub struct CleanConnector {
    pub group_id: GroupId,
    pub service: String,
    pub account: String,
    pub env_var: String,
    pub note: String,
    pub secret: String,
    pub agents: Vec<AgentId>,
}

impl std::fmt::Debug for CleanConnector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CleanConnector").field("env_var", &self.env_var).finish_non_exhaustive()
    }
}

pub fn validate_secret(secret: &str) -> Result<(), ConnectorError> {
    if secret.trim().is_empty() {
        return Err(ConnectorError::BlankSecret);
    }
    if secret.len() > 65_536 || secret.contains('\0') {
        return Err(ConnectorError::InvalidSecret);
    }
    Ok(())
}

pub(crate) fn reserved(name: &str) -> bool {
    matches!(
        name,
        "PATH"
            | "HOME"
            | "SHELL"
            | "ENV"
            | "BASH_ENV"
            | "ZDOTDIR"
            | "NODE_OPTIONS"
            | "PYTHONPATH"
            | "PYTHONHOME"
            | "RUBYOPT"
            | "PERL5OPT"
            | "GIT_SSH"
            | "GIT_SSH_COMMAND"
            | "GIT_ASKPASS"
            | "SSH_ASKPASS"
            | "GIT_EXEC_PATH"
            | "GIT_TEMPLATE_DIR"
            | "GIT_TERMINAL_PROMPT"
    ) || ["GUACA_", "GUAC_", "LD_", "DYLD_", "GIT_CONFIG", "BASH_FUNC_"]
        .iter()
        .any(|prefix| name.starts_with(prefix))
}

fn is_env_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with(|c: char| c.is_ascii_digit())
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_controls_and_invalid_values_are_refused() {
        for name in [
            "PATH",
            "HOME",
            "BASH_ENV",
            "NODE_OPTIONS",
            "LD_PRELOAD",
            "GUACA_TOKEN",
            "GIT_CONFIG_COUNT",
        ] {
            let mut input = draft();
            input.env_var = name.into();
            assert_eq!(input.validate().unwrap_err(), ConnectorError::ReservedEnvVar);
        }
        for value in ["a\0b".to_string(), "x".repeat(65_537)] {
            assert_eq!(validate_secret(&value), Err(ConnectorError::InvalidSecret));
        }
        assert_eq!(validate_secret("  "), Err(ConnectorError::BlankSecret));
        let mut input = draft();
        input.env_var = "CLOUDFLARE_API_TOKEN".into();
        assert!(input.validate().is_ok());
    }

    fn draft() -> ConnectorDraft {
        ConnectorDraft {
            group_id: GroupId::new(),
            service: "  GitHub  ".into(),
            account: "  madebywelch ".into(),
            env_var: " github_token ".into(),
            note: "  read-only  ".into(),
            agents: vec![],
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
            agents: vec![],
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

        let anonymous = Connector { account: String::new(), ..connector("") };
        assert_eq!(anonymous.own_line(), "- GitHub — the credential is in $GITHUB_TOKEN");
    }

    #[test]
    fn a_connector_carries_no_secret_anywhere_it_could_be_serialized() {
        let held = connector("");
        let json = serde_json::to_value(&held).unwrap();
        assert_eq!(json["secretSet"], true);
        assert!(
            !json.as_object().unwrap().contains_key("secret"),
            "there must be no field a value could arrive in: {json}"
        );
    }
}
