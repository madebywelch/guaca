use serde::{Deserialize, Serialize};

use super::ids::GroupId;

pub const MAX_NAME: usize = 64;

pub const MAX_DESCRIPTION: usize = 300;

pub const MAX_BODY: usize = 48_000;

pub const LISTED: usize = 40;

pub const MAX_PER_SCOPE: usize = 100;

pub const MAX_FILES: usize = 256;

pub const MAX_BUNDLE: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Scope {
    Bundled,
    Workspace,
    Crew { group_id: GroupId },
}

impl Scope {
    pub fn label(self) -> &'static str {
        match self {
            Scope::Bundled => "Guaca",
            Scope::Workspace => "operator",
            Scope::Crew { .. } => "your group",
        }
    }

    pub fn owner(self) -> &'static str {
        match self {
            Scope::Bundled => "Guaca",
            Scope::Workspace => "the operator",
            Scope::Crew { .. } => "your group",
        }
    }

    pub fn precedence(self) -> u8 {
        match self {
            Scope::Crew { .. } => 0,
            Scope::Workspace => 1,
            Scope::Bundled => 2,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub scope: Scope,
    #[serde(default)]
    pub body: String,
    pub updated_at: i64,
    #[serde(default)]
    pub files: Vec<String>,
    #[serde(default)]
    pub origin: Option<String>,
}

impl Skill {
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

pub const RESERVED: &[&str] = &["guaca"];

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Clean {
    pub name: String,
    pub description: String,
    pub body: String,
}

impl Clean {
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

    pub fn render(&self) -> String {
        let quoted =
            serde_json::to_string(&self.description).unwrap_or_else(|_| "\"\"".to_string());
        format!("---\nname: {}\ndescription: {quoted}\n---\n\n{}\n", self.name, self.body)
    }
}

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Package {
    pub name: String,
    pub description: String,
    pub body: String,
    pub text: String,
    pub files: Vec<(String, String)>,
    pub hash: String,
}

impl Package {
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
            let dotted = |part: &str| part.starts_with('.') && part != "." && part != "..";
            if path.split('/').any(dotted) {
                continue;
            }
            check_file(&path)?;
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

fn field(front: &[&str], key: &str) -> Option<String> {
    let prefix = format!("{key}:");
    let at = front.iter().position(|line| line.starts_with(&prefix))?;
    let value = front[at][prefix.len()..].trim();
    if let Some(style) = value.chars().next().filter(|c| *c == '>' || *c == '|') {
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
        for bad in ["", "Deploy", "deploy site", "../etc", "a/b", "-x", "x-", "é", "a.b"] {
            let refused = check_name(bad).unwrap_err();
            assert!(matches!(refused, SkillError::BadName { .. }), "{bad}");
        }
        assert!(check_name(&"a".repeat(MAX_NAME + 1)).is_err());
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
        let wire = serde_json::to_value(crew).unwrap();
        assert_eq!(wire["kind"], "crew");
        assert!(wire.get("groupId").is_some(), "{wire}");
    }
}
