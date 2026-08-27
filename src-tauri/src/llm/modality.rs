//! What the model behind a turn can be sent, and what its agent is told it is.
//!
//! The model is a text box. An operator types one slug this week and another
//! next week, and the two do not have to be able to do the same things: the
//! only thing every one of them can be relied on to take is text. Everything
//! else this app puts in front of a model (an attached photograph, a picture
//! of a machine's screen) is a capability of that particular model, and
//! Guaca assumed it of all of them for as long as there was only one worth
//! assuming. A screenshot sent to a text-only model is a refusal from
//! OpenRouter and a turn spent finding that out; `use_screen` offered to one is
//! a tool whose entire answer is a picture that reaches nobody.
//!
//! ## Only what Guaca can send is worth asking about
//!
//! What a model accepts and what this app can produce are two lists, and the
//! answer worth having is the intersection. Guaca produces exactly two things:
//! text, and pictures: an attachment, or a screen. It records no audio and no
//! video and has nothing to make either out of, so a model that would happily
//! take a video still gets none, and its agent is told so rather than offering
//! the operator something that cannot arrive.
//!
//! The other direction is a constant rather than a question. A reply is text.
//! A transcript, a peer's inbox, an eval and this app's own record are all
//! text, so a model that can draw a picture has nowhere to put one, and the way
//! an agent shows something it drew is already a chart spec or a page it wrote.
//! Nothing here is asked about output, and the prompt says the same thing in
//! words: what you produce is text.
//!
//! So there is one question, and this module answers only it: does a picture
//! reach this model.
//!
//! ## The answer comes from the endpoint, because it is the only place it is
//!
//! Not from a table of model names in this repo, which would be a build that
//! goes stale the week after it ships and a lie about every model released
//! since. Not from a checkbox in settings either: an operator swapping a model
//! is changing one field, and a second field beside it that has to be changed
//! in step is one that will not be.
//!
//! OpenRouter publishes `architecture.input_modalities` for every model it
//! routes to, on the same `/models` any OpenAI-compatible endpoint answers, so
//! the question is asked of whatever endpoint the turn is being paid through.
//! Read once and kept, for the reason a plugin's tool list is: it changes when
//! a vendor ships, not when an agent thinks, and a round trip in front of every
//! model call to re-learn the same answer is one every agent in the crew pays.
//!
//! ## An endpoint that says nothing leaves everything as it was
//!
//! A local LM Studio answers `/models` with ids and no architecture, and a
//! model that has never been near OpenRouter is not in its list at all. Both
//! are answered here the same way: a picture is sent, exactly as it was before
//! any of this existed.
//!
//! That asymmetry is deliberate and it is the whole safety of the change. A
//! wrong "it can see" is what happens today, on every endpoint, and costs a
//! turn that fails with an error naming the model. A wrong "it cannot see"
//! takes `use_screen` off an agent that was using it and quietly stops
//! delivering attachments, with nothing on screen saying why. Only an endpoint
//! that has published a modality list without `image` in it is taken to have
//! said no; nothing else is taken to have said anything.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde::Deserialize;

use crate::config::{InferenceConfig, Provider};

/// Long enough that a crew working all afternoon asks once, short enough that a
/// model added this morning is not invisible until tomorrow. What is being
/// cached moves when a vendor ships.
const TTL: Duration = Duration::from_secs(6 * 60 * 60);

/// How long an endpoint that could not be asked is left alone.
///
/// Shorter than the TTL because nothing was learned, and not zero because the
/// commonest reason to be here is an endpoint that is down, which is about to
/// fail the model call too: paying for the same refused request in front of
/// every turn adds a round trip to an outage.
const COOLDOWN: Duration = Duration::from_secs(5 * 60);

/// Long enough for a full catalog over a slow link, short enough that a hung
/// endpoint does not hold a turn open. Nothing is blocked on the answer: a
/// timeout is an endpoint that said nothing, which is the ordinary case.
const HTTP_TIMEOUT: Duration = Duration::from_secs(10);

/// What Guaca will put in front of the model on this turn.
///
/// One field, because one of the two things this app can send is in question.
/// Text is not a field: a model that cannot read text is not a model this app
/// can use at all, and there is nothing to decide.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Modalities {
    /// Whether a picture reaches the model as a picture.
    ///
    /// False makes three things true at once, and they have to stay in step: a
    /// picture is not sent, `use_screen` is not offered, and the agent's prompt
    /// says it cannot see. Two out of three is an agent told it is blind that
    /// is still handed a screenshot, or one taking screenshots it will never be
    /// shown.
    pub image: bool,
}

impl Modalities {
    /// A model that takes pictures, which is most of them and is what an
    /// endpoint that publishes nothing is taken to mean.
    pub fn seeing() -> Self {
        Modalities { image: true }
    }

    /// A model the endpoint has said takes text and nothing else.
    pub fn text_only() -> Self {
        Modalities { image: false }
    }
}

/// What each endpoint publishes about its own models, kept between turns.
///
/// Cheap to clone behind the `Arc` its owner holds. The mutex is taken to read
/// or replace one endpoint's answer and never across a request, so two turns
/// starting at once on a cold cache make two requests rather than one blocking
/// the other. The same trade [`crate::llm::catalog::Catalog`] makes, for the
/// same reason: the answer is idempotent and the lock is not worth holding
/// across a network call.
#[derive(Debug)]
pub struct Registry {
    http: reqwest::Client,
    known: Mutex<HashMap<String, Known>>,
}

#[derive(Debug)]
struct Known {
    at: Instant,
    /// [`TTL`] for an endpoint that answered, [`COOLDOWN`] for one that could
    /// not be asked. Carried per entry rather than decided at the read, because
    /// what expires is the attempt rather than the endpoint.
    ttl: Duration,
    /// Only the models the endpoint published a modality list for. A model that
    /// is absent from this map is one nothing was said about, which is not the
    /// same as one said to take text: see the module comment.
    published: HashMap<String, Modalities>,
}

impl Registry {
    pub fn new() -> Self {
        Registry {
            http: reqwest::Client::builder().timeout(HTTP_TIMEOUT).build().unwrap_or_default(),
            known: Mutex::new(HashMap::new()),
        }
    }

    /// What Guaca will send this model, settled before the turn is assembled.
    ///
    /// The two subscription providers are answered without a request. Their
    /// models are the vendor's rather than the operator's (a ChatGPT plan runs
    /// what the plan runs, and `claude` runs whatever it is configured to),
    /// and every model either of them will hand a turn to takes pictures. There
    /// is no endpoint to ask and no list to read: `codex.rs` and `claude.rs`
    /// each already translate an image part into their own wire shape, which is
    /// this same fact written once in each of them.
    pub async fn of(&self, cfg: &InferenceConfig, model: &str) -> Modalities {
        match cfg.provider {
            Provider::Chatgpt | Provider::Claude => Modalities::seeing(),
            Provider::Compatible => {
                let answer =
                    self.published(cfg, model.trim()).await.unwrap_or_else(Modalities::seeing);
                // The one line that answers "why did my agent stop seeing
                // screenshots". Everything else about this is silent, and the
                // change it makes to a turn is three things not happening.
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

    /// What this endpoint published for this model, or `None` for a model it
    /// said nothing about.
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

    /// `None` when this endpoint has not been asked lately. The nesting is the
    /// distinction the module is about: `Some(None)` is an endpoint that was
    /// asked and said nothing about this model, which is an answer and does not
    /// want asking again.
    fn remembered(&self, base: &str, model: &str) -> Option<Option<Modalities>> {
        let held = self.known.lock();
        let entry = held.get(base)?;
        (entry.at.elapsed() < entry.ttl).then(|| entry.published.get(model).copied())
    }

    /// Reads the endpoint's model list, and how long to trust the reading.
    ///
    /// Never an error. Every way this can fail (no route, a refusal, a key
    /// this endpoint wants for its catalog, a body in some other shape) is an
    /// endpoint that published nothing, which is a state with a defined
    /// meaning. Turning any of them into a failure would mean a turn that
    /// cannot start because a nicety could not be looked up.
    async fn ask(
        &self,
        cfg: &InferenceConfig,
        base: &str,
    ) -> (HashMap<String, Modalities>, Duration) {
        let url = format!("{base}/models");
        let mut request = self.http.get(&url);
        // Sent when there is one, because a gateway in front of an endpoint
        // refuses an unauthenticated catalog, and left off when there is not:
        // a bare `Bearer` is a header some servers object to more than they
        // object to no header at all.
        let key = cfg.api_key.trim();
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
            // A 200 in some other shape is an endpoint that does not publish
            // this, not one that is broken: it answered.
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

// ---- the wire ------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct Listing {
    #[serde(default)]
    data: Vec<WireModel>,
}

#[derive(Debug, Deserialize)]
struct WireModel {
    #[serde(default)]
    id: String,
    /// OpenRouter's own addition to the OpenAI model object. Absent everywhere
    /// else, which is what makes an endpoint's silence readable.
    #[serde(default)]
    architecture: Option<WireArchitecture>,
}

#[derive(Debug, Deserialize)]
struct WireArchitecture {
    /// `["text", "image", "file"]`, as the vendor spells them.
    ///
    /// `output_modalities` sits beside it on the wire and is deliberately not
    /// read: see the module comment on why output is a constant here.
    #[serde(default)]
    input_modalities: Vec<String>,
}

impl Listing {
    /// Every model the endpoint actually said something about.
    ///
    /// A row with an empty list is dropped rather than read as text-only. The
    /// field defaults to empty when it is missing, so "published nothing" and
    /// "published an empty list" arrive here as the same value, and the one
    /// that is not a real answer is the one worth being wrong about.
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

    /// An endpoint answering `/models` with whatever is handed in, and a count
    /// of how many times it was asked.
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

    /// Two models, one of each, in OpenRouter's own shape.
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

    /// The one case any of this exists for.
    #[tokio::test]
    async fn a_model_published_as_text_only_is_not_sent_pictures() {
        let (base, _count) = stub(listing).await;

        let seen = Registry::new().of(&against(&base), "vendor/reading").await;

        assert_eq!(seen, Modalities::text_only());
    }

    /// A local LM Studio answers with ids and nothing else. Reading that as
    /// "takes text only" would take `use_screen` and every attachment away from
    /// an operator running a vision model on their own machine, silently.
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

    /// A model swapped to one this endpoint has never heard of. Not an error
    /// and not an answer: the model call is where a name that is wrong is
    /// reported, with a message naming it.
    #[tokio::test]
    async fn a_model_that_is_not_on_the_list_is_left_as_it_was() {
        let (base, _count) = stub(listing).await;

        let seen = Registry::new().of(&against(&base), "vendor/nobody-has-heard-of").await;

        assert_eq!(seen, Modalities::seeing());
    }

    #[tokio::test]
    async fn an_endpoint_with_no_such_route_is_asked_once_and_left_alone() {
        // Nothing answers `/models` here, which is every OpenAI-compatible
        // server that only implements the one endpoint this app calls.
        let (base, count) = stub(|| axum::http::StatusCode::NOT_FOUND.into_response()).await;
        let registry = Registry::new();

        assert_eq!(registry.of(&against(&base), "vendor/reading").await, Modalities::seeing());
        assert_eq!(registry.of(&against(&base), "vendor/reading").await, Modalities::seeing());

        // The cooldown is the point: an endpoint that is down is about to fail
        // the model call too, and a refused request in front of every turn adds
        // a round trip to an outage.
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn an_unreachable_endpoint_is_no_answer_rather_than_a_failure() {
        // Nothing is listening here, and nothing ever will be.
        let seen = Registry::new().of(&against("http://127.0.0.1:1/v1"), "vendor/reading").await;

        assert_eq!(seen, Modalities::seeing());
    }

    /// A network round trip in front of every model call, paid by every agent
    /// in the crew, to re-learn something that changes when a vendor ships.
    #[tokio::test]
    async fn one_endpoint_is_read_once_however_many_models_are_asked_about() {
        let (base, count) = stub(listing).await;
        let registry = Registry::new();

        assert_eq!(registry.of(&against(&base), "vendor/seeing").await, Modalities::seeing());
        assert_eq!(registry.of(&against(&base), "vendor/reading").await, Modalities::text_only());
        assert_eq!(registry.of(&against(&base), "vendor/seeing").await, Modalities::seeing());

        assert_eq!(count.load(Ordering::SeqCst), 1);
    }

    /// Both subscription providers run the vendor's own models, and every one
    /// of them takes pictures. There is no endpoint to ask, and asking the
    /// operator's would answer for a model that is not paying for this turn.
    #[tokio::test]
    async fn a_subscription_is_answered_without_asking_anybody() {
        let (base, count) = stub(listing).await;
        let registry = Registry::new();

        for provider in [Provider::Chatgpt, Provider::Claude] {
            let cfg = InferenceConfig { provider, ..against(&base) };
            // Named as the text-only model on purpose: the endpoint's list must
            // not be consulted for a turn that endpoint is not paying for.
            assert_eq!(registry.of(&cfg, "vendor/reading").await, Modalities::seeing());
        }

        assert_eq!(count.load(Ordering::SeqCst), 0);
    }

    /// The failure no stub can see.
    ///
    /// `architecture.input_modalities` is OpenRouter's own field, and every
    /// test above is this build agreeing with itself about it. Renamed or
    /// dropped on their side, every model becomes one nothing was published
    /// about, which is a state that looks exactly like the day before this
    /// existed: pictures sent to everything, `use_screen` offered to everything,
    /// and no offline suite any the wiser. Same shape as the live halves of
    /// `tests/plugins.rs` and [`crate::llm::catalog`].
    ///
    /// Asserted on the shape rather than on one slug, because a model delisted
    /// by a vendor is not this build going stale. The one exception is the
    /// model this app ships as its default: if that is gone, this repo has
    /// something to change either way.
    ///
    /// Reaches the internet, authorizes nothing and spends nothing.
    ///
    ///   cargo test --manifest-path src-tauri/Cargo.toml --lib \
    ///     llm::modality::tests::openrouter_still -- --ignored --nocapture
    #[tokio::test]
    #[ignore = "reaches OpenRouter"]
    async fn openrouter_still_publishes_what_each_of_its_models_takes() {
        let cfg = InferenceConfig::default();
        let (published, ttl) = Registry::new().ask(&cfg, cfg.base_url.trim_end_matches('/')).await;

        assert!(!published.is_empty(), "OpenRouter published no modalities for any model");
        assert_eq!(ttl, TTL, "an endpoint that answered is trusted for the full term");
        // Both kinds have to be readable, or a field that has quietly become a
        // constant reads as a catalog where everything can see.
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
