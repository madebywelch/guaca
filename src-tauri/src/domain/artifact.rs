//! An artifact: a page a crew keeps.
//!
//! A page in an ```html fence is drawn once, inside the reply that carried it,
//! and leaves the operator's view when the conversation moves on. That is right
//! for a picker or a one-off diagram and wrong for the thing an operator comes
//! back to: the pipeline board, the hiring plan, the comparison they will want
//! again next week. So an artifact is the same kind of page with an address. It
//! belongs to a crew, it is listed in that crew's Artifacts where the operator
//! opens it, and every change to it is a version with a sentence saying what
//! changed.
//!
//! What runs is exactly what a fenced page runs: the same loopback origin in
//! `crate::artifact`, the same content policy, reaching nothing. Keeping a page
//! changes where it lives, not what it may do.
//!
//! ## Anyone in the crew edits it, and one agent owns it
//!
//! The two are separate on purpose. Editing is open to every agent in the crew
//! because a crew works on shared things, and an artifact only its author could
//! touch would be one the rest of them rebuild beside it. Ownership is who
//! answers for it: the name on the list, the agent the others ask about it, the
//! one whose prompt marks it as its own to keep current.
//!
//! So ownership changes only by a deliberate act. An agent takes one over with
//! a reason, and the operator can hand one to any agent in the crew. Nothing
//! moves it implicitly, not an edit, not the owner leaving: an owner that has
//! been deleted or moved to another crew is still named, and said to be gone,
//! until somebody decides to take it.
//!
//! An edit by somebody else is *read* by the owner, from its own list, never
//! sent to it. A message would start a turn, which is paid for, and an owner
//! woken by every edit edits back and wakes the editor: a cascade the guard
//! would then have to stop. The log is there to be read.
//!
//! ## The log is the record, and it is written in the words of the moment
//!
//! Every row names who acted and who owned the page afterward by the names they
//! had then, the way a routine's chip keeps the routine's name at the moment it
//! fired. An agent renamed or deleted later does not rewrite what the log says
//! happened, and a log that joined names in at read time would say that a
//! deleted agent's edits were made by nobody.
//!
//! The rows that made a version carry the whole page, which is what lets the
//! operator open any earlier one and put it back. The rows that did not, taking
//! and handing, carry none: ownership is not content.
//!
//! ## Live data is declared, allowed, and then read without a model
//!
//! A page can say what it reads: a connector tool and the exact arguments to
//! call it with, fixed when the page is written. The operator allows that list
//! once, and from then on Guaca makes those calls every time the page is
//! opened, as the page's owner, and hands the results to the page. No model is
//! involved in a read, so opening a board costs a connector call and nothing
//! else.
//!
//! The arguments being fixed is the whole of the safety argument. A page is
//! the least trusted content in the app and it reaches no network, so the only
//! way anything it read could leave is through something it can say. A read
//! whose arguments the page chose would be one: a search whose query is the
//! row it just read is a message to whoever runs the search. A page that can
//! only replay reads somebody approved has nothing to say with. Changing the
//! list is a new version, and a new version's reads wait for the operator
//! again; the approval is of an exact list, compared as stored.
//!
//! ## Whose it is
//!
//! A crew's, enforced where the calendar enforces it: every read and write an
//! agent makes is scoped to the group on its own card, in the WHERE clause, and
//! an id from another crew is not refused but *not found*. The operator stands
//! above that wall, as they do for the calendar.

use serde::{Deserialize, Serialize};

use super::cut_to;
use super::ids::{AgentId, ArtifactId, GroupId};
use super::worknote::how_long_ago;

/// How long a title may be. The line in a list, not a description.
pub const MAX_TITLE: usize = 120;

/// The sentence that goes with a change. One line in a log, like a commit
/// message: what changed and, where it matters, why.
pub const MAX_NOTE: usize = 300;

/// The most a page may be, in bytes.
///
/// Also what the frame server will hold for one, and the same number rather
/// than two that agree today: a page kept here that the server then refused to
/// frame would be a version nobody can open.
pub const MAX_PAGE: usize = 512 * 1024;

/// How many a prompt names. The list is on every turn of every agent in the
/// crew, so it is capped rather than scrolled; `list` has the rest.
pub const LISTED: usize = 20;

/// How much of the log `view` hands back, newest first.
pub const LOG_SHOWN: usize = 20;

/// How many reads one page may declare. A board reads a handful of things; a
/// page declaring dozens is a page doing an agent's job on every open.
pub const MAX_SOURCES: usize = 8;

/// How long a source's name may be. It is a key the page's script reads, so it
/// is an identifier rather than a description.
pub const MAX_SOURCE_NAME: usize = 32;

/// The most one source's arguments may be, serialized. Arguments are a query,
/// not a payload.
pub const MAX_SOURCE_ARGUMENTS: usize = 4 * 1024;

/// One read a page declares: a connector tool, and exactly what to send it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Source {
    /// The key the page reads the result under.
    pub name: String,
    /// A connector's tool, named as an agent calls it: `linear__list_issues`.
    pub tool: String,
    /// Fixed when the page is written. See the module note for why the page
    /// can never choose these.
    pub arguments: serde_json::Value,
}

/// Who did something to an artifact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Actor {
    Operator,
    /// An agent, by the name it had at the time.
    Agent {
        id: AgentId,
        name: String,
    },
}

impl Actor {
    pub fn agent_id(&self) -> Option<AgentId> {
        match self {
            Actor::Operator => None,
            Actor::Agent { id, .. } => Some(*id),
        }
    }

    /// The name stored beside the id. Empty for the operator, whose row carries
    /// no id and needs no name.
    pub fn stored_name(&self) -> &str {
        match self {
            Actor::Operator => "",
            Actor::Agent { name, .. } => name,
        }
    }

    /// How an agent reading about this is told who did it.
    fn said_to(&self, reader: AgentId) -> String {
        match self {
            Actor::Operator => "the operator".to_string(),
            Actor::Agent { id, .. } if *id == reader => "you".to_string(),
            Actor::Agent { name, .. } => name.clone(),
        }
    }
}

/// What happened, one row of the log.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Change {
    Created,
    Edited,
    /// An earlier version put back, as a new version. History is never
    /// rewound: what was there before the restore is still a version.
    Restored,
    /// An agent took ownership, with a reason.
    Took,
    /// The operator gave ownership to an agent.
    Handed,
    /// The operator allowed the reads the current version declares.
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

    /// Whether a row of this kind made a version, and so carries a page. The
    /// table's CHECK constraint says the same thing, and a test holds the two
    /// together.
    pub fn makes_version(self) -> bool {
        matches!(self, Change::Created | Change::Edited | Change::Restored)
    }
}

/// Who answers for an artifact, as the list reads it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Owner {
    pub id: AgentId,
    pub name: String,
    /// Deleted, or moved to another crew. Still the owner until somebody takes
    /// it: see the module note.
    pub gone: bool,
}

/// One artifact, as the list shows it. The page itself is read separately,
/// since a list of forty would otherwise carry forty pages.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Artifact {
    pub id: ArtifactId,
    pub group_id: GroupId,
    /// `None` only once the owning agent's row is gone for good.
    pub owner: Option<Owner>,
    pub title: String,
    /// The current version. Starts at 1 and only goes up.
    pub version: u32,
    /// Who made the current version.
    pub edited_by: Actor,
    pub created_at: i64,
    /// When the current version was made. Ownership changes do not move it:
    /// "last updated" is about the page.
    pub updated_at: i64,
    /// The reads the current version declares. Empty for a page with no live
    /// data, which is most of them.
    pub sources: Vec<Source>,
    /// Whether the operator allowed exactly these reads. Never true for a page
    /// with none.
    pub sources_allowed: bool,
}

/// One row of an artifact's log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub seq: u32,
    pub at: i64,
    pub change: Change,
    pub by: Actor,
    /// The version once this was done. An ownership change leaves it where it
    /// was.
    pub version: u32,
    /// Who owned it once this was done, by the name they had then. `None` is
    /// nobody.
    pub owner: Option<String>,
    pub note: String,
}

/// What a tool call made or changed, recorded on the call so the transcript can
/// draw a card for it. The title is the one it had at that moment, as a
/// routine's chip keeps the routine's name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Made {
    pub id: ArtifactId,
    pub version: u32,
    pub title: String,
}

/// Why a write was not taken. Each says what to send instead.
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
        "{0:?} is not a connector tool. A source is one of your crew's connector tools, named \
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

/// A title, trimmed and cut. The flag is handed back to whoever wrote it.
pub fn title(raw: &str) -> Result<(String, bool), Invalid> {
    let (kept, cut) = cut_to(raw, MAX_TITLE);
    if kept.is_empty() {
        return Err(Invalid::NoTitle);
    }
    Ok((kept, cut))
}

/// A page, whole. Never cut: half a document is a broken one, so a page over
/// the cap is refused rather than trimmed.
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

/// The sentence that goes with an edit or a take-over, which is required on
/// both: an edit nobody explained is a version nobody can choose between, and
/// ownership taken without a reason is the one change the log exists to
/// explain.
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

/// What one of a page's reads came back with, as the page is handed it.
///
/// Three fields rather than an enum, because they are what the page's script
/// reads and the tool description names them: `data` is JSON when the tool
/// answered with some, `text` is what it said, and `error` is why there is
/// neither. A server that answered with `structuredContent` gave data outright;
/// one that wrote JSON as text is parsed, because that is how most of them
/// answer.
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

/// A page's declared reads, checked for shape.
///
/// Whether the crew has that connector and its owner may call that tool is a
/// question for the store, asked where the refusal can say what to do about it.
/// This is only what a list of reads has to be to mean anything: names a
/// script can use, no two alike, and arguments that are an object of a sane
/// size. Absent arguments are an empty object, which is what a tool that
/// takes none is called with.
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
    /// The reference a tool call records, for the card.
    pub fn made(&self) -> Made {
        Made { id: self.id, version: self.version, title: self.title.clone() }
    }

    /// Who owns it, as an agent reading its crew's list is told.
    pub fn owner_words(&self, reader: AgentId) -> String {
        match &self.owner {
            None => "owned by nobody".to_string(),
            Some(owner) if owner.id == reader => "yours".to_string(),
            Some(owner) if owner.gone => {
                format!("owned by {}, who is no longer in this crew", owner.name)
            }
            Some(owner) => format!("owned by {}", owner.name),
        }
    }

    /// One line in the prompt and in `list`. Owner and last editor both, so an
    /// owner reads that somebody else changed its page without being told.
    pub fn index_line(&self, reader: AgentId, now: i64) -> String {
        format!(
            "- {} \"{}\" v{}, {}, updated {} by {}",
            self.id,
            self.title,
            self.version,
            self.owner_words(reader),
            how_long_ago(self.updated_at, now),
            self.edited_by.said_to(reader),
        )
    }
}

/// The log as lines, oldest first, for an agent.
///
/// Ownership changes are said with who held it before, which no single row
/// records: it is the owner on the row before. That is why this walks the whole
/// log in order rather than rendering rows one at a time.
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
        // Half an HTML document is a broken one. Cutting it the way a title is
        // cut would store a version that draws a blank frame.
        let big = format!("<p>{}</p>", "x".repeat(MAX_PAGE));
        assert!(matches!(page(&big), Err(Invalid::PageTooBig { .. })));
        assert_eq!(page("  <p>hi</p>\n").unwrap(), "<p>hi</p>");
        assert_eq!(page("   "), Err(Invalid::NoPage));
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
        // Most servers write their JSON as text. A page should not have to
        // know which kind of server it is reading from.
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
        // No row records the previous owner. It is the owner on the row
        // before, and a log rendered row by row would say "took it over from"
        // and stop.
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
        };
        let line = mine.index_line(me, 2 * 3_600_000);
        assert!(line.contains("\"Pipeline by stage\" v7, yours, updated 2h ago by Milo"), "{line}");
    }

    #[test]
    fn an_owner_who_left_is_still_named_and_said_to_be_gone() {
        // Ownership moves only by a decision. An owner that left is not
        // silently replaced, so the list has to say that it left.
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
        };
        assert_eq!(gone.owner_words(AgentId::new()), "owned by Rae, who is no longer in this crew");
    }
}
