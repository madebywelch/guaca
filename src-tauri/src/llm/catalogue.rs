//! What OpenRouter is asked to do, and which models it hands that work to.
//!
//! Beside `openrouter.rs` because it is the same vendor, and separate from it
//! because it is not the same conversation: that file is how a turn is spent,
//! this one is read once while a dialog is open and never during a turn. No
//! agent, prompt, tool or guard reads anything here.
//!
//! ## Why the ranking is not the one the endpoint returns by default
//!
//! `?category=` orders by how many tokens OpenRouter routed to each model for
//! that kind of work, which sounds like the answer and is not. Bulk traffic
//! dominates it: the same cheap high-throughput model tops eleven of the twelve
//! use cases, so a picker built on that order suggests one model for every
//! agent and says something different each time about why. The category is
//! still what makes the list relevant — it is the set of models people actually
//! send this kind of work to — so it decides the pool, and `sort=` decides the
//! order inside it. Capability, from the index OpenRouter publishes on the same
//! response, is what makes twelve pools read as twelve answers.
//!
//! ## Why the use cases are checked here
//!
//! OpenRouter answers an unrecognised category with 200 and an empty list, so a
//! slug that has been renamed on their side is indistinguishable from a use case
//! nobody sends work to. Checking against the published set before spending a
//! request turns the first case into a sentence naming what was asked for, and
//! leaves the empty answer meaning only one thing.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

/// The catalogue is OpenRouter's, so this is not the operator's endpoint.
///
/// Suggestions are only offered when OpenRouter is what the agent's turns are
/// paid through, and the ranking is still OpenRouter's own even then. Pointing
/// this at a configured base URL would ask a local LM Studio to rank the world.
const DEFAULT_BASE: &str = "https://openrouter.ai/api/v1";

/// The use cases OpenRouter classifies traffic into, as it spells them.
///
/// Written out rather than discovered, because there is no endpoint that lists
/// them: they are an enum in the vendor's OpenAPI document. `src/lib/roles.ts`
/// holds the same twelve and `ipc.contract.test.ts` compares the two files, so a
/// use case added on one side fails the build rather than a click.
pub const CATEGORIES: [&str; 12] = [
    "programming",
    "roleplay",
    "marketing",
    "marketing/seo",
    "technology",
    "science",
    "translation",
    "legal",
    "finance",
    "health",
    "trivia",
    "academia",
];

/// Long enough that opening the same dialog twice costs one request, short
/// enough that "today" is true. The ranking moves when a vendor ships, which is
/// weeks, not minutes.
const TTL: Duration = Duration::from_secs(6 * 60 * 60);

/// How many of the twenty are worth carrying across IPC.
///
/// The dialog offers three and drops the model already in the box, so four is
/// the most it can draw. The other sixteen are a payload nothing renders.
const KEEP: usize = 4;

/// Long enough for a cold edge, short enough that a dialog does not sit there.
/// Nothing is blocked on this: the field works while it is in flight.
const HTTP_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, thiserror::Error)]
pub enum CatalogueError {
    /// Caught here rather than at OpenRouter, which answers this with an empty
    /// list. See the module comment.
    #[error("OpenRouter does not rank models for {asked}; ask for one of: {}", CATEGORIES.join(", "))]
    Unsupported { asked: String },
    #[error("could not reach OpenRouter for {category}: {detail}")]
    Transport { category: String, detail: String },
    #[error("OpenRouter returned HTTP {status} ranking models for {category}")]
    Status { category: String, status: u16 },
    #[error("OpenRouter sent something that is not a model list for {category}: {detail}")]
    Malformed { category: String, detail: String },
    /// A use case this build believes in, that the vendor now ranks nothing for.
    /// Distinct from the refusal above because there is nothing the caller can
    /// spell differently: the set above has gone stale and wants updating.
    #[error("OpenRouter ranks no models for {0} any more; the use case has been withdrawn")]
    Withdrawn(String),
}

/// One model, as much of it as a suggestion has to say.
///
/// Price is on it because the ranking is by capability alone, and the most
/// capable model for a use case is regularly the most expensive one in the pool.
/// A row that names a model and hides what it costs is a one-click way to make
/// every turn of an agent forty times dearer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RankedModel {
    /// The slug a turn is made with, which is what the model field holds.
    pub id: String,
    /// How the vendor writes it for a person, e.g. "Anthropic: Claude Opus 5".
    pub name: String,
    pub context_length: u32,
    /// Dollars per million prompt tokens. Converted here rather than in the
    /// webview so the number that crosses IPC is the number that is drawn, and
    /// `None` when the vendor quoted no price rather than a misleading zero.
    pub prompt_per_million: Option<f64>,
    /// Dollars per million completion tokens.
    pub completion_per_million: Option<f64>,
}

/// OpenRouter's ranked catalogue, with what has already been asked for.
///
/// Cheap to clone behind the `Arc` its owner holds. The mutex is held to read or
/// replace a cache entry and never across a request, so two dialogs opening at
/// once make two requests rather than one blocking the other.
#[derive(Debug)]
pub struct Catalogue {
    base: String,
    http: reqwest::Client,
    cached: Mutex<HashMap<String, Cached>>,
}

#[derive(Debug, Clone)]
struct Cached {
    at: Instant,
    models: Vec<RankedModel>,
}

impl Catalogue {
    /// Against OpenRouter, which is the only place this data exists.
    pub fn new() -> Self {
        Self::against(DEFAULT_BASE)
    }

    /// The same, against a named base. The seam for the test suite; deliberately
    /// not reachable from settings, for the reason on `DEFAULT_BASE`.
    pub fn against(base: impl Into<String>) -> Self {
        Self {
            base: base.into().trim_end_matches('/').to_string(),
            http: reqwest::Client::builder().timeout(HTTP_TIMEOUT).build().unwrap_or_default(),
            cached: Mutex::new(HashMap::new()),
        }
    }

    /// The models OpenRouter sees doing this kind of work, most capable first.
    pub async fn ranked(&self, category: &str) -> Result<Vec<RankedModel>, CatalogueError> {
        let category = category.trim().to_ascii_lowercase();
        if !CATEGORIES.contains(&category.as_str()) {
            return Err(CatalogueError::Unsupported { asked: category });
        }
        if let Some(fresh) = self.fresh(&category) {
            return Ok(fresh);
        }

        let response = self
            .http
            .get(format!("{}/models", self.base))
            // `query` rather than a formatted URL: one of the twelve has a
            // slash in it, and `marketing/seo` spliced into a path is a request
            // for a page that does not exist.
            .query(&[("category", category.as_str()), ("sort", "intelligence-high-to-low")])
            .send()
            .await
            .map_err(|err| CatalogueError::Transport {
                category: category.clone(),
                detail: err.to_string(),
            })?;

        let status = response.status();
        if !status.is_success() {
            return Err(CatalogueError::Status { category, status: status.as_u16() });
        }

        let body: Listing = response.json().await.map_err(|err| CatalogueError::Malformed {
            category: category.clone(),
            detail: err.to_string(),
        })?;

        let models: Vec<RankedModel> =
            body.data.into_iter().filter_map(RankedModel::from_wire).take(KEEP).collect();
        if models.is_empty() {
            return Err(CatalogueError::Withdrawn(category));
        }

        self.cached.lock().insert(category, Cached { at: Instant::now(), models: models.clone() });
        Ok(models)
    }

    fn fresh(&self, category: &str) -> Option<Vec<RankedModel>> {
        let held = self.cached.lock();
        let entry = held.get(category)?;
        (entry.at.elapsed() < TTL).then(|| entry.models.clone())
    }
}

impl Default for Catalogue {
    fn default() -> Self {
        Self::new()
    }
}

// ---- the wire ------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct Listing {
    data: Vec<WireModel>,
}

#[derive(Debug, Deserialize)]
struct WireModel {
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    context_length: u32,
    #[serde(default)]
    pricing: Option<WirePricing>,
}

/// Only the two prices a turn is actually billed on. The vendor sends a dozen
/// more — image, audio, web search, four kinds of cache — and a suggestion row
/// that quoted any of them would be quoting a price this app does not pay.
#[derive(Debug, Deserialize)]
struct WirePricing {
    prompt: Option<String>,
    completion: Option<String>,
}

impl RankedModel {
    /// `None` for a row with no id, which is nothing this can offer.
    fn from_wire(wire: WireModel) -> Option<Self> {
        if wire.id.trim().is_empty() {
            return None;
        }
        let (prompt, completion) = match wire.pricing {
            Some(pricing) => (per_million(pricing.prompt), per_million(pricing.completion)),
            None => (None, None),
        };
        Some(Self {
            name: if wire.name.trim().is_empty() { wire.id.clone() } else { wire.name },
            id: wire.id,
            context_length: wire.context_length,
            prompt_per_million: prompt,
            completion_per_million: completion,
        })
    }
}

/// Dollars per token, as the vendor quotes it, into dollars per million.
///
/// A string on the wire because the numbers run to eleven decimal places and
/// the vendor would rather not argue with a JSON parser about them. Anything
/// unparseable is no price rather than zero: a free model quotes "0", and the
/// two must not read the same.
fn per_million(quoted: Option<String>) -> Option<f64> {
    let parsed: f64 = quoted?.trim().parse().ok()?;
    parsed.is_finite().then_some(parsed * 1_000_000.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use axum::extract::Query;
    use axum::response::IntoResponse;
    use axum::routing::get;
    use axum::Router;

    /// Spins a stub catalogue and returns its base URL, the queries it saw, and
    /// how many requests it answered.
    async fn stub(
        answer: impl Fn() -> axum::response::Response + Clone + Send + Sync + 'static,
    ) -> (String, Arc<Mutex<Vec<HashMap<String, String>>>>, Arc<AtomicUsize>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let count = Arc::new(AtomicUsize::new(0));
        let recorder = seen.clone();
        let counter = count.clone();

        let app = Router::new().route(
            "/v1/models",
            get(move |Query(query): Query<HashMap<String, String>>| {
                let answer = answer.clone();
                let recorder = recorder.clone();
                let counter = counter.clone();
                async move {
                    recorder.lock().push(query);
                    counter.fetch_add(1, Ordering::SeqCst);
                    answer()
                }
            }),
        );

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        (format!("http://{addr}/v1"), seen, count)
    }

    fn listing(models: serde_json::Value) -> axum::response::Response {
        axum::Json(serde_json::json!({ "data": models })).into_response()
    }

    fn two() -> serde_json::Value {
        serde_json::json!([
            {
                "id": "openai/gpt-5.6-sol",
                "name": "OpenAI: GPT-5.6 Sol",
                "context_length": 400000,
                "pricing": { "prompt": "0.000002", "completion": "0.000012" },
            },
            {
                "id": "z-ai/glm-5.2",
                "name": "Z.AI: GLM 5.2",
                "context_length": 200000,
                "pricing": { "prompt": "0", "completion": "0" },
            },
        ])
    }

    /// The refusal that costs nothing. OpenRouter answers a use case it does not
    /// know with 200 and an empty list, so asking it would report the typo as an
    /// absence of models and the operator would go looking for the wrong thing.
    #[tokio::test]
    async fn a_use_case_openrouter_does_not_rank_is_refused_without_asking() {
        let (base, _seen, count) = stub(|| listing(two())).await;

        let refused = Catalogue::against(base).ranked("sales").await.unwrap_err();

        assert!(matches!(refused, CatalogueError::Unsupported { ref asked } if asked == "sales"));
        // The whole point: no request was spent finding that out.
        assert_eq!(count.load(Ordering::SeqCst), 0);
        // And the refusal says what may be asked for instead.
        assert!(refused.to_string().contains("marketing"));
    }

    #[tokio::test]
    async fn a_use_case_is_asked_for_by_capability_inside_its_own_pool() {
        let (base, seen, _count) = stub(|| listing(two())).await;

        Catalogue::against(base).ranked("legal").await.unwrap();

        let query = seen.lock()[0].clone();
        assert_eq!(query["category"], "legal");
        // Popularity would hand back the same three models for all twelve; see
        // the module comment.
        assert_eq!(query["sort"], "intelligence-high-to-low");
    }

    /// The one use case with a slash in it. Spliced into a path it asks for a
    /// page rather than a list, and the answer is a 404 the operator reads as
    /// OpenRouter being down.
    #[tokio::test]
    async fn the_use_case_with_a_slash_in_it_survives_the_query_string() {
        let (base, seen, _count) = stub(|| listing(two())).await;

        Catalogue::against(base).ranked("marketing/seo").await.unwrap();

        assert_eq!(seen.lock()[0]["category"], "marketing/seo");
    }

    #[tokio::test]
    async fn a_price_crosses_as_dollars_per_million_and_a_free_model_as_zero() {
        let (base, _seen, _count) = stub(|| listing(two())).await;

        let ranked = Catalogue::against(base).ranked("legal").await.unwrap();

        assert_eq!(ranked[0].id, "openai/gpt-5.6-sol");
        assert_eq!(ranked[0].name, "OpenAI: GPT-5.6 Sol");
        assert_eq!(ranked[0].prompt_per_million, Some(2.0));
        assert_eq!(ranked[0].completion_per_million, Some(12.0));
        // A free model quotes a real zero, which is not the same as no quote.
        assert_eq!(ranked[1].prompt_per_million, Some(0.0));
    }

    /// A model quoted no price is offered with no price beside it. Rendering the
    /// absence as $0.00 advertises a free model that is not one.
    #[tokio::test]
    async fn a_model_with_no_quoted_price_carries_no_price() {
        let unpriced = serde_json::json!([
            { "id": "local/whatever", "name": "Local", "context_length": 8192 },
            { "id": "odd/one", "name": "Odd", "pricing": { "prompt": "free", "completion": null } },
        ]);
        let (base, _seen, _count) = stub(move || listing(unpriced.clone())).await;

        let ranked = Catalogue::against(base).ranked("legal").await.unwrap();

        assert_eq!(ranked[0].prompt_per_million, None);
        assert_eq!(ranked[1].prompt_per_million, None);
        assert_eq!(ranked[1].completion_per_million, None);
    }

    /// A row with no slug is a row nothing can be swapped to, and a name with no
    /// slug behind it is worse than no row: it is a button that cannot work.
    #[tokio::test]
    async fn a_row_with_no_slug_is_dropped_and_a_row_with_no_name_keeps_its_slug() {
        let ragged = serde_json::json!([
            { "id": "  ", "name": "Nameless" },
            { "id": "vendor/model" },
        ]);
        let (base, _seen, _count) = stub(move || listing(ragged.clone())).await;

        let ranked = Catalogue::against(base).ranked("legal").await.unwrap();

        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0].id, "vendor/model");
        assert_eq!(ranked[0].name, "vendor/model");
    }

    /// Four is what the dialog can draw. The rest is a payload nothing renders,
    /// paid for on every keystroke that changes which use case an agent reads as.
    #[tokio::test]
    async fn only_what_the_dialog_can_draw_crosses_ipc() {
        let twenty = serde_json::json!((0..20)
            .map(|n| serde_json::json!({ "id": format!("v/m{n}"), "name": format!("M{n}") }))
            .collect::<Vec<_>>());
        let (base, _seen, _count) = stub(move || listing(twenty.clone())).await;

        let ranked = Catalogue::against(base).ranked("legal").await.unwrap();

        assert_eq!(ranked.len(), KEEP);
        assert_eq!(ranked[0].id, "v/m0");
    }

    /// A network round trip in front of a text field, on a list that changes
    /// when a vendor ships rather than when an operator types.
    #[tokio::test]
    async fn the_same_use_case_is_asked_for_once() {
        let (base, _seen, count) = stub(|| listing(two())).await;
        let catalogue = Catalogue::against(base);

        let first = catalogue.ranked("legal").await.unwrap();
        let again = catalogue.ranked("  LEGAL  ").await.unwrap();

        assert_eq!(first, again);
        assert_eq!(count.load(Ordering::SeqCst), 1);
        // Two use cases are two lists, so the second one is still a request.
        catalogue.ranked("finance").await.unwrap();
        assert_eq!(count.load(Ordering::SeqCst), 2);
    }

    /// A use case this build believes in and the vendor no longer ranks. Its own
    /// error because nothing the caller spells differently would fix it: the set
    /// in `CATEGORIES` has gone stale.
    #[tokio::test]
    async fn a_use_case_that_has_been_withdrawn_says_so() {
        let (base, _seen, count) = stub(|| listing(serde_json::json!([]))).await;
        let catalogue = Catalogue::against(base);

        let refused = catalogue.ranked("legal").await.unwrap_err();

        assert!(matches!(refused, CatalogueError::Withdrawn(ref what) if what == "legal"));
        // Nothing empty is cached, so a vendor putting the use case back is one
        // dialog away rather than a restart.
        assert!(catalogue.fresh("legal").is_none());
        catalogue.ranked("legal").await.unwrap_err();
        assert_eq!(count.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn a_refusal_from_openrouter_names_the_status_and_is_not_cached() {
        let (base, _seen, count) =
            stub(|| (axum::http::StatusCode::TOO_MANY_REQUESTS, "slow down").into_response()).await;
        let catalogue = Catalogue::against(base);

        let refused = catalogue.ranked("legal").await.unwrap_err();

        assert!(matches!(refused, CatalogueError::Status { status: 429, .. }));
        assert!(refused.to_string().contains("legal"));
        catalogue.ranked("legal").await.unwrap_err();
        assert_eq!(count.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn an_answer_that_is_not_a_model_list_says_which_use_case_it_was_for() {
        let (base, _seen, _count) =
            stub(|| axum::Json(serde_json::json!({ "models": [] })).into_response()).await;

        let refused = Catalogue::against(base).ranked("science").await.unwrap_err();

        assert!(matches!(refused, CatalogueError::Malformed { .. }));
        assert!(refused.to_string().contains("science"));
    }

    /// The failure no stub can see.
    ///
    /// Every test above is this build agreeing with itself about a protocol, and
    /// the twelve use cases are the half of that agreement OpenRouter can change
    /// without telling anyone: a category renamed there answers 200 with an empty
    /// list, so the dialog would draw nothing for exactly the agents it was built
    /// for, silently, and no offline suite would notice. Same shape as the live
    /// halves of `tests/plugins.rs` and `tests/account.rs`.
    ///
    /// Reaches the internet, authorises nothing and spends nothing.
    ///
    ///   cargo test --manifest-path src-tauri/Cargo.toml \
    ///     llm::catalogue::tests::every_use_case -- --ignored --nocapture
    #[tokio::test]
    #[ignore = "reaches OpenRouter"]
    async fn every_use_case_this_build_believes_in_is_one_openrouter_still_ranks() {
        let catalogue = Catalogue::new();
        let mut missing = Vec::new();

        for use_case in CATEGORIES {
            match catalogue.ranked(use_case).await {
                Ok(models) => {
                    println!("{use_case:<14} {}", models[0].id);
                    // Capability ordering is the whole reason this is not the
                    // endpoint's default order, and a `sort` value the vendor
                    // stops accepting is a 400 rather than a quiet reordering.
                    assert!(!models[0].id.is_empty(), "{use_case} ranked a model with no slug");
                }
                Err(err) => missing.push(format!("{use_case}: {err}")),
            }
        }

        assert!(missing.is_empty(), "OpenRouter no longer ranks: {}", missing.join("; "));
    }

    #[tokio::test]
    async fn an_unreachable_catalogue_names_openrouter_rather_than_a_port() {
        // Nothing is listening here, and nothing ever will be.
        let refused =
            Catalogue::against("http://127.0.0.1:1/v1").ranked("legal").await.unwrap_err();

        assert!(matches!(refused, CatalogueError::Transport { .. }));
        assert!(refused.to_string().contains("OpenRouter"));
    }
}
