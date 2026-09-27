//! A skill: a markdown document of instructions for one kind of task, read
//! when a task fits it.
//!
//! Memory is what one agent knows, and it is in every prompt. A skill is how a
//! kind of work is done, and it is not: only its name and the one line saying
//! when it applies are, and the body is read with the `skill` tool when a task
//! fits. That split is the whole design. A crew with forty procedures cannot
//! carry forty procedures on every turn, and a model told "these exist, load
//! the one that fits" reads the one it needs.
//!
//! The format is the `SKILL.md` other harnesses already read: front matter with
//! `name` and `description`, then markdown. An operator can write one in any
//! editor and drop it in the directory, and a skill written here can be carried
//! to Claude Code or Hermes without translation.
//!
//! ## Three places, and who may write each
//!
//! A skill is instructions another agent will follow, which makes where it
//! lives a trust decision rather than a filing one:
//!
//! - **Bundled** skills ship in the binary and nobody writes them. `guaca` is
//!   one: the app, described to the agents that run in it.
//! - **Workspace** skills are the operator's, and every crew reads them.
//! - **Crew** skills belong to one crew, and its own agents may write them.
//!
//! An agent writes only to its own crew. A skill one crew's agent could put in
//! front of another crew's agents is a way to hand instructions across the wall
//! every other crew-scoped thing here keeps, and an agent that has just read a
//! hostile page is the likeliest author of one.

use serde::{Deserialize, Serialize};

use super::ids::GroupId;

/// A name is a directory and a word a model types, so it is short and plain.
pub const MAX_NAME: usize = 64;

/// The line that says when to load it. In every prompt of every agent that can
/// see it, so it is a sentence, not the skill.
pub const MAX_DESCRIPTION: usize = 300;

/// The body, read on demand. Long enough for a real procedure with its
/// commands and caveats; past it, a skill is two skills.
pub const MAX_BODY: usize = 48_000;

/// How many a prompt names before it says how many more `list` would show.
pub const LISTED: usize = 40;

/// How many one scope holds. A crew whose agents write a skill a turn has a
/// transcript, not a library, and every one of them is a line in every prompt.
pub const MAX_PER_SCOPE: usize = 100;

/// Where a skill lives, which is who may write it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Scope {
    /// Compiled into Guaca. Nobody writes it.
    Bundled,
    /// The operator's, read by every crew.
    Workspace,
    /// One crew's, written by its operator or its own agents.
    Crew { group_id: GroupId },
}

impl Scope {
    /// How the scope reads in a list a model is shown.
    pub fn label(self) -> &'static str {
        match self {
            Scope::Bundled => "Guaca",
            Scope::Workspace => "operator",
            Scope::Crew { .. } => "your crew",
        }
    }

    /// Who a skill in this scope belongs to, in a sentence.
    pub fn owner(self) -> &'static str {
        match self {
            Scope::Bundled => "Guaca",
            Scope::Workspace => "the operator",
            Scope::Crew { .. } => "your crew",
        }
    }

    /// Which of two same-named skills an agent sees: its crew's own over the
    /// operator's, and the operator's over nothing. Bundled names are reserved,
    /// so nothing competes with them.
    pub fn precedence(self) -> u8 {
        match self {
            Scope::Crew { .. } => 0,
            Scope::Workspace => 1,
            Scope::Bundled => 2,
        }
    }
}

/// One skill, with or without its body.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub scope: Scope,
    /// Empty in a listing. The body is read when the skill is opened.
    #[serde(default)]
    pub body: String,
    /// Milliseconds since the epoch. Zero for a bundled skill, which has no
    /// moment it was written on this machine.
    pub updated_at: i64,
}

impl Skill {
    /// The line a prompt carries for it.
    pub fn index_line(&self) -> String {
        format!("- {} ({}): {}", self.name, self.scope.label(), self.description)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SkillError {
    #[error(
        "`{given}` is not a skill name. A name is lowercase letters, digits and dashes, at most \
         {MAX_NAME} characters; `{suggested}` would do"
    )]
    BadName { given: String, suggested: String },
    #[error("`{0}` is Guaca's own skill and is read-only. Write yours under another name")]
    Reserved(String),
    #[error("a skill needs a description: one line saying when an agent should load it")]
    NoDescription,
    #[error(
        "the description is {0} characters and the limit is {MAX_DESCRIPTION}. Say when to load \
         it in one line, and put the rest in the body"
    )]
    LongDescription(usize),
    #[error("a skill needs a body: the instructions an agent should follow")]
    NoBody,
    #[error(
        "the skill is {0} characters and the limit is {MAX_BODY}. Split it into two skills, each \
         with its own description"
    )]
    LongBody(usize),
    #[error("this file is not a skill ({0}). A skill starts with `---`, then `name:` and `description:` lines, then `---` and the body")]
    Unreadable(String),
    #[error(
        "this scope already holds {MAX_PER_SCOPE} skills. Delete one that is no longer used, or \
         fold two that overlap into one, before writing another"
    )]
    Full,
}

/// Names Guaca ships, which nothing else may take.
pub const RESERVED: &[&str] = &["guaca"];

/// Whether a name can be a skill's, and what to use instead when it cannot.
pub fn check_name(name: &str) -> Result<(), SkillError> {
    let valid = !name.is_empty()
        && name.len() <= MAX_NAME
        && name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !name.starts_with('-')
        && !name.ends_with('-');
    if valid {
        Ok(())
    } else {
        Err(SkillError::BadName { given: name.to_string(), suggested: slug(name) })
    }
}

/// A usable name made from whatever was typed.
pub fn slug(raw: &str) -> String {
    let mut out = String::new();
    for ch in raw.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
        if out.len() >= MAX_NAME {
            break;
        }
    }
    let trimmed = out.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "skill".to_string()
    } else {
        trimmed
    }
}

/// A skill that is fit to store: named, described in a line, and within size.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Clean {
    pub name: String,
    pub description: String,
    pub body: String,
}

impl Clean {
    /// Checks a skill someone is writing. Reserved names are refused here, so
    /// no path that writes can skip it.
    pub fn new(name: &str, description: &str, body: &str) -> Result<Self, SkillError> {
        let name = name.trim();
        check_name(name)?;
        if RESERVED.contains(&name) {
            return Err(SkillError::Reserved(name.to_string()));
        }
        let description = description.split_whitespace().collect::<Vec<_>>().join(" ");
        if description.is_empty() {
            return Err(SkillError::NoDescription);
        }
        let length = description.chars().count();
        if length > MAX_DESCRIPTION {
            return Err(SkillError::LongDescription(length));
        }
        let body = body.trim();
        if body.is_empty() {
            return Err(SkillError::NoBody);
        }
        let length = body.chars().count();
        if length > MAX_BODY {
            return Err(SkillError::LongBody(length));
        }
        Ok(Self { name: name.to_string(), description, body: body.to_string() })
    }

    /// The file, in the format other harnesses read.
    ///
    /// The description is written as a quoted string. A plain YAML scalar ends
    /// at the first `: ` or starts something else at a leading `-`, `#` or
    /// quote, and a description is prose that contains all of those.
    pub fn render(&self) -> String {
        let quoted =
            serde_json::to_string(&self.description).unwrap_or_else(|_| "\"\"".to_string());
        format!("---\nname: {}\ndescription: {quoted}\n---\n\n{}\n", self.name, self.body)
    }
}

/// Reads a `SKILL.md`: its name, its description and its body.
///
/// Front matter is read, not interpreted as YAML. Only `name` and
/// `description` mean anything here, and the three spellings a hand-written
/// file uses for them are handled: plain, quoted, and folded over lines with
/// `>` or `|`. Every other key is kept in the file and ignored.
pub fn parse(text: &str) -> Result<(String, String, String), SkillError> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut lines = text.lines();
    if lines.next().map(str::trim_end) != Some("---") {
        return Err(SkillError::Unreadable("no front matter".into()));
    }
    let mut front: Vec<&str> = Vec::new();
    let mut closed = false;
    for line in lines.by_ref() {
        if line.trim_end() == "---" {
            closed = true;
            break;
        }
        front.push(line);
    }
    if !closed {
        return Err(SkillError::Unreadable("front matter never ends".into()));
    }
    let body = lines.collect::<Vec<_>>().join("\n").trim().to_string();
    let name = field(&front, "name").ok_or(SkillError::Unreadable("no name".into()))?;
    let description =
        field(&front, "description").ok_or(SkillError::Unreadable("no description".into()))?;
    Ok((name, description, body))
}

/// One top-level key's value in front matter.
fn field(front: &[&str], key: &str) -> Option<String> {
    let prefix = format!("{key}:");
    let at = front.iter().position(|line| line.starts_with(&prefix))?;
    let value = front[at][prefix.len()..].trim();
    if let Some(style) = value.chars().next().filter(|c| *c == '>' || *c == '|') {
        // A block: every following line that is indented, or blank inside it.
        let mut block: Vec<&str> = Vec::new();
        for line in &front[at + 1..] {
            if line.trim().is_empty() || line.starts_with(' ') || line.starts_with('\t') {
                block.push(line.trim());
            } else {
                break;
            }
        }
        let joined = if style == '>' { block.join(" ") } else { block.join("\n") };
        return Some(joined.trim().to_string());
    }
    if value.starts_with('"') {
        return serde_json::from_str::<String>(value).ok();
    }
    if let Some(inner) = value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')) {
        return Some(inner.replace("''", "'"));
    }
    Some(value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_is_a_word_a_model_can_type_and_a_directory_can_hold() {
        for good in ["deploy", "deploy-site", "q3-close", "a"] {
            assert!(check_name(good).is_ok(), "{good}");
        }
        // Separators and dots are refused because a name is a directory: `..`
        // or `a/b` would write outside the scope it was given.
        for bad in ["", "Deploy", "deploy site", "../etc", "a/b", "-x", "x-", "é", "a.b"] {
            let refused = check_name(bad).unwrap_err();
            assert!(matches!(refused, SkillError::BadName { .. }), "{bad}");
        }
        assert!(check_name(&"a".repeat(MAX_NAME + 1)).is_err());
        // The refusal says what would work, so the retry is right the first time.
        assert!(check_name("Deploy Site").unwrap_err().to_string().contains("`deploy-site`"));
    }

    #[test]
    fn guacas_own_name_cannot_be_taken() {
        assert_eq!(Clean::new("guaca", "x", "y"), Err(SkillError::Reserved("guaca".into())));
    }

    #[test]
    fn a_description_is_one_line_and_a_body_has_to_exist() {
        let clean = Clean::new("x", "  When   deploying\n the site ", "Run it.").unwrap();
        assert_eq!(clean.description, "When deploying the site");
        assert_eq!(Clean::new("x", " ", "y"), Err(SkillError::NoDescription));
        assert_eq!(Clean::new("x", "y", "\n "), Err(SkillError::NoBody));
        let long = "w".repeat(MAX_DESCRIPTION + 1);
        assert_eq!(
            Clean::new("x", &long, "y"),
            Err(SkillError::LongDescription(MAX_DESCRIPTION + 1))
        );
        let body = "w".repeat(MAX_BODY + 1);
        assert_eq!(Clean::new("x", "y", &body), Err(SkillError::LongBody(MAX_BODY + 1)));
    }

    #[test]
    fn what_is_written_reads_back_whatever_the_description_holds() {
        let description = "Use when: a \"release\" is cut - # not a comment, 'quoted'";
        let clean = Clean::new("release", description, "# Steps\n\n1. Tag it.").unwrap();
        let (name, read, body) = parse(&clean.render()).unwrap();
        assert_eq!(name, "release");
        assert_eq!(read, description);
        assert_eq!(body, "# Steps\n\n1. Tag it.");
    }

    #[test]
    fn a_hand_written_file_is_read_in_the_spellings_people_use() {
        let plain = "---\nname: notes\ndescription: When taking notes\nlicense: MIT\n---\nBody";
        assert_eq!(parse(plain).unwrap().1, "When taking notes");
        let single = "---\nname: n\ndescription: 'It''s for notes'\n---\nBody";
        assert_eq!(parse(single).unwrap().1, "It's for notes");
        let folded = "---\nname: n\ndescription: >\n  Folded over\n  two lines\nother: x\n---\nB";
        assert_eq!(parse(folded).unwrap().1, "Folded over two lines");
        let bom = "\u{feff}---\r\nname: n\r\ndescription: d\r\n---\r\nB";
        assert_eq!(parse(bom).unwrap().0, "n");
    }

    #[test]
    fn a_file_that_is_not_a_skill_says_what_one_looks_like() {
        for text in ["# Just markdown", "---\nname: n\n", "---\nname: n\n---\nno description"] {
            let refused = parse(text).unwrap_err();
            assert!(refused.to_string().contains("`description:`"), "{text}: {refused}");
        }
    }

    #[test]
    fn a_crew_skill_shadows_the_operators_and_nothing_shadows_guacas() {
        let crew = Scope::Crew { group_id: GroupId::new() };
        assert!(crew.precedence() < Scope::Workspace.precedence());
        assert!(Scope::Workspace.precedence() < Scope::Bundled.precedence());
        // What crosses IPC names its field the way the frontend spells it.
        let wire = serde_json::to_value(crew).unwrap();
        assert_eq!(wire["kind"], "crew");
        assert!(wire.get("groupId").is_some(), "{wire}");
    }
}
