use serde::{Deserialize, Serialize};

use super::cut_to;
use super::ids::{AgentId, ArtifactId, GroupId};
use super::worknote::how_long_ago;

pub const MAX_TITLE: usize = 120;

pub const MAX_NOTE: usize = 300;

pub const MAX_PAGE: usize = 512 * 1024;

pub const MAX_CONDENSED: usize = 16 * 1024;

pub const LISTED: usize = 20;

pub const LOG_SHOWN: usize = 20;

pub const MAX_SOURCES: usize = 8;

pub const MAX_SOURCE_NAME: usize = 32;

pub const MAX_SOURCE_ARGUMENTS: usize = 4 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Source {
    pub name: String,
    pub tool: String,
    pub arguments: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Actor {
    Operator,
    Agent { id: AgentId, name: String },
}

impl Actor {
    pub fn agent_id(&self) -> Option<AgentId> {
        match self {
            Actor::Operator => None,
            Actor::Agent { id, .. } => Some(*id),
        }
    }

    pub fn stored_name(&self) -> &str {
        match self {
            Actor::Operator => "",
            Actor::Agent { name, .. } => name,
        }
    }

    fn said_to(&self, reader: AgentId) -> String {
        match self {
            Actor::Operator => "the operator".to_string(),
            Actor::Agent { id, .. } if *id == reader => "you".to_string(),
            Actor::Agent { name, .. } => name.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Change {
    Created,
    Edited,
    Restored,
    Took,
    Handed,
    Allowed,
}

impl Change {
    pub fn as_str(self) -> &'static str {
        match self {
            Change::Created => "created",
            Change::Edited => "edited",
            Change::Restored => "restored",
            Change::Took => "took",
            Change::Handed => "handed",
            Change::Allowed => "allowed",
        }
    }

    pub fn parse(raw: &str) -> Option<Change> {
        Some(match raw {
            "created" => Change::Created,
            "edited" => Change::Edited,
            "restored" => Change::Restored,
            "took" => Change::Took,
            "handed" => Change::Handed,
            "allowed" => Change::Allowed,
            _ => return None,
        })
    }

    pub fn makes_version(self) -> bool {
        matches!(self, Change::Created | Change::Edited | Change::Restored)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Owner {
    pub id: AgentId,
    pub name: String,
    pub gone: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Artifact {
    pub id: ArtifactId,
    pub group_id: GroupId,
    pub owner: Option<Owner>,
    pub title: String,
    pub version: u32,
    pub edited_by: Actor,
    pub created_at: i64,
    pub updated_at: i64,
    pub sources: Vec<Source>,
    pub sources_allowed: bool,
    pub condensed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub seq: u32,
    pub at: i64,
    pub change: Change,
    pub by: Actor,
    pub version: u32,
    pub owner: Option<String>,
    pub note: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Made {
    pub id: ArtifactId,
    pub version: u32,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Invalid {
    #[error("an artifact needs a `title`: what the page is, in a few words")]
    NoTitle,
    #[error("an artifact needs a `page`: the whole HTML document, with its own style and script")]
    NoPage,
    #[error(
        "the page is {bytes} bytes and an artifact holds at most {MAX_PAGE}. Draw less, or \
         compute what it shows in its own script instead of writing every value out"
    )]
    PageTooBig { bytes: usize },
    #[error(
        "the condensed view is {bytes} bytes and holds at most {MAX_CONDENSED}. It is one row of \
         the status bar: a number, a state or a next date. Put the rest in the page"
    )]
    CondensedTooBig { bytes: usize },
    #[error(
        "a condensed view cannot be empty. Leave `condensed` out to keep the one the page has"
    )]
    NoCondensed,
    #[error("{doing} needs a `note`: one line saying {what}")]
    NoNote { doing: &'static str, what: &'static str },
    #[error("a page may declare at most {MAX_SOURCES} sources; read fewer things, or fewer times")]
    TooManySources,
    #[error(
        "{0:?} is not a source name. Use lowercase letters, digits and underscores, at most \
         {MAX_SOURCE_NAME} characters, starting with a letter: `open_issues`"
    )]
    BadSourceName(String),
    #[error("two sources are called {0:?}; each needs a name of its own, because it is the key the page reads")]
    DuplicateSource(String),
    #[error(
        "{0:?} is not a connector tool. A source is one of your group's connector tools, named \
         the way you call it: `linear__list_issues`"
    )]
    NotAConnectorTool(String),
    #[error("the arguments for source {0:?} have to be a JSON object, like the ones you call the tool with")]
    SourceArgumentsNotObject(String),
    #[error(
        "the arguments for source {0:?} are over {MAX_SOURCE_ARGUMENTS} bytes. A source is a \
         query, not a payload"
    )]
    SourceArgumentsTooBig(String),
}

pub fn title(raw: &str) -> Result<(String, bool), Invalid> {
    let (kept, cut) = cut_to(raw, MAX_TITLE);
    if kept.is_empty() {
        return Err(Invalid::NoTitle);
    }
    Ok((kept, cut))
}

pub fn page(raw: &str) -> Result<String, Invalid> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(Invalid::NoPage);
    }
    if trimmed.len() > MAX_PAGE {
        return Err(Invalid::PageTooBig { bytes: trimmed.len() });
    }
    Ok(trimmed.to_string())
}

pub fn condensed(raw: &str) -> Result<String, Invalid> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(Invalid::NoCondensed);
    }
    if trimmed.len() > MAX_CONDENSED {
        return Err(Invalid::CondensedTooBig { bytes: trimmed.len() });
    }
    Ok(trimmed.to_string())
}

pub fn note(
    raw: Option<&str>,
    doing: &'static str,
    what: &'static str,
) -> Result<(String, bool), Invalid> {
    let (kept, cut) = cut_to(raw.unwrap_or_default(), MAX_NOTE);
    if kept.is_empty() {
        return Err(Invalid::NoNote { doing, what });
    }
    Ok((kept, cut))
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Read {
    pub name: String,
    pub data: Option<serde_json::Value>,
    pub text: Option<String>,
    pub error: Option<String>,
}

impl Read {
    pub fn answered(name: &str, text: String, structured: Option<serde_json::Value>) -> Read {
        let data = structured.or_else(|| serde_json::from_str(text.trim()).ok());
        Read { name: name.to_string(), data, text: Some(text), error: None }
    }

    pub fn refused(name: &str, why: &str) -> Read {
        Read { name: name.to_string(), data: None, text: None, error: Some(why.to_string()) }
    }
}

pub fn sources(declared: Vec<Source>) -> Result<Vec<Source>, Invalid> {
    if declared.len() > MAX_SOURCES {
        return Err(Invalid::TooManySources);
    }
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::with_capacity(declared.len());
    for Source { name, tool, arguments } in declared {
        let name = name.trim().to_string();
        let shaped = name.len() <= MAX_SOURCE_NAME
            && name.chars().next().is_some_and(|c| c.is_ascii_lowercase())
            && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
        if !shaped {
            return Err(Invalid::BadSourceName(name));
        }
        if !seen.insert(name.clone()) {
            return Err(Invalid::DuplicateSource(name));
        }
        let tool = tool.trim().to_string();
        match tool.split_once("__") {
            Some((server, call)) if !server.is_empty() && !call.is_empty() => {}
            _ => return Err(Invalid::NotAConnectorTool(tool)),
        }
        let arguments = match arguments {
            serde_json::Value::Null => serde_json::json!({}),
            object @ serde_json::Value::Object(_) => object,
            _ => return Err(Invalid::SourceArgumentsNotObject(name)),
        };
        if arguments.to_string().len() > MAX_SOURCE_ARGUMENTS {
            return Err(Invalid::SourceArgumentsTooBig(name));
        }
        out.push(Source { name, tool, arguments });
    }
    Ok(out)
}

impl Artifact {
    pub fn made(&self) -> Made {
        Made { id: self.id, version: self.version, title: self.title.clone() }
    }

    pub fn owner_words(&self, reader: AgentId) -> String {
        match &self.owner {
            None => "owned by nobody".to_string(),
            Some(owner) if owner.id == reader => "yours".to_string(),
            Some(owner) if owner.gone => {
                format!("owned by {}, who is no longer in this group", owner.name)
            }
            Some(owner) => format!("owned by {}", owner.name),
        }
    }

    pub fn index_line(&self, reader: AgentId, now: i64, pinned: bool) -> String {
        format!(
            "- {} \"{}\" v{}, {}, updated {} by {}{}",
            self.id,
            self.title,
            self.version,
            self.owner_words(reader),
            how_long_ago(self.updated_at, now),
            self.edited_by.said_to(reader),
            if pinned { ", on the operator's status bar" } else { "" },
        )
    }
}

pub fn log_lines(entries: &[Entry], reader: AgentId, now: i64) -> Vec<String> {
    let mut before: Option<&str> = None;
    let mut out = Vec::with_capacity(entries.len());
    for entry in entries {
        let who = entry.by.said_to(reader);
        let whose = |name: Option<&str>| match name {
            None => "nobody".to_string(),
            Some(name) => name.to_string(),
        };
        let what = match entry.change {
            Change::Created => format!("{who} created it"),
            Change::Edited => format!("{who} edited it"),
            Change::Restored => format!("{who} restored an earlier version"),
            Change::Took => format!("{who} took it over from {}", whose(before)),
            Change::Handed => format!(
                "{who} handed it from {} to {}",
                whose(before),
                whose(entry.owner.as_deref())
            ),
            Change::Allowed => format!("{who} allowed its reads"),
        };
        let mut line = format!("v{}, {}: {what}", entry.version, how_long_ago(entry.at, now));
        if !entry.note.is_empty() {
            line.push_str(&format!(". {}", entry.note));
        }
        out.push(line);
        before = entry.owner.as_deref();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(name: &str) -> Actor {
        Actor::Agent { id: AgentId::new(), name: name.to_string() }
    }

    fn entry(change: Change, by: Actor, version: u32, owner: Option<&str>, note: &str) -> Entry {
        Entry {
            seq: 0,
            at: 0,
            change,
            by,
            version,
            owner: owner.map(str::to_string),
            note: note.to_string(),
        }
    }

    #[test]
    fn a_page_over_the_cap_is_refused_whole_rather_than_cut() {
        let big = format!("<p>{}</p>", "x".repeat(MAX_PAGE));
        assert!(matches!(page(&big), Err(Invalid::PageTooBig { .. })));
        assert_eq!(page("  <p>hi</p>\n").unwrap(), "<p>hi</p>");
        assert_eq!(page("   "), Err(Invalid::NoPage));
    }

    #[test]
    fn a_condensed_view_is_held_to_a_strip_and_never_cut() {
        let page_sized = format!("<p>{}</p>", "x".repeat(MAX_CONDENSED));
        assert!(page(&page_sized).is_ok(), "fine as a page");
        assert!(matches!(condensed(&page_sized), Err(Invalid::CondensedTooBig { .. })));
        assert_eq!(condensed(" <b>3 open</b>\n").unwrap(), "<b>3 open</b>");
        assert_eq!(condensed("  "), Err(Invalid::NoCondensed));
    }

    #[test]
    fn an_edit_and_a_take_over_both_need_a_sentence() {
        assert!(matches!(note(None, "update", "what changed"), Err(Invalid::NoNote { .. })));
        assert!(matches!(note(Some("  "), "take", "why"), Err(Invalid::NoNote { .. })));
        let (kept, cut) = note(Some(&"word ".repeat(200)), "update", "what changed").unwrap();
        assert!(cut, "a cut note is handed back as cut");
        assert!(kept.chars().count() <= MAX_NOTE);
    }

    #[test]
    fn a_blank_title_is_refused_and_a_long_one_is_cut_and_says_so() {
        assert_eq!(title("  "), Err(Invalid::NoTitle));
        let (kept, cut) = title(&"Pipeline ".repeat(40)).unwrap();
        assert!(cut);
        assert!(kept.chars().count() <= MAX_TITLE);
    }

    #[test]
    fn every_change_reads_back_from_the_word_it_is_stored_as() {
        for change in [
            Change::Created,
            Change::Edited,
            Change::Restored,
            Change::Took,
            Change::Handed,
            Change::Allowed,
        ] {
            assert_eq!(Change::parse(change.as_str()), Some(change));
        }
        assert_eq!(Change::parse("deleted"), None);
    }

    fn source(name: &str, tool: &str, arguments: serde_json::Value) -> Source {
        Source { name: name.into(), tool: tool.into(), arguments }
    }

    #[test]
    fn a_source_has_to_be_a_connector_tool_under_a_name_a_script_can_read() {
        let bad = |one: Source| sources(vec![one]).unwrap_err();
        assert!(matches!(
            bad(source("open", "run_command", serde_json::json!({}))),
            Invalid::NotAConnectorTool(_)
        ));
        assert!(matches!(
            bad(source("Open issues", "linear__list_issues", serde_json::json!({}))),
            Invalid::BadSourceName(_)
        ));
        assert!(matches!(
            bad(source("open", "linear__list_issues", serde_json::json!(["x"]))),
            Invalid::SourceArgumentsNotObject(_)
        ));
        let big = serde_json::json!({ "q": "x".repeat(MAX_SOURCE_ARGUMENTS) });
        assert!(matches!(
            bad(source("open", "linear__list_issues", big)),
            Invalid::SourceArgumentsTooBig(_)
        ));
        let twice = vec![
            source("open", "linear__list_issues", serde_json::json!({})),
            source("open", "linear__list_projects", serde_json::json!({})),
        ];
        assert!(matches!(sources(twice), Err(Invalid::DuplicateSource(_))));
    }

    #[test]
    fn a_read_hands_the_page_data_whichever_way_the_server_answered() {
        let parsed = Read::answered("issues", " [{\"id\": 1}] ".into(), None);
        assert_eq!(parsed.data, Some(serde_json::json!([{ "id": 1 }])));
        let structured =
            Read::answered("issues", "1 issue".into(), Some(serde_json::json!({ "count": 1 })));
        assert_eq!(structured.data, Some(serde_json::json!({ "count": 1 })));
        let prose = Read::answered("issues", "One open issue.".into(), None);
        assert_eq!((prose.data, prose.text.as_deref()), (None, Some("One open issue.")));
    }

    #[test]
    fn a_tool_that_takes_nothing_is_called_with_an_empty_object() {
        let kept =
            sources(vec![source(" issues ", "linear__list_issues", serde_json::Value::Null)])
                .unwrap();
        assert_eq!(kept[0].name, "issues");
        assert_eq!(kept[0].arguments, serde_json::json!({}));
    }

    #[test]
    fn a_take_over_is_told_with_who_held_it_before() {
        let reader = AgentId::new();
        let lines = log_lines(
            &[
                entry(Change::Created, agent("Rae"), 1, Some("Rae"), "First cut"),
                entry(Change::Edited, agent("Milo"), 2, Some("Rae"), "Added a Q4 column"),
                entry(Change::Took, agent("Juno"), 2, Some("Juno"), "Rae moved to Ops"),
                entry(Change::Handed, Actor::Operator, 2, Some("Milo"), ""),
            ],
            reader,
            1_000,
        );
        assert_eq!(lines[1], "v2, just now: Milo edited it. Added a Q4 column");
        assert_eq!(lines[2], "v2, just now: Juno took it over from Rae. Rae moved to Ops");
        assert_eq!(lines[3], "v2, just now: the operator handed it from Juno to Milo");
    }

    #[test]
    fn an_agent_reads_its_own_work_as_its_own() {
        let me = AgentId::new();
        let mine = Artifact {
            id: ArtifactId::new(),
            group_id: GroupId::new(),
            owner: Some(Owner { id: me, name: "Rae".into(), gone: false }),
            title: "Pipeline by stage".into(),
            version: 7,
            edited_by: Actor::Agent { id: AgentId::new(), name: "Milo".into() },
            created_at: 0,
            updated_at: 0,
            sources: vec![],
            sources_allowed: false,
            condensed: false,
        };
        let line = mine.index_line(me, 2 * 3_600_000, false);
        assert!(
            line.ends_with("\"Pipeline by stage\" v7, yours, updated 2h ago by Milo"),
            "{line}"
        );
        let pinned = mine.index_line(me, 2 * 3_600_000, true);
        assert!(pinned.ends_with("by Milo, on the operator's status bar"), "{pinned}");
    }

    #[test]
    fn an_owner_who_left_is_still_named_and_said_to_be_gone() {
        let gone = Artifact {
            id: ArtifactId::new(),
            group_id: GroupId::new(),
            owner: Some(Owner { id: AgentId::new(), name: "Rae".into(), gone: true }),
            title: "Plan".into(),
            version: 1,
            edited_by: Actor::Operator,
            created_at: 0,
            updated_at: 0,
            sources: vec![],
            sources_allowed: false,
            condensed: false,
        };
        assert_eq!(
            gone.owner_words(AgentId::new()),
            "owned by Rae, who is no longer in this group"
        );
    }
}
