use std::collections::HashMap;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde::Deserialize;

use crate::config::{InferenceConfig, Provider};

const TTL: Duration = Duration::from_secs(6 * 60 * 60);

const COOLDOWN: Duration = Duration::from_secs(5 * 60);

const HTTP_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Modalities {
    pub image: bool,
}

impl Modalities {
    pub fn seeing() -> Self {
        Modalities { image: true }
    }

    pub fn text_only() -> Self {
        Modalities { image: false }
    }
}

#[derive(Debug)]
pub struct Registry {
    http: reqwest::Client,
    known: Mutex<HashMap<String, Known>>,
}

#[derive(Debug)]
struct Known {
    at: Instant,
    ttl: Duration,
    published: HashMap<String, Modalities>,
}

impl Registry {
    pub fn new() -> Self {
        Registry {
            http: reqwest::Client::builder().timeout(HTTP_TIMEOUT).build().unwrap_or_default(),
            known: Mutex::new(HashMap::new()),
        }
    }

    pub async fn of(&self, cfg: &InferenceConfig, model: &str) -> Modalities {
        match cfg.provider {
            Provider::Chatgpt | Provider::Claude => Modalities::seeing(),
            Provider::Compatible => {
                let answer =
                    self.published(cfg, model.trim()).await.unwrap_or_else(Modalities::seeing);
                if !answer.image {
                    tracing::debug!(
                        model,
                        endpoint = %cfg.base_url,
                        "published as text only: no picture, no screen"
                    );
                }
                answer
            }
        }
    }

    async fn published(&self, cfg: &InferenceConfig, model: &str) -> Option<Modalities> {
        let base = cfg.base_url.trim().trim_end_matches('/').to_string();
        if base.is_empty() {
            return None;
        }
        if let Some(remembered) = self.remembered(&base, model) {
            return remembered;
        }

        let (published, ttl) = self.ask(cfg, &base).await;
        let answer = published.get(model).copied();
        self.known.lock().insert(base, Known { at: Instant::now(), ttl, published });
        answer
    }

    fn remembered(&self, base: &str, model: &str) -> Option<Option<Modalities>> {
        let held = self.known.lock();
        let entry = held.get(base)?;
        (entry.at.elapsed() < entry.ttl).then(|| entry.published.get(model).copied())
    }

    async fn ask(
        &self,
        cfg: &InferenceConfig,
        base: &str,
    ) -> (HashMap<String, Modalities>, Duration) {
        let url = format!("{base}/models");
        let mut request = self.http.get(&url);
        let key = cfg.api_key.expose().trim();
        if !key.is_empty() {
            request = request.bearer_auth(key);
        }

        let listing = match request.send().await {
            Ok(response) if response.status().is_success() => response.json::<Listing>().await,
            Ok(response) => {
                tracing::debug!(%url, status = %response.status(), "endpoint published no model list");
                return (HashMap::new(), COOLDOWN);
            }
            Err(err) => {
                tracing::debug!(%url, %err, "could not read the endpoint's model list");
                return (HashMap::new(), COOLDOWN);
            }
        };

        match listing {
            Ok(listing) => {
                let published = listing.into_published();
                tracing::debug!(%url, models = published.len(), "read what this endpoint's models take");
                (published, TTL)
            }
            Err(err) => {
                tracing::debug!(%url, %err, "the endpoint's model list says nothing about modalities");
                (HashMap::new(), TTL)
            }
        }
    }
}

impl Default for Registry {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Deserialize)]
struct Listing {
    #[serde(default)]
    data: Vec<WireModel>,
}

#[derive(Debug, Deserialize)]
struct WireModel {
    #[serde(default)]
    id: String,
    #[serde(default)]
    architecture: Option<WireArchitecture>,
}

#[derive(Debug, Deserialize)]
struct WireArchitecture {
    #[serde(default)]
    input_modalities: Vec<String>,
}

impl Listing {
    fn into_published(self) -> HashMap<String, Modalities> {
        self.data
            .into_iter()
            .filter_map(|model| {
                let id = model.id.trim();
                let takes = model.architecture?.input_modalities;
                if id.is_empty() || takes.is_empty() {
                    return None;
                }
                let image = takes.iter().any(|kind| kind.eq_ignore_ascii_case("image"));
                Some((id.to_string(), Modalities { image }))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use axum::response::IntoResponse;
    use axum::routing::get;
    use axum::Router;

    async fn stub(
        answer: impl Fn() -> axum::response::Response + Clone + Send + Sync + 'static,
    ) -> (String, Arc<AtomicUsize>) {
        let count = Arc::new(AtomicUsize::new(0));
        let counter = count.clone();

        let app = Router::new().route(
            "/v1/models",
            get(move || {
                let answer = answer.clone();
                let counter = counter.clone();
                async move {
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

        (format!("http://{addr}/v1"), count)
    }

    fn against(base: &str) -> InferenceConfig {
        InferenceConfig {
            provider: Provider::Compatible,
            base_url: base.to_string(),
            api_key: "sk-test".into(),
            default_model: "vendor/seeing".into(),
            ..Default::default()
        }
    }

    fn listing() -> axum::response::Response {
        axum::Json(serde_json::json!({
            "data": [
                {
                    "id": "vendor/seeing",
                    "architecture": {
                        "input_modalities": ["text", "image"],
                        "output_modalities": ["text"],
                    },
                },
                {
                    "id": "vendor/reading",
                    "architecture": {
                        "input_modalities": ["text"],
                        "output_modalities": ["text"],
                    },
                },
            ]
        }))
        .into_response()
    }

    #[tokio::test]
    async fn a_model_the_endpoint_says_takes_pictures_is_sent_them() {
        let (base, _count) = stub(listing).await;

        let seen = Registry::new().of(&against(&base), "vendor/seeing").await;

        assert_eq!(seen, Modalities::seeing());
    }

    #[tokio::test]
    async fn a_model_published_as_text_only_is_not_sent_pictures() {
        let (base, _count) = stub(listing).await;

        let seen = Registry::new().of(&against(&base), "vendor/reading").await;

        assert_eq!(seen, Modalities::text_only());
    }

    #[tokio::test]
    async fn an_endpoint_that_publishes_no_modalities_changes_nothing() {
        let (base, _count) = stub(|| {
            axum::Json(serde_json::json!({
                "data": [
                    { "id": "vendor/reading", "object": "model", "owned_by": "somebody" },
                    { "id": "vendor/empty", "architecture": { "input_modalities": [] } },
                ]
            }))
            .into_response()
        })
        .await;
        let registry = Registry::new();

        for model in ["vendor/reading", "vendor/empty"] {
            assert_eq!(registry.of(&against(&base), model).await, Modalities::seeing(), "{model}");
        }
    }

    #[tokio::test]
    async fn a_model_that_is_not_on_the_list_is_left_as_it_was() {
        let (base, _count) = stub(listing).await;

        let seen = Registry::new().of(&against(&base), "vendor/nobody-has-heard-of").await;

        assert_eq!(seen, Modalities::seeing());
    }

    #[tokio::test]
    async fn an_endpoint_with_no_such_route_is_asked_once_and_left_alone() {
        let (base, count) = stub(|| axum::http::StatusCode::NOT_FOUND.into_response()).await;
        let registry = Registry::new();

        assert_eq!(registry.of(&against(&base), "vendor/reading").await, Modalities::seeing());
        assert_eq!(registry.of(&against(&base), "vendor/reading").await, Modalities::seeing());

        assert_eq!(count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn an_unreachable_endpoint_is_no_answer_rather_than_a_failure() {
        let seen = Registry::new().of(&against("http://127.0.0.1:1/v1"), "vendor/reading").await;

        assert_eq!(seen, Modalities::seeing());
    }

    #[tokio::test]
    async fn one_endpoint_is_read_once_however_many_models_are_asked_about() {
        let (base, count) = stub(listing).await;
        let registry = Registry::new();

        assert_eq!(registry.of(&against(&base), "vendor/seeing").await, Modalities::seeing());
        assert_eq!(registry.of(&against(&base), "vendor/reading").await, Modalities::text_only());
        assert_eq!(registry.of(&against(&base), "vendor/seeing").await, Modalities::seeing());

        assert_eq!(count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_subscription_is_answered_without_asking_anybody() {
        let (base, count) = stub(listing).await;
        let registry = Registry::new();

        for provider in [Provider::Chatgpt, Provider::Claude] {
            let cfg = InferenceConfig { provider, ..against(&base) };
            assert_eq!(registry.of(&cfg, "vendor/reading").await, Modalities::seeing());
        }

        assert_eq!(count.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    #[ignore = "reaches OpenRouter"]
    async fn openrouter_still_publishes_what_each_of_its_models_takes() {
        let cfg = InferenceConfig::default();
        let (published, ttl) = Registry::new().ask(&cfg, cfg.base_url.trim_end_matches('/')).await;

        assert!(!published.is_empty(), "OpenRouter published no modalities for any model");
        assert_eq!(ttl, TTL, "an endpoint that answered is trusted for the full term");
        assert!(published.values().any(|takes| takes.image), "nothing takes pictures");
        assert!(published.values().any(|takes| !takes.image), "nothing is text only");

        println!(
            "{} models published, {} of them take pictures",
            published.len(),
            published.values().filter(|takes| takes.image).count()
        );
        assert_eq!(
            published.get(crate::config::DEFAULT_MODEL),
            Some(&Modalities::seeing()),
            "the model every install starts on is not published as one that takes pictures"
        );
    }

    #[tokio::test]
    async fn a_model_name_is_matched_however_it_was_typed_into_the_box() {
        let (base, _count) = stub(listing).await;

        let seen = Registry::new().of(&against(&base), "  vendor/reading  ").await;

        assert_eq!(seen, Modalities::text_only());
    }
}
