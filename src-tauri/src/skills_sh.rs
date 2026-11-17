use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::domain::skill::{Package, Scope, Skill, SkillError};
use crate::skills::{Skills, SkillsError};

const BASE: &str = "https://skills.sh";

const TIMEOUT: Duration = Duration::from_secs(20);

const SEARCH_LIMIT: usize = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Board {
    AllTime,
    Trending,
    Hot,
}

impl Board {
    fn segment(self) -> &'static str {
        match self {
            Board::AllTime => "all-time",
            Board::Trending => "trending",
            Board::Hot => "hot",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Listing {
    pub id: String,
    pub source: String,
    pub slug: String,
    pub name: String,
    pub installs: u64,
    pub url: String,
    pub addable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Page {
    pub skills: Vec<Listing>,
    pub page: u32,
    pub has_more: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Audit {
    pub provider: String,
    pub verdict: Verdict,
    pub summary: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Verdict {
    Pass,
    Warn,
    Fail,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    pub id: String,
    pub name: String,
    pub description: String,
    pub body: String,
    pub files: Vec<String>,
    pub hash: String,
    pub url: String,
    pub audits: Option<Vec<Audit>>,
}

#[derive(Debug, thiserror::Error)]
pub enum DirectoryError {
    #[error("skills.sh could not be reached ({0}). Check the connection and try again")]
    Unreachable(String),
    #[error("skills.sh is busy and asked for a pause. Try again in a minute")]
    Busy,
    #[error(
        "skills.sh answered {0}. Try again later; if it keeps happening, skills.sh may have \
         changed how it answers"
    )]
    Status(u16),
    #[error("skills.sh has no skill `{0}`. It may have been removed; search for it again")]
    Missing(String),
    #[error(
        "`{0}` is published by its own site rather than a GitHub repository, and Guaca only adds \
         skills that skills.sh keeps a copy of. Open its page on skills.sh to read it"
    )]
    NotAddable(String),
    #[error(
        "skills.sh answered in a shape Guaca does not recognize ({0}). It may have changed; try \
         again after updating Guaca"
    )]
    Unexpected(String),
    #[error("type at least two characters to search skills.sh")]
    ShortQuery,
    #[error(
        "this skill changed on skills.sh after you opened it. Open it again and read what \
         changed before adding it"
    )]
    Changed,
    #[error(transparent)]
    Invalid(#[from] SkillError),
    #[error(transparent)]
    Store(#[from] SkillsError),
}

fn github_id(id: &str) -> Result<[&str; 3], DirectoryError> {
    let parts: Vec<&str> = id.trim().split('/').collect();
    let plain = |part: &&str| {
        !part.is_empty()
            && *part != "."
            && *part != ".."
            && part.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
    };
    match parts.as_slice() {
        [owner, repo, slug] if [owner, repo, slug].iter().all(|part| plain(part)) => {
            Ok([owner, repo, slug])
        }
        [site, _] if site.contains('.') => Err(DirectoryError::NotAddable(id.trim().to_string())),
        _ => Err(DirectoryError::Missing(id.trim().to_string())),
    }
}

#[derive(Debug, Clone)]
pub struct Directory {
    http: reqwest::Client,
    base: String,
}

impl Default for Directory {
    fn default() -> Self {
        Self::new()
    }
}

impl Directory {
    pub fn new() -> Self {
        Self::at(BASE)
    }

    pub fn at(base: &str) -> Self {
        let http = reqwest::Client::builder()
            .timeout(TIMEOUT)
            .user_agent(concat!("Guaca/", env!("CARGO_PKG_VERSION")))
            .build()
            .unwrap_or_default();
        Self { http, base: base.trim_end_matches('/').to_string() }
    }

    async fn get(&self, url: &str) -> Result<Option<Value>, DirectoryError> {
        let response = self.http.get(url).send().await.map_err(|err| {
            tracing::warn!(%err, %url, "skills.sh could not be reached");
            DirectoryError::Unreachable(if err.is_timeout() {
                "it did not answer in time".to_string()
            } else {
                "the request failed".to_string()
            })
        })?;
        let status = response.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS
            || status == reqwest::StatusCode::SERVICE_UNAVAILABLE
        {
            tracing::warn!(%url, %status, "skills.sh asked for a pause");
            return Err(DirectoryError::Busy);
        }
        if !status.is_success() {
            tracing::warn!(%url, %status, "skills.sh refused a request");
            return Err(DirectoryError::Status(status.as_u16()));
        }
        let value = response.json::<Value>().await.map_err(|err| {
            tracing::warn!(%err, %url, "skills.sh answered with something other than JSON");
            DirectoryError::Unexpected("the answer was not JSON".to_string())
        })?;
        Ok(Some(value))
    }

    pub async fn board(&self, board: Board, page: u32) -> Result<Page, DirectoryError> {
        let url = format!("{}/api/skills/{}/{page}", self.base, board.segment());
        let value = self
            .get(&url)
            .await?
            .ok_or_else(|| DirectoryError::Unexpected("the ranking is gone".into()))?;
        let skills = listings(&value)?;
        let has_more = value.get("hasMore").and_then(Value::as_bool).unwrap_or(false);
        Ok(Page { skills, page, has_more })
    }

    pub async fn search(&self, query: &str) -> Result<Vec<Listing>, DirectoryError> {
        let query = query.trim();
        if query.chars().count() < 2 {
            return Err(DirectoryError::ShortQuery);
        }
        let url = reqwest::Url::parse_with_params(
            &format!("{}/api/search", self.base),
            &[("q", query), ("limit", &SEARCH_LIMIT.to_string())],
        )
        .map_err(|err| DirectoryError::Unexpected(err.to_string()))?;
        match self.get(url.as_str()).await? {
            Some(value) => listings(&value),
            None => Ok(Vec::new()),
        }
    }

    pub async fn package(&self, id: &str) -> Result<Package, DirectoryError> {
        let [owner, repo, slug] = github_id(id)?;
        let value = self
            .get(&format!("{}/api/download/{owner}/{repo}/{slug}", self.base))
            .await?
            .ok_or_else(|| DirectoryError::Missing(id.trim().to_string()))?;
        let files = value
            .get("files")
            .and_then(Value::as_array)
            .ok_or_else(|| DirectoryError::Unexpected("a skill came with no file list".into()))?
            .iter()
            .filter_map(|file| {
                let path = file.get("path")?.as_str()?;
                let contents = file.get("contents")?.as_str()?;
                Some((path.to_string(), contents.to_string()))
            })
            .collect();
        Ok(Package::new(slug, files)?)
    }

    pub async fn audits(&self, id: &str) -> Result<Vec<Audit>, DirectoryError> {
        let [owner, repo, slug] = github_id(id)?;
        let url = format!("{}/api/v1/skills/audit/{owner}/{repo}/{slug}", self.base);
        let Some(value) = self.get(&url).await? else {
            return Ok(Vec::new());
        };
        let audits = value
            .get("audits")
            .and_then(Value::as_array)
            .ok_or_else(|| DirectoryError::Unexpected("an audit came with no findings".into()))?;
        Ok(audits
            .iter()
            .filter_map(|audit| {
                let provider = audit.get("provider")?.as_str()?.to_string();
                let verdict = match audit.get("status").and_then(Value::as_str) {
                    Some("pass") => Verdict::Pass,
                    Some("warn") => Verdict::Warn,
                    Some("fail") => Verdict::Fail,
                    _ => Verdict::Unknown,
                };
                let summary = audit.get("summary").and_then(Value::as_str).unwrap_or_default();
                let (summary, _) = crate::domain::cut_to(summary, 600);
                Some(Audit { provider, verdict, summary })
            })
            .collect())
    }

    pub async fn add(
        &self,
        skills: &Skills,
        scope: Scope,
        id: &str,
        hash: &str,
    ) -> Result<Skill, DirectoryError> {
        let package = self.package(id).await?;
        if package.hash != hash.trim() {
            return Err(DirectoryError::Changed);
        }
        let origin = page_url(id);
        let added = skills.install(scope, &package, Some(&origin))?;
        tracing::info!(skill = %added.name, %origin, files = added.files.len(), "added a skill from skills.sh");
        Ok(added)
    }

    pub async fn preview(&self, id: &str) -> Result<Preview, DirectoryError> {
        let (package, audits) = tokio::join!(self.package(id), self.audits(id));
        let package = package?;
        let audits = match audits {
            Ok(audits) => Some(audits),
            Err(err) => {
                tracing::warn!(%err, %id, "could not read a skill's audits");
                None
            }
        };
        Ok(Preview {
            id: id.trim().to_string(),
            name: package.name,
            description: package.description,
            body: package.body,
            files: package.files.into_iter().map(|(path, _)| path).collect(),
            hash: package.hash,
            url: page_url(id),
            audits,
        })
    }
}

pub fn page_url(id: &str) -> String {
    format!("{BASE}/{}", id.trim())
}

fn listings(value: &Value) -> Result<Vec<Listing>, DirectoryError> {
    let items = value
        .get("skills")
        .and_then(Value::as_array)
        .ok_or_else(|| DirectoryError::Unexpected("no list of skills in the answer".into()))?;
    Ok(items
        .iter()
        .filter_map(|item| {
            let source = item.get("source")?.as_str()?.trim().to_string();
            let slug = item.get("skillId")?.as_str()?.trim().to_string();
            if source.is_empty() || slug.is_empty() {
                return None;
            }
            let id = format!("{source}/{slug}");
            let name = item
                .get("name")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .unwrap_or(&slug)
                .to_string();
            let installs = item.get("installs").and_then(Value::as_u64).unwrap_or(0);
            let addable = github_id(&id).is_ok();
            let url = page_url(&id);
            Some(Listing { id, source, slug, name, installs, url, addable })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_id_names_a_repository_and_a_skill_or_nothing() {
        assert_eq!(github_id("anthropics/skills/pdf").unwrap(), ["anthropics", "skills", "pdf"]);
        assert!(matches!(github_id("open.feishu.cn/lark-doc"), Err(DirectoryError::NotAddable(_))));
        for bad in ["", "a/b", "a/b/c/d", "../b/c", "a/../c", "a/b/c?x=1", "a/b/c#x", "a/b/%2e%2e"]
        {
            assert!(matches!(github_id(bad), Err(DirectoryError::Missing(_))), "{bad}");
        }
    }

    #[test]
    fn a_listing_is_read_field_by_field_and_a_broken_entry_is_skipped() {
        let answer = serde_json::json!({ "skills": [
            { "source": "anthropics/skills", "skillId": "pdf", "name": "pdf", "installs": 201834 },
            { "source": "open.feishu.cn", "skillId": "lark-doc", "installs": 9 },
            { "source": "no/skill-id" },
            { "source": "a/b", "skillId": "c", "installs": "many" },
        ]});
        let found = listings(&answer).unwrap();
        let ids: Vec<_> = found.iter().map(|listing| listing.id.as_str()).collect();
        assert_eq!(ids, ["anthropics/skills/pdf", "open.feishu.cn/lark-doc", "a/b/c"]);
        assert!(found[0].addable && !found[1].addable);
        assert_eq!(found[1].name, "lark-doc", "a missing name falls back to the slug");
        assert_eq!(found[2].installs, 0);
        let changed = listings(&serde_json::json!({ "data": [] })).unwrap_err();
        assert!(matches!(changed, DirectoryError::Unexpected(_)), "{changed}");
    }

    #[test]
    fn a_board_is_asked_for_by_the_path_the_site_uses() {
        assert_eq!(Board::AllTime.segment(), "all-time");
        assert_eq!(serde_json::to_value(Board::AllTime).unwrap(), "allTime");
        assert_eq!(page_url(" anthropics/skills/pdf "), "https://skills.sh/anthropics/skills/pdf");
    }
}
