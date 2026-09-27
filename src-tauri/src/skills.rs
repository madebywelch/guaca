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
//!   crews/<group id>/<name>/SKILL.md one crew's
//! ```
//!
//! Bundled skills are compiled in and never touch the disk, so a host that
//! was updated serves the manual of the build it runs rather than a copy an
//! older build left behind.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::domain::ids::GroupId;
use crate::domain::skill::{self, Clean, Scope, Skill, SkillError, MAX_BODY, MAX_DESCRIPTION};

/// Skills that ship with Guaca, as `(name, file)`.
const BUNDLED: &[(&str, &str)] = &[("guaca", include_str!("../skills/guaca/SKILL.md"))];

#[derive(Debug, thiserror::Error)]
pub enum SkillsError {
    #[error("no skill called `{0}` is visible here. `list` shows the ones that are")]
    NotFound(String),
    #[error("`{0}` is Guaca's own skill and is read-only")]
    ReadOnly(String),
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
        // The directory is the name. It is what every write and delete
        // addresses, and a file that says otherwise is still found by it.
        Ok(Skill { name: name.to_string(), description, scope, body, updated_at })
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
