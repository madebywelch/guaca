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

/// How many files a skill may carry beside its `SKILL.md`. The largest in the
/// top forty on skills.sh carried 195 (September 2026), mostly references an
/// agent reads one at a time.
pub const MAX_FILES: usize = 256;

/// All of a skill's files together. The same survey's largest was 1.2 MB.
pub const MAX_BUNDLE: usize = 4 * 1024 * 1024;

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
    /// What sits beside `SKILL.md`, as paths relative to it: references,
    /// templates, scripts. Read one at a time; nothing here runs a script.
    #[serde(default)]
    pub files: Vec<String>,
    /// Where it was added from, as a page a person can open. `None` for one
    /// written here or dropped in by hand.
    #[serde(default)]
    pub origin: Option<String>,
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
    #[error(
        "`{0}` is not a file beside a skill. Name one the way the skill lists it: a relative \
         path such as `references/api.md`"
    )]
    BadFile(String),
    #[error("there is no `SKILL.md` at the top of this skill, so there is nothing to follow")]
    NoSkillFile,
    #[error(
        "this skill carries {0} files beside its `SKILL.md` and the limit is {MAX_FILES}. It is \
         a library rather than a skill; add a smaller one"
    )]
    TooManyFiles(usize),
    #[error(
        "this skill's files come to {0} bytes and the limit is {MAX_BUNDLE}. Add a smaller one"
    )]
    TooLarge(usize),
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

/// Whether a path can name a file beside a skill's `SKILL.md`.
///
/// Relative, forward slashes, and inside: a skill's files are read by a path a
/// model types, so `../` is a way out of the skill and a leading `/` is a way
/// out of everything. A component starting with a dot is refused as well,
/// because the dot names in a skill's directory are Guaca's own bookkeeping.
pub fn check_file(path: &str) -> Result<(), SkillError> {
    let bad = || SkillError::BadFile(path.to_string());
    if path.is_empty()
        || path.len() > 512
        || path == "SKILL.md"
        || path.chars().any(|c| c == '\\' || c == ':' || c.is_control())
    {
        return Err(bad());
    }
    for part in path.split('/') {
        if part.is_empty() || part.starts_with('.') {
            return Err(bad());
        }
    }
    Ok(())
}

/// A skill written somewhere else, checked and ready to put in a scope whole:
/// its `SKILL.md` exactly as its author wrote it, and every file beside it.
///
/// Checked differently from [`Clean`], and on purpose. `Clean` is a skill being
/// written here, so it is held to the line a prompt can carry. A package is
/// somebody else's file, and more than half of the forty most installed on
/// skills.sh describe themselves in more than [`MAX_DESCRIPTION`] characters,
/// so the description is accepted at any length and cut where it is read, as a
/// hand-written file's is. The body is not: a procedure cut short is a
/// different procedure, and the operator should hear that before it is added.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Package {
    /// The directory it will live in, which is its name everywhere here.
    pub name: String,
    pub description: String,
    pub body: String,
    /// `SKILL.md` as it arrived, front matter and all, so the license and
    /// author lines its author wrote travel with it.
    pub text: String,
    /// Everything beside it, sorted by path.
    pub files: Vec<(String, String)>,
    /// SHA-256 over every path and its contents. What the operator read is
    /// what gets added: the add is refused when the hash moved in between.
    pub hash: String,
}

impl Package {
    /// Checks a skill as `(path, contents)` pairs, named by `slug` because
    /// that is the name its directory had where it came from.
    ///
    /// Files under a dot-named directory (`.github/`, `.gitignore`) are left
    /// behind rather than refused: they are the repository's, not the skill's.
    pub fn new(slug: &str, files: Vec<(String, String)>) -> Result<Self, SkillError> {
        let name = if check_name(slug).is_ok() { slug.to_string() } else { self::slug(slug) };
        if RESERVED.contains(&name.as_str()) {
            return Err(SkillError::Reserved(name));
        }
        let total: usize = files.iter().map(|(path, contents)| path.len() + contents.len()).sum();
        if total > MAX_BUNDLE {
            return Err(SkillError::TooLarge(total));
        }

        let mut text = None;
        let mut beside: Vec<(String, String)> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for (path, contents) in files {
            let path = path.trim_start_matches("./").to_string();
            if path == "SKILL.md" {
                text.get_or_insert(contents);
                continue;
            }
            // `..` is not a dot name but a way out, and is refused below.
            let dotted = |part: &str| part.starts_with('.') && part != "." && part != "..";
            if path.split('/').any(dotted) {
                continue;
            }
            check_file(&path)?;
            // Once per path as a case-insensitive disk sees it, or the second
            // quietly overwrites the first on the operator's Mac.
            if seen.insert(path.to_lowercase()) {
                beside.push((path, contents));
            }
        }
        let text = text.ok_or(SkillError::NoSkillFile)?;
        if beside.len() > MAX_FILES {
            return Err(SkillError::TooManyFiles(beside.len()));
        }
        beside.sort_by(|a, b| a.0.cmp(&b.0));

        let (_, description, body) = parse(&text)?;
        let description = description.split_whitespace().collect::<Vec<_>>().join(" ");
        if description.is_empty() {
            return Err(SkillError::NoDescription);
        }
        if body.is_empty() {
            return Err(SkillError::NoBody);
        }
        let length = body.chars().count();
        if length > MAX_BODY {
            return Err(SkillError::LongBody(length));
        }

        use sha2::{Digest, Sha256};
        let mut digest = Sha256::new();
        for (path, contents) in std::iter::once(("SKILL.md", text.as_str()))
            .chain(beside.iter().map(|(p, c)| (p.as_str(), c.as_str())))
        {
            digest.update(path.as_bytes());
            digest.update([0]);
            digest.update(contents.as_bytes());
            digest.update([0]);
        }
        let hash = format!("{:x}", digest.finalize());
        Ok(Self { name, description, body, text, files: beside, hash })
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

    fn file(path: &str, contents: &str) -> (String, String) {
        (path.to_string(), contents.to_string())
    }

    const SKILL_MD: &str =
        "---\nname: Nice Name\ndescription: When it fits\nlicense: MIT\n---\n# Do it\n";

    #[test]
    fn a_file_beside_a_skill_is_named_from_inside_it() {
        for bad in [
            "",
            "SKILL.md",
            "/etc/passwd",
            "../x.md",
            "a/../../x",
            "a//b",
            "a\\b",
            "C:x",
            ".origin",
            "refs/.hidden",
            "a\nb",
        ] {
            assert!(matches!(check_file(bad), Err(SkillError::BadFile(_))), "{bad:?}");
        }
        for good in ["reference.md", "references/api.md", "scripts/fill form.py", "a/SKILL.md"] {
            assert!(check_file(good).is_ok(), "{good}");
        }
    }

    #[test]
    fn a_package_refuses_what_it_cannot_carry_and_says_so() {
        assert_eq!(Package::new("x", vec![file("README.md", "hi")]), Err(SkillError::NoSkillFile));
        assert_eq!(
            Package::new("guaca", vec![file("SKILL.md", SKILL_MD)]),
            Err(SkillError::Reserved("guaca".into()))
        );
        let traversal = Package::new("x", vec![file("SKILL.md", SKILL_MD), file("../up", "x")]);
        assert_eq!(traversal, Err(SkillError::BadFile("../up".into())));
        let long = format!("---\nname: x\ndescription: d\n---\n{}", "w".repeat(MAX_BODY + 1));
        assert_eq!(
            Package::new("x", vec![file("SKILL.md", &long)]),
            Err(SkillError::LongBody(MAX_BODY + 1))
        );
        let empty = "---\nname: x\ndescription: d\n---\n";
        assert_eq!(Package::new("x", vec![file("SKILL.md", empty)]), Err(SkillError::NoBody));
        let many: Vec<_> = std::iter::once(file("SKILL.md", SKILL_MD))
            .chain((0..=MAX_FILES).map(|n| file(&format!("r/{n}.md"), "x")))
            .collect();
        assert_eq!(Package::new("x", many), Err(SkillError::TooManyFiles(MAX_FILES + 1)));
        let heavy = vec![file("SKILL.md", SKILL_MD), file("big.txt", &"x".repeat(MAX_BUNDLE))];
        assert!(matches!(Package::new("x", heavy), Err(SkillError::TooLarge(_))));
    }

    #[test]
    fn a_package_keeps_its_authors_file_and_leaves_the_repositorys_behind() {
        let long = "w ".repeat(MAX_DESCRIPTION);
        let text = format!("---\nname: pdf\ndescription: {long}\nlicense: MIT\n---\n# Do it\n");
        let package = Package::new(
            "Fancy_Name",
            vec![
                file("scripts/run.py", "print()"),
                file("./SKILL.md", &text),
                file(".github/workflows/ci.yml", "on: push"),
                file("README.md", "one"),
                file("readme.md", "the same file on a Mac"),
            ],
        )
        .unwrap();
        assert_eq!(package.name, "fancy-name", "the directory is a name a model can type");
        assert_eq!(package.text, text, "front matter survives, license and all");
        assert!(package.description.chars().count() > MAX_DESCRIPTION, "cut where it is read");
        let paths: Vec<_> = package.files.iter().map(|(path, _)| path.as_str()).collect();
        assert_eq!(paths, ["README.md", "scripts/run.py"]);
    }

    #[test]
    fn a_package_hash_moves_with_any_file_and_not_with_arrival_order() {
        let one = Package::new("x", vec![file("SKILL.md", SKILL_MD), file("a.md", "a")]).unwrap();
        let again = Package::new("x", vec![file("a.md", "a"), file("SKILL.md", SKILL_MD)]).unwrap();
        let edited =
            Package::new("x", vec![file("SKILL.md", SKILL_MD), file("a.md", "b")]).unwrap();
        let moved = Package::new("x", vec![file("SKILL.md", SKILL_MD), file("b.md", "a")]).unwrap();
        assert_eq!(one.hash, again.hash);
        assert_ne!(one.hash, edited.hash);
        assert_ne!(one.hash, moved.hash);
        assert_eq!(one.hash.len(), 64);
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
