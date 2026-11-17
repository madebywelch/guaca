use std::collections::HashMap;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

const DEFAULT_BASE: &str = "https://openrouter.ai/api/v1";

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

const TTL: Duration = Duration::from_secs(6 * 60 * 60);

const KEEP: usize = 4;

const HTTP_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, thiserror::Error)]
pub enum CatalogError {
    #[error("OpenRouter does not rank models for {asked}; ask for one of: {}", CATEGORIES.join(", "))]
    Unsupported { asked: String },
    #[error("could not reach OpenRouter for {category}: {detail}")]
    Transport { category: String, detail: String },
    #[error("OpenRouter returned HTTP {status} ranking models for {category}")]
    Status { category: String, status: u16 },
    #[error("OpenRouter sent something that is not a model list for {category}: {detail}")]
    Malformed { category: String, detail: String },
    #[error("OpenRouter ranks no models for {0} any more; the use case has been withdrawn")]
    Withdrawn(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RankedModel {
    pub id: String,
    pub name: String,
    pub context_length: u32,
    pub prompt_per_million: Option<f64>,
    pub completion_per_million: Option<f64>,
}

#[derive(Debug)]
pub struct Catalog {
    base: String,
    http: reqwest::Client,
    cached: Mutex<HashMap<String, Cached>>,
}

#[derive(Debug, Clone)]
struct Cached {
    at: Instant,
    models: Vec<RankedModel>,
}

impl Catalog {
    pub fn new() -> Self {
        Self::against(DEFAULT_BASE)
    }

    pub fn against(base: impl Into<String>) -> Self {
        Self {
            base: base.into().trim_end_matches('/').to_string(),
            http: reqwest::Client::builder().timeout(HTTP_TIMEOUT).build().unwrap_or_default(),
            cached: Mutex::new(HashMap::new()),
        }
    }

    pub(super) async fn ranked(&self, category: &str) -> Result<Vec<RankedModel>, CatalogError> {
        let category = category.trim().to_ascii_lowercase();
        if !CATEGORIES.contains(&category.as_str()) {
            return Err(CatalogError::Unsupported { asked: category });
        }
        if let Some(fresh) = self.fresh(&category) {
            return Ok(fresh);
        }

        let response = self
            .http
            .get(format!("{}/models", self.base))
            .query(&[("category", category.as_str()), ("sort", "intelligence-high-to-low")])
            .send()
            .await
            .map_err(|err| CatalogError::Transport {
                category: category.clone(),
                detail: err.to_string(),
            })?;

        let status = response.status();
        if !status.is_success() {
            return Err(CatalogError::Status { category, status: status.as_u16() });
        }

        let body: Listing = response.json().await.map_err(|err| CatalogError::Malformed {
            category: category.clone(),
            detail: err.to_string(),
        })?;

        let models: Vec<RankedModel> =
            body.data.into_iter().filter_map(RankedModel::from_wire).take(KEEP).collect();
        if models.is_empty() {
            return Err(CatalogError::Withdrawn(category));
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

impl Default for Catalog {
    fn default() -> Self {
        Self::new()
    }
}

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

#[derive(Debug, Deserialize)]
struct WirePricing {
    prompt: Option<String>,
    completion: Option<String>,
}

impl RankedModel {
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

    #[tokio::test]
    async fn a_use_case_openrouter_does_not_rank_is_refused_without_asking() {
        let (base, _seen, count) = stub(|| listing(two())).await;

        let refused = Catalog::against(base).ranked("sales").await.unwrap_err();

        assert!(matches!(refused, CatalogError::Unsupported { ref asked } if asked == "sales"));
        assert_eq!(count.load(Ordering::SeqCst), 0);
        assert!(refused.to_string().contains("marketing"));
    }

    #[tokio::test]
    async fn a_use_case_is_asked_for_by_capability_inside_its_own_pool() {
        let (base, seen, _count) = stub(|| listing(two())).await;

        Catalog::against(base).ranked("legal").await.unwrap();

        let query = seen.lock()[0].clone();
        assert_eq!(query["category"], "legal");
        assert_eq!(query["sort"], "intelligence-high-to-low");
    }

    #[tokio::test]
    async fn the_use_case_with_a_slash_in_it_survives_the_query_string() {
        let (base, seen, _count) = stub(|| listing(two())).await;

        Catalog::against(base).ranked("marketing/seo").await.unwrap();

        assert_eq!(seen.lock()[0]["category"], "marketing/seo");
    }

    #[tokio::test]
    async fn a_price_crosses_as_dollars_per_million_and_a_free_model_as_zero() {
        let (base, _seen, _count) = stub(|| listing(two())).await;

        let ranked = Catalog::against(base).ranked("legal").await.unwrap();

        assert_eq!(ranked[0].id, "openai/gpt-5.6-sol");
        assert_eq!(ranked[0].name, "OpenAI: GPT-5.6 Sol");
        assert_eq!(ranked[0].prompt_per_million, Some(2.0));
        assert_eq!(ranked[0].completion_per_million, Some(12.0));
        assert_eq!(ranked[1].prompt_per_million, Some(0.0));
    }

    #[tokio::test]
    async fn a_model_with_no_quoted_price_carries_no_price() {
        let unpriced = serde_json::json!([
            { "id": "local/whatever", "name": "Local", "context_length": 8192 },
            { "id": "odd/one", "name": "Odd", "pricing": { "prompt": "free", "completion": null } },
        ]);
        let (base, _seen, _count) = stub(move || listing(unpriced.clone())).await;

        let ranked = Catalog::against(base).ranked("legal").await.unwrap();

        assert_eq!(ranked[0].prompt_per_million, None);
        assert_eq!(ranked[1].prompt_per_million, None);
        assert_eq!(ranked[1].completion_per_million, None);
    }

    #[tokio::test]
    async fn a_row_with_no_slug_is_dropped_and_a_row_with_no_name_keeps_its_slug() {
        let ragged = serde_json::json!([
            { "id": "  ", "name": "Nameless" },
            { "id": "vendor/model" },
        ]);
        let (base, _seen, _count) = stub(move || listing(ragged.clone())).await;

        let ranked = Catalog::against(base).ranked("legal").await.unwrap();

        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0].id, "vendor/model");
        assert_eq!(ranked[0].name, "vendor/model");
    }

    #[tokio::test]
    async fn only_what_the_dialog_can_draw_crosses_ipc() {
        let twenty = serde_json::json!((0..20)
            .map(|n| serde_json::json!({ "id": format!("v/m{n}"), "name": format!("M{n}") }))
            .collect::<Vec<_>>());
        let (base, _seen, _count) = stub(move || listing(twenty.clone())).await;

        let ranked = Catalog::against(base).ranked("legal").await.unwrap();

        assert_eq!(ranked.len(), KEEP);
        assert_eq!(ranked[0].id, "v/m0");
    }

    #[tokio::test]
    async fn the_same_use_case_is_asked_for_once() {
        let (base, _seen, count) = stub(|| listing(two())).await;
        let catalog = Catalog::against(base);

        let first = catalog.ranked("legal").await.unwrap();
        let again = catalog.ranked("  LEGAL  ").await.unwrap();

        assert_eq!(first, again);
        assert_eq!(count.load(Ordering::SeqCst), 1);
        catalog.ranked("finance").await.unwrap();
        assert_eq!(count.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn a_use_case_that_has_been_withdrawn_says_so() {
        let (base, _seen, count) = stub(|| listing(serde_json::json!([]))).await;
        let catalog = Catalog::against(base);

        let refused = catalog.ranked("legal").await.unwrap_err();

        assert!(matches!(refused, CatalogError::Withdrawn(ref what) if what == "legal"));
        assert!(catalog.fresh("legal").is_none());
        catalog.ranked("legal").await.unwrap_err();
        assert_eq!(count.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn a_refusal_from_openrouter_names_the_status_and_is_not_cached() {
        let (base, _seen, count) =
            stub(|| (axum::http::StatusCode::TOO_MANY_REQUESTS, "slow down").into_response()).await;
        let catalog = Catalog::against(base);

        let refused = catalog.ranked("legal").await.unwrap_err();

        assert!(matches!(refused, CatalogError::Status { status: 429, .. }));
        assert!(refused.to_string().contains("legal"));
        catalog.ranked("legal").await.unwrap_err();
        assert_eq!(count.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn an_answer_that_is_not_a_model_list_says_which_use_case_it_was_for() {
        let (base, _seen, _count) =
            stub(|| axum::Json(serde_json::json!({ "models": [] })).into_response()).await;

        let refused = Catalog::against(base).ranked("science").await.unwrap_err();

        assert!(matches!(refused, CatalogError::Malformed { .. }));
        assert!(refused.to_string().contains("science"));
    }

    #[tokio::test]
    #[ignore = "reaches OpenRouter"]
    async fn every_use_case_this_build_believes_in_is_one_openrouter_still_ranks() {
        let catalog = Catalog::new();
        let mut missing = Vec::new();

        for use_case in CATEGORIES {
            match catalog.ranked(use_case).await {
                Ok(models) => {
                    println!("{use_case:<14} {}", models[0].id);
                    assert!(!models[0].id.is_empty(), "{use_case} ranked a model with no slug");
                }
                Err(err) => missing.push(format!("{use_case}: {err}")),
            }
        }

        assert!(missing.is_empty(), "OpenRouter no longer ranks: {}", missing.join("; "));
    }

    #[tokio::test]
    async fn an_unreachable_catalog_names_openrouter_rather_than_a_port() {
        let refused = Catalog::against("http://127.0.0.1:1/v1").ranked("legal").await.unwrap_err();

        assert!(matches!(refused, CatalogError::Transport { .. }));
        assert!(refused.to_string().contains("OpenRouter"));
    }
}
