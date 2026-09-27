//! Skills on disk: one directory per skill, holding its `SKILL.md`.
//!
//! Files rather than rows, for the reason memory is a file: a skill is a
//! document a person may want to write in their own editor, carry to another
//! harness, or drop into the volume of a box, and a table would make every one
//! of those an export. `domain::skill` holds the rules; this holds the disk.
//!
//! ```text
//! skills/
//!   workspace/<name>/SKILL.md        the operator's, read by every crew
//!   workspace/<name>/references/...  anything else the skill carries
//!   workspace/<name>/.origin         where it was added from, if anywhere
//!   crews/<group id>/<name>/SKILL.md one crew's
//!   .defaults                        the starter skills already offered
//! ```
//!
//! Bundled skills are compiled in and never touch the disk, so a host that
//! was updated serves the manual of the build it runs rather than a copy an
//! older build left behind. The starter skills are the opposite: copied into
//! the operator's scope once, because they are the operator's to edit or
//! delete, and a deleted one stays deleted.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::domain::ids::GroupId;
use crate::domain::skill::{
    self, Clean, Package, Scope, Skill, SkillError, MAX_BODY, MAX_DESCRIPTION, MAX_FILES,
};

/// Skills that ship with Guaca, as `(name, file)`.
const BUNDLED: &[(&str, &str)] = &[("guaca", include_str!("../skills/guaca/SKILL.md"))];

/// The license the starter skills came under, carried beside each one so it
/// travels wherever the skill is copied.
const STARTER_LICENSE: &str = include_str!("../skills/defaults/LICENSE");

macro_rules! starter {
    ($name:literal) => {
        (
            $name,
            &[
                ("SKILL.md", include_str!(concat!("../skills/defaults/", $name, "/SKILL.md"))),
                ("LICENSE.txt", STARTER_LICENSE),
            ],
        )
    };
}

/// What every workspace starts with, adapted from Hermes Agent's own
/// (`skills/defaults/LICENSE` says which, and at what commit). Kept to general
/// procedure: nothing that names a program an agent here cannot run, a
/// platform it is not on, or a tool it does not have.
pub const STARTERS: &[(&str, &[(&str, &str)])] = &[
    starter!("grounded-citations"),
    starter!("grill-me"),
    starter!("one-three-one"),
    starter!("systematic-debugging"),
    starter!("document-to-action-items"),
    starter!("meeting-action-items"),
];

/// Where a skill added from elsewhere says it came from.
const ORIGIN: &str = ".origin";

/// The starter skills a workspace has already been given, one name a line.
const OFFERED: &str = ".defaults";

#[derive(Debug, thiserror::Error)]
pub enum SkillsError {
    #[error("no skill called `{0}` is visible here. `list` shows the ones that are")]
    NotFound(String),
    #[error("`{0}` is Guaca's own skill and is read-only")]
    ReadOnly(String),
    #[error(
        "a skill called `{0}` is already here. Delete it or rename it first, then add this one"
    )]
    Taken(String),
    #[error("`{name}` has no file `{path}`. `view` the skill to see the files it carries")]
    NoFile { name: String, path: String },
    #[error(
        "`{name}` belongs to {owner}, not your crew, so it cannot be deleted from here. Leave \
         it, or ask the operator"
    )]
    NotYours { name: String, owner: &'static str },
    #[error(transparent)]
    Invalid(#[from] SkillError),
    #[error("could not access the skill at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

fn io(path: &Path) -> impl FnOnce(io::Error) -> SkillsError + '_ {
    move |source| SkillsError::Io { path: path.to_path_buf(), source }
}

#[derive(Debug, Clone)]
pub struct Skills {
    root: PathBuf,
}

impl Skills {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    fn dir(&self, scope: Scope) -> Option<PathBuf> {
        match scope {
            Scope::Bundled => None,
            Scope::Workspace => Some(self.root.join("workspace")),
            Scope::Crew { group_id } => Some(self.root.join("crews").join(group_id.to_string())),
        }
    }

    /// The skills compiled into this build.
    pub fn bundled() -> Vec<Skill> {
        BUNDLED
            .iter()
            .map(|(name, text)| {
                let (_, description, body) =
                    skill::parse(text).expect("a bundled skill is checked by its test");
                Skill {
                    name: name.to_string(),
                    description,
                    scope: Scope::Bundled,
                    body,
                    updated_at: 0,
                    files: Vec::new(),
                    origin: None,
                }
            })
            .collect()
    }

    /// Every skill in one scope, in name order, without bodies.
    pub fn in_scope(&self, scope: Scope) -> Vec<Skill> {
        let Some(dir) = self.dir(scope) else {
            return Self::bundled().into_iter().map(without_body).collect();
        };
        let Ok(entries) = fs::read_dir(&dir) else {
            return Vec::new();
        };
        let mut found: Vec<Skill> = entries
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| {
                let name = entry.file_name().to_str()?.to_string();
                skill::check_name(&name).ok()?;
                match self.load(scope, &name) {
                    Ok(found) => Some(without_body(found)),
                    Err(err) => {
                        // Somebody's hand-written file. Skipped rather than
                        // failing the whole list, and said where it can be read.
                        tracing::warn!(%err, skill = %name, "a skill could not be read");
                        None
                    }
                }
            })
            .collect();
        found.sort_by(|a, b| a.name.cmp(&b.name));
        found
    }

    /// What an agent in `group` can read, one per name: its crew's own over
    /// the operator's, and Guaca's own beside both.
    pub fn visible(&self, group: GroupId) -> Vec<Skill> {
        let mut all = self.in_scope(Scope::Crew { group_id: group });
        all.extend(self.in_scope(Scope::Workspace));
        all.extend(self.in_scope(Scope::Bundled));
        all.sort_by(|a, b| {
            a.name.cmp(&b.name).then(a.scope.precedence().cmp(&b.scope.precedence()))
        });
        all.dedup_by(|later, first| later.name == first.name);
        all
    }

    /// One skill with its body, resolved the way `visible` resolves names.
    /// `None` reads as the operator does, without a crew.
    pub fn read(&self, group: Option<GroupId>, name: &str) -> Result<Skill, SkillsError> {
        let mut scopes = Vec::with_capacity(3);
        if let Some(group_id) = group {
            scopes.push(Scope::Crew { group_id });
        }
        scopes.extend([Scope::Workspace, Scope::Bundled]);
        for scope in scopes {
            match self.load(scope, name) {
                Err(SkillsError::NotFound(_)) => continue,
                other => return other,
            }
        }
        Err(SkillsError::NotFound(name.to_string()))
    }

    /// One skill in exactly one scope.
    pub fn load(&self, scope: Scope, name: &str) -> Result<Skill, SkillsError> {
        skill::check_name(name)?;
        let Some(dir) = self.dir(scope) else {
            return Self::bundled()
                .into_iter()
                .find(|found| found.name == name)
                .ok_or_else(|| SkillsError::NotFound(name.to_string()));
        };
        let path = dir.join(name).join("SKILL.md");
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                return Err(SkillsError::NotFound(name.to_string()))
            }
            Err(err) => return Err(io(&path)(err)),
        };
        let (_, description, body) = skill::parse(&text)?;
        // Read leniently: a file somebody wrote by hand past a limit is still
        // served, cut, rather than hidden. Only a write is refused for size.
        let (description, _) = crate::domain::cut_to(&description, MAX_DESCRIPTION);
        let (body, cut) = crate::domain::cut_to(&body, MAX_BODY);
        let body = if cut { format!("{body}\n\n[The rest of this skill was cut.]") } else { body };
        let updated_at = fs::metadata(&path)
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|at| at.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|since| since.as_millis() as i64)
            .unwrap_or(0);
        let folder = dir.join(name);
        let origin = fs::read_to_string(folder.join(ORIGIN))
            .ok()
            .map(|text| text.trim().to_string())
            .filter(|text| !text.is_empty());
        // The directory is the name. It is what every write and delete
        // addresses, and a file that says otherwise is still found by it.
        Ok(Skill {
            name: name.to_string(),
            description,
            scope,
            body,
            updated_at,
            files: beside(&folder),
            origin,
        })
    }

    /// One file a skill carries, resolved the way [`Skills::read`] resolves
    /// the skill. Served cut past [`MAX_BODY`], like a body.
    pub fn read_file(
        &self,
        group: Option<GroupId>,
        name: &str,
        path: &str,
    ) -> Result<String, SkillsError> {
        let found = self.read(group, name)?;
        let path = path.trim().trim_start_matches("./");
        skill::check_file(path)?;
        let no_file = || SkillsError::NoFile { name: found.name.clone(), path: path.to_string() };
        // Listed means readable: a path that is not in the listing is refused
        // here rather than resolved on disk, so a symlink a skill carried in
        // cannot point a read somewhere else.
        if !found.files.iter().any(|listed| listed == path) {
            return Err(no_file());
        }
        let dir = self.dir(found.scope).ok_or_else(no_file)?;
        let full = dir.join(&found.name).join(path);
        let text = match fs::read(&full) {
            Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Err(no_file()),
            Err(err) => return Err(io(&full)(err)),
        };
        let (text, cut) = crate::domain::cut_to(&text, MAX_BODY);
        Ok(if cut { format!("{text}\n\n[The rest of this file was cut.]") } else { text })
    }

    /// Creates or replaces one skill in one scope.
    pub fn write(&self, scope: Scope, clean: &Clean) -> Result<Skill, SkillsError> {
        let Some(dir) = self.dir(scope) else {
            return Err(SkillsError::ReadOnly(clean.name.clone()));
        };
        let folder = dir.join(&clean.name);
        if !folder.exists() && self.in_scope(scope).len() >= skill::MAX_PER_SCOPE {
            return Err(SkillError::Full.into());
        }
        fs::create_dir_all(&folder).map_err(io(&folder))?;
        let path = folder.join("SKILL.md");
        let temp = folder.join("SKILL.md.tmp");
        fs::write(&temp, clean.render()).map_err(io(&temp))?;
        fs::rename(&temp, &path).map_err(io(&path))?;
        self.load(scope, &clean.name)
    }

    /// Puts a skill written elsewhere into one scope, whole: its `SKILL.md` as
    /// its author wrote it, every file beside it, and where it came from.
    ///
    /// Assembled in a hidden directory and renamed into place, so a crew never
    /// reads a skill whose references are half written, and a failure leaves
    /// nothing behind. Never over one already there: an operator who meant to
    /// replace one deletes it first, and one who did not mean to keeps theirs.
    pub fn install(
        &self,
        scope: Scope,
        package: &Package,
        origin: Option<&str>,
    ) -> Result<Skill, SkillsError> {
        let Some(dir) = self.dir(scope) else {
            return Err(SkillsError::ReadOnly(package.name.clone()));
        };
        let folder = dir.join(&package.name);
        if folder.exists() {
            return Err(SkillsError::Taken(package.name.clone()));
        }
        if self.in_scope(scope).len() >= skill::MAX_PER_SCOPE {
            return Err(SkillError::Full.into());
        }
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|since| since.as_nanos())
            .unwrap_or(0);
        let staging = dir.join(format!(".incoming-{}-{stamp}", package.name));
        let assembled = (|| {
            let mut entries: Vec<(&str, &str)> = vec![("SKILL.md", package.text.as_str())];
            entries.extend(package.files.iter().map(|(p, c)| (p.as_str(), c.as_str())));
            if let Some(origin) = origin {
                entries.push((ORIGIN, origin));
            }
            for (path, contents) in entries {
                let target = staging.join(path);
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent).map_err(io(parent))?;
                }
                fs::write(&target, contents).map_err(io(&target))?;
            }
            fs::rename(&staging, &folder).map_err(io(&folder))
        })();
        if let Err(err) = assembled {
            let _ = fs::remove_dir_all(&staging);
            return Err(err);
        }
        self.load(scope, &package.name)
    }

    /// Moves one skill to another name in the same scope, with everything it
    /// carries. False when the new name is taken, which leaves both alone.
    pub fn rename(&self, scope: Scope, from: &str, to: &str) -> Result<bool, SkillsError> {
        skill::check_name(from)?;
        skill::check_name(to)?;
        let Some(dir) = self.dir(scope) else {
            return Err(SkillsError::ReadOnly(from.to_string()));
        };
        let (old, new) = (dir.join(from), dir.join(to));
        if new.exists() || !old.exists() {
            return Ok(false);
        }
        fs::rename(&old, &new).map_err(io(&new))?;
        Ok(true)
    }

    /// Copies each starter skill into the operator's scope the first time a
    /// workspace is opened by a build that ships it, and never again.
    ///
    /// A name is offered once. One the operator deleted is not put back on
    /// the next start, and one they already had under that name is left as
    /// theirs rather than overwritten. Returns the names added.
    pub fn offer(&self, starters: &[(&str, &[(&str, &str)])]) -> Result<Vec<String>, SkillsError> {
        let ledger = self.root.join(OFFERED);
        let mut offered: Vec<String> = match fs::read_to_string(&ledger) {
            Ok(text) => {
                text.lines().map(str::trim).filter(|l| !l.is_empty()).map(String::from).collect()
            }
            Err(err) if err.kind() == io::ErrorKind::NotFound => Vec::new(),
            Err(err) => return Err(io(&ledger)(err)),
        };
        let mut added = Vec::new();
        for (name, files) in starters {
            if offered.iter().any(|done| done == name) {
                continue;
            }
            let files = files.iter().map(|(p, c)| (p.to_string(), c.to_string())).collect();
            let package = Package::new(name, files)?;
            match self.install(Scope::Workspace, &package, None) {
                Ok(_) => added.push(name.to_string()),
                // The operator's own under that name, or a scope with no room:
                // theirs either way, and not asked again.
                Err(SkillsError::Taken(_)) | Err(SkillsError::Invalid(SkillError::Full)) => {}
                Err(err) => return Err(err),
            }
            offered.push(name.to_string());
        }
        fs::create_dir_all(&self.root).map_err(io(&self.root))?;
        let mut text = offered.join("\n");
        text.push('\n');
        let temp = self.root.join(".defaults.tmp");
        fs::write(&temp, text).map_err(io(&temp))?;
        fs::rename(&temp, &ledger).map_err(io(&ledger))?;
        Ok(added)
    }

    /// Removes one skill from one scope. False when there was none.
    pub fn delete(&self, scope: Scope, name: &str) -> Result<bool, SkillsError> {
        skill::check_name(name)?;
        let Some(dir) = self.dir(scope) else {
            return Err(SkillsError::ReadOnly(name.to_string()));
        };
        let folder = dir.join(name);
        match fs::remove_dir_all(&folder) {
            Ok(()) => Ok(true),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(err) => Err(io(&folder)(err)),
        }
    }

    /// Everything one crew wrote, when the crew is gone.
    pub fn forget_crew(&self, group: GroupId) {
        if let Some(dir) = self.dir(Scope::Crew { group_id: group }) {
            if let Err(err) = fs::remove_dir_all(&dir) {
                if err.kind() != io::ErrorKind::NotFound {
                    tracing::warn!(%err, dir = %dir.display(), "could not remove a crew's skills");
                }
            }
        }
    }
}

fn without_body(mut found: Skill) -> Skill {
    found.body = String::new();
    found
}

/// Every file under a skill's directory but its `SKILL.md` and Guaca's own dot
/// names, as sorted relative paths. Regular files only: a symlink is not
/// followed, so nothing a skill carries can list a file outside it.
fn beside(folder: &Path) -> Vec<String> {
    let mut found = Vec::new();
    let mut pending = vec![(folder.to_path_buf(), String::new())];
    while let Some((dir, prefix)) = pending.pop() {
        let Ok(entries) = fs::read_dir(&dir) else { continue };
        for entry in entries.filter_map(|entry| entry.ok()) {
            let Some(part) = entry.file_name().to_str().map(str::to_string) else { continue };
            if part.starts_with('.') {
                continue;
            }
            let path = format!("{prefix}{part}");
            match entry.file_type() {
                Ok(kind) if kind.is_dir() => pending.push((entry.path(), format!("{path}/"))),
                Ok(kind) if kind.is_file() && path != "SKILL.md" => found.push(path),
                _ => {}
            }
        }
        if found.len() > MAX_FILES {
            break;
        }
    }
    found.sort();
    found.truncate(MAX_FILES);
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (Skills, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        (Skills::new(dir.path().join("skills")), dir)
    }

    fn clean(name: &str, description: &str) -> Clean {
        Clean::new(name, description, "Do the thing.").unwrap()
    }

    #[test]
    fn every_bundled_skill_reads_and_guaca_is_one_of_them() {
        let bundled = Skills::bundled();
        assert!(bundled.iter().any(|found| found.name == "guaca"));
        for found in &bundled {
            assert!(!found.description.is_empty(), "{}", found.name);
            assert!(found.description.chars().count() <= MAX_DESCRIPTION, "{}", found.name);
            assert!(found.body.chars().count() <= MAX_BODY, "{}", found.name);
            assert!(
                skill::RESERVED.contains(&found.name.as_str()),
                "{} is not reserved",
                found.name
            );
        }
    }

    #[test]
    fn every_starter_is_a_skill_this_build_can_hold_and_names_no_tool_it_does_not_have() {
        for (name, files) in STARTERS {
            let files = files.iter().map(|(p, c)| (p.to_string(), c.to_string())).collect();
            let package = Package::new(name, files).unwrap_or_else(|err| panic!("{name}: {err}"));
            assert_eq!(package.name, *name);
            assert!(
                package.description.chars().count() <= MAX_DESCRIPTION,
                "{name}: a starter's description is read whole, so it fits the line"
            );
            assert!(package.description.starts_with("Use "), "{name} says when to load it");
            let (_, _, body) = skill::parse(&package.text).unwrap();
            // Written for another harness first. Each of these is a tool or a
            // word from there that a model here would try to call or look for.
            for foreign in [
                "web_search",
                "web_extract",
                "browser_navigate",
                "delegate_task",
                "execute_code",
                "search_files",
                "skill_view",
                "terminal(",
                "Hermes",
                "hermes",
                "scripts/",
                "\u{2014}",
            ] {
                assert!(!body.contains(foreign), "{name} mentions `{foreign}`");
            }
        }
    }

    #[test]
    fn a_crew_sees_its_own_the_operators_and_guacas_and_never_another_crews() {
        let (skills, _dir) = store();
        let ours = GroupId::new();
        let theirs = GroupId::new();
        skills.write(Scope::Workspace, &clean("deploy", "the operator's")).unwrap();
        skills.write(Scope::Crew { group_id: ours }, &clean("deploy", "ours")).unwrap();
        skills.write(Scope::Crew { group_id: theirs }, &clean("secret-plan", "theirs")).unwrap();

        let seen = skills.visible(ours);
        let names: Vec<_> = seen.iter().map(|found| found.name.as_str()).collect();
        assert_eq!(names, ["deploy", "guaca"]);
        assert_eq!(seen[0].description, "ours", "the crew's own shadows the operator's");
        assert!(seen.iter().all(|found| found.body.is_empty()), "a listing carries no bodies");

        assert!(matches!(skills.read(Some(ours), "secret-plan"), Err(SkillsError::NotFound(_))));
        assert_eq!(skills.read(Some(theirs), "deploy").unwrap().description, "the operator's");
        assert_eq!(skills.read(None, "deploy").unwrap().description, "the operator's");
        assert!(skills.read(Some(ours), "guaca").unwrap().body.contains("Guaca"));
    }

    #[test]
    fn nothing_is_written_to_guacas_scope_or_outside_its_own_directory() {
        let (skills, dir) = store();
        assert!(matches!(
            skills.write(Scope::Bundled, &clean("mine", "x")),
            Err(SkillsError::ReadOnly(_))
        ));
        assert!(matches!(skills.delete(Scope::Bundled, "guaca"), Err(SkillsError::ReadOnly(_))));
        assert!(matches!(skills.delete(Scope::Workspace, "../.."), Err(SkillsError::Invalid(_))));
        assert!(dir.path().exists(), "a traversal must not reach the directory above");
    }

    #[test]
    fn a_hand_written_file_is_listed_and_a_broken_one_is_skipped() {
        let (skills, _dir) = store();
        let root = skills.dir(Scope::Workspace).unwrap();
        fs::create_dir_all(root.join("notes")).unwrap();
        fs::write(
            root.join("notes/SKILL.md"),
            "---\nname: notes\ndescription: >\n  When taking\n  notes\n---\n# Notes\n",
        )
        .unwrap();
        fs::create_dir_all(root.join("broken")).unwrap();
        fs::write(root.join("broken/SKILL.md"), "no front matter").unwrap();
        fs::create_dir_all(root.join("Not A Name")).unwrap();

        let listed = skills.in_scope(Scope::Workspace);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].description, "When taking notes");
        assert!(listed[0].updated_at > 0);
    }

    fn package(name: &str, files: &[(&str, &str)]) -> Package {
        let text =
            format!("---\nname: {name}\ndescription: when {name}\nlicense: MIT\n---\n# {name}\n");
        let mut all = vec![("SKILL.md".to_string(), text)];
        all.extend(files.iter().map(|(p, c)| (p.to_string(), c.to_string())));
        Package::new(name, all).unwrap()
    }

    #[test]
    fn an_install_never_lands_on_a_skill_already_there_and_leaves_nothing_when_refused() {
        let (skills, _dir) = store();
        skills.write(Scope::Workspace, &clean("pdf", "mine")).unwrap();
        let refused = skills.install(Scope::Workspace, &package("pdf", &[]), None).unwrap_err();
        assert!(matches!(refused, SkillsError::Taken(ref name) if name == "pdf"), "{refused}");
        assert_eq!(skills.load(Scope::Workspace, "pdf").unwrap().description, "mine");
        assert!(matches!(
            skills.install(Scope::Bundled, &package("x", &[]), None),
            Err(SkillsError::ReadOnly(_))
        ));
        let root = skills.dir(Scope::Workspace).unwrap();
        let leftovers: Vec<_> = fs::read_dir(&root)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(leftovers, ["pdf"], "no staging directory is left behind");
    }

    #[test]
    fn an_installed_skill_carries_its_files_its_origin_and_its_authors_front_matter() {
        let (skills, _dir) = store();
        let pdf = package("pdf", &[("reference.md", "# Ref"), ("scripts/fill.py", "print()")]);
        let origin = "https://skills.sh/anthropics/skills/pdf";
        let added = skills.install(Scope::Workspace, &pdf, Some(origin)).unwrap();
        assert_eq!(added.files, ["reference.md", "scripts/fill.py"]);
        assert_eq!(added.origin.as_deref(), Some(origin));
        let on_disk =
            fs::read_to_string(skills.dir(Scope::Workspace).unwrap().join("pdf/SKILL.md"));
        assert!(on_disk.unwrap().contains("license: MIT"));
        // Listed with the rest, and its files never counted as skills.
        assert_eq!(skills.in_scope(Scope::Workspace).len(), 1);
    }

    #[test]
    fn a_file_is_read_only_from_a_skill_that_lists_it() {
        let (skills, dir) = store();
        let ours = GroupId::new();
        let pdf = package("pdf", &[("reference.md", "# Ref")]);
        skills.install(Scope::Crew { group_id: ours }, &pdf, None).unwrap();
        fs::write(dir.path().join("secret.txt"), "not yours").unwrap();

        assert_eq!(skills.read_file(Some(ours), "pdf", "reference.md").unwrap(), "# Ref");
        assert_eq!(skills.read_file(Some(ours), "pdf", "./reference.md").unwrap(), "# Ref");
        for (path, expect_invalid) in
            [("../../../secret.txt", true), (".origin", true), ("missing.md", false)]
        {
            let refused = skills.read_file(Some(ours), "pdf", path).unwrap_err();
            let invalid = matches!(refused, SkillsError::Invalid(SkillError::BadFile(_)));
            let missing = matches!(refused, SkillsError::NoFile { .. });
            assert!(if expect_invalid { invalid } else { missing }, "{path}: {refused}");
        }
        // Another crew does not see the skill at all.
        let other = skills.read_file(Some(GroupId::new()), "pdf", "reference.md").unwrap_err();
        assert!(matches!(other, SkillsError::NotFound(_)), "{other}");
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_a_skill_carries_is_neither_listed_nor_read() {
        let (skills, dir) = store();
        skills.install(Scope::Workspace, &package("pdf", &[]), None).unwrap();
        fs::write(dir.path().join("secret.txt"), "not yours").unwrap();
        let folder = skills.dir(Scope::Workspace).unwrap().join("pdf");
        std::os::unix::fs::symlink(dir.path().join("secret.txt"), folder.join("link.md")).unwrap();
        assert!(skills.load(Scope::Workspace, "pdf").unwrap().files.is_empty());
        assert!(matches!(
            skills.read_file(None, "pdf", "link.md"),
            Err(SkillsError::NoFile { .. })
        ));
    }

    #[test]
    fn a_rename_carries_the_files_and_never_lands_on_another_skill() {
        let (skills, _dir) = store();
        skills.install(Scope::Workspace, &package("pdf", &[("a.md", "a")]), None).unwrap();
        skills.write(Scope::Workspace, &clean("taken", "mine")).unwrap();
        assert!(!skills.rename(Scope::Workspace, "pdf", "taken").unwrap());
        assert_eq!(skills.load(Scope::Workspace, "taken").unwrap().description, "mine");
        assert!(skills.rename(Scope::Workspace, "pdf", "documents").unwrap());
        assert_eq!(skills.load(Scope::Workspace, "documents").unwrap().files, ["a.md"]);
        assert!(matches!(skills.load(Scope::Workspace, "pdf"), Err(SkillsError::NotFound(_))));
    }

    const STARTER: &str = "---\nname: cite\ndescription: When citing\n---\nCite it.";

    #[test]
    fn a_starter_skill_is_offered_once_and_never_over_the_operators_own() {
        let (skills, _dir) = store();
        skills.write(Scope::Workspace, &clean("mine", "the operator's")).unwrap();
        let first: &[(&str, &[(&str, &str)])] = &[
            ("cite", &[("SKILL.md", STARTER), ("LICENSE.txt", "MIT")]),
            ("mine", &[("SKILL.md", STARTER)]),
        ];
        assert_eq!(skills.offer(first).unwrap(), ["cite"]);
        assert_eq!(skills.load(Scope::Workspace, "cite").unwrap().files, ["LICENSE.txt"]);
        assert_eq!(skills.load(Scope::Workspace, "mine").unwrap().description, "the operator's");

        // Deleted by the operator, and a later start does not put it back.
        skills.delete(Scope::Workspace, "cite").unwrap();
        assert!(skills.offer(first).unwrap().is_empty());
        assert!(matches!(skills.load(Scope::Workspace, "cite"), Err(SkillsError::NotFound(_))));

        // A later build that ships one more offers only the new one.
        let later: &[(&str, &[(&str, &str)])] =
            &[("cite", &[("SKILL.md", STARTER)]), ("grill", &[("SKILL.md", STARTER)])];
        assert_eq!(skills.offer(later).unwrap(), ["grill"]);
    }

    #[test]
    fn a_rewrite_replaces_a_delete_removes_and_a_full_scope_refuses() {
        let (skills, _dir) = store();
        let scope = Scope::Crew { group_id: GroupId::new() };
        skills.write(scope, &clean("a", "first")).unwrap();
        skills.write(scope, &clean("a", "second")).unwrap();
        assert_eq!(skills.in_scope(scope).len(), 1);
        assert_eq!(skills.load(scope, "a").unwrap().description, "second");
        assert!(skills.delete(scope, "a").unwrap());
        assert!(!skills.delete(scope, "a").unwrap());

        for n in 0..skill::MAX_PER_SCOPE {
            skills.write(scope, &clean(&format!("s{n}"), "x")).unwrap();
        }
        let refused = skills.write(scope, &clean("one-more", "x")).unwrap_err();
        assert!(matches!(refused, SkillsError::Invalid(SkillError::Full)), "{refused}");
        // Rewriting one already there is not growth.
        assert!(skills.write(scope, &clean("s0", "again")).is_ok());
    }
}
