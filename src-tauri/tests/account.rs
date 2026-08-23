//! End-to-end tests for signing in to a Guaca account.
//!
//! A scripted authorization server, and the real [`Account`] driven against it:
//! discovery, the loopback listener, the PKCE challenge, the redirect, the
//! exchange, and the first call the token is spent on. Nothing is stubbed
//! inside the module, because the failures worth catching here are all seams.
//! `account.rs`'s own unit tests cover what can be decided without a network,
//! and every one of them would pass with the flow never completing.
//!
//! The stub asserts the parts of the protocol the service is holding up its end
//! of: a challenge is sent and it is S256, the client is the public one, the
//! redirect is loopback, and the verifier presented at the token endpoint
//! actually hashes to the challenge that was sent. A regression in the last one
//! is a sign-in that still works and no longer proves anything.

use std::sync::Arc;

use axum::extract::Query;
use axum::response::{IntoResponse, Redirect};
use axum::routing::{get, post};
use axum::{Json, Router};
use parking_lot::Mutex;
use sha2::{Digest, Sha256};

use guac_lib::account::{Account, AccountError, CLIENT_ID};

/// What the browser did with the authorization request, as the stub saw it.
#[derive(Debug, Clone, Default)]
struct Asked {
    client_id: String,
    redirect_uri: String,
    scope: String,
    state: String,
    challenge: String,
    method: String,
}

/// One posted form, as name/value pairs in the order they arrived.
type Form = Vec<(String, String)>;

struct Stub {
    origin: String,
    asked: Arc<Mutex<Option<Asked>>>,
    /// Bearer tokens the connectors endpoint was called with.
    presented: Arc<Mutex<Vec<String>>>,
    /// Every post to the token endpoint, so a refresh can be told from a grant.
    exchanges: Arc<Mutex<Vec<Form>>>,
}

/// How the stub should behave, so one server covers the failure paths too.
#[derive(Debug, Clone, Default)]
struct Script {
    /// Refuse in the browser rather than redirecting with a code.
    deny: bool,
    /// Answer the redirect with a state the app never sent.
    wrong_state: bool,
    /// Publish endpoints on another origin, which is the one thing discovery
    /// could do to move a credential.
    foreign_endpoints: bool,
    /// Refuse the bearer on the connectors endpoint.
    reject_token: bool,
    /// Seconds of life on the access token. `None` means the server did not say.
    expires_in: Option<i64>,
}

fn base64url(raw: &[u8]) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::new();
    for chunk in raw.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(TABLE[(n >> 18) as usize & 63] as char);
        out.push(TABLE[(n >> 12) as usize & 63] as char);
        if chunk.len() > 1 {
            out.push(TABLE[(n >> 6) as usize & 63] as char);
        }
        if chunk.len() > 2 {
            out.push(TABLE[n as usize & 63] as char);
        }
    }
    out
}

async fn serve(script: Script) -> Stub {
    let asked: Arc<Mutex<Option<Asked>>> = Arc::new(Mutex::new(None));
    let presented: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let exchanges: Arc<Mutex<Vec<Form>>> = Arc::new(Mutex::new(Vec::new()));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let origin = format!("http://{addr}");

    let metadata_origin =
        if script.foreign_endpoints { "http://127.0.0.1:9".to_string() } else { origin.clone() };

    let app = Router::new()
        .route(
            "/.well-known/oauth-authorization-server",
            get(move || {
                let base = metadata_origin.clone();
                async move {
                    Json(serde_json::json!({
                        "issuer": base,
                        "authorization_endpoint": format!("{base}/oauth2/authorize"),
                        "token_endpoint": format!("{base}/oauth2/token"),
                        "code_challenge_methods_supported": ["S256"],
                        "scopes_supported": ["openid", "email", "offline_access", "connectors"],
                    }))
                }
            }),
        )
        .route(
            "/oauth2/authorize",
            get({
                let asked = asked.clone();
                let script = script.clone();
                move |Query(query): Query<std::collections::HashMap<String, String>>| {
                    let (asked, script) = (asked.clone(), script.clone());
                    async move {
                        let seen = Asked {
                            client_id: query.get("client_id").cloned().unwrap_or_default(),
                            redirect_uri: query.get("redirect_uri").cloned().unwrap_or_default(),
                            scope: query.get("scope").cloned().unwrap_or_default(),
                            state: query.get("state").cloned().unwrap_or_default(),
                            challenge: query.get("code_challenge").cloned().unwrap_or_default(),
                            method: query.get("code_challenge_method").cloned().unwrap_or_default(),
                        };

                        // The protocol this end is holding up. A sign-in that
                        // still completes without a challenge is one that has
                        // stopped proving anything.
                        assert_eq!(seen.method, "S256", "only S256 is sent");
                        assert!(!seen.challenge.is_empty(), "a challenge is always sent");
                        assert!(
                            seen.redirect_uri.starts_with("http://127.0.0.1:"),
                            "the redirect must be a loopback port: {}",
                            seen.redirect_uri
                        );

                        let back = seen.redirect_uri.clone();
                        let state = if script.wrong_state {
                            "not-the-state".to_string()
                        } else {
                            seen.state.clone()
                        };
                        *asked.lock() = Some(seen);

                        let to = if script.deny {
                            format!(
                                "{back}?error=access_denied&error_description=Nope&state={state}"
                            )
                        } else {
                            format!("{back}?code=the-code&state={state}")
                        };
                        Redirect::temporary(&to)
                    }
                }
            }),
        )
        .route(
            "/oauth2/token",
            post({
                let asked = asked.clone();
                let exchanges = exchanges.clone();
                let script = script.clone();
                move |body: String| {
                    let (asked, exchanges, script) =
                        (asked.clone(), exchanges.clone(), script.clone());
                    async move {
                        let fields: Form = body
                            .split('&')
                            .filter_map(|pair| pair.split_once('='))
                            .map(|(k, v)| (k.to_string(), urlencoding_decode(v)))
                            .collect();
                        let get = |name: &str| {
                            fields
                                .iter()
                                .find(|(k, _)| k == name)
                                .map(|(_, v)| v.clone())
                                .unwrap_or_default()
                        };

                        if get("grant_type") == "authorization_code" {
                            // The whole point of PKCE, checked rather than
                            // assumed: the verifier presented here has to hash
                            // to the challenge sent to the browser.
                            let challenge = asked
                                .lock()
                                .as_ref()
                                .map(|a| a.challenge.clone())
                                .unwrap_or_default();
                            let verifier = get("code_verifier");
                            assert!(!verifier.is_empty(), "a verifier is always presented");
                            assert_eq!(
                                base64url(&Sha256::digest(verifier.as_bytes())),
                                challenge,
                                "the verifier must hash to the challenge that was sent"
                            );
                            assert_eq!(get("client_id"), CLIENT_ID);
                        }

                        exchanges.lock().push(fields);
                        let mut answer = serde_json::json!({
                            "access_token": "at-1",
                            "refresh_token": "rt-1",
                            "token_type": "Bearer",
                        });
                        if let Some(secs) = script.expires_in {
                            answer["expires_in"] = secs.into();
                        }
                        Json(answer)
                    }
                }
            }),
        )
        .route(
            "/api/connectors",
            get({
                let presented = presented.clone();
                let script = script.clone();
                move |headers: axum::http::HeaderMap| {
                    let (presented, script) = (presented.clone(), script.clone());
                    async move {
                        let token = headers
                            .get("authorization")
                            .and_then(|v| v.to_str().ok())
                            .unwrap_or_default()
                            .trim_start_matches("Bearer ")
                            .to_string();
                        presented.lock().push(token);

                        if script.reject_token {
                            return (axum::http::StatusCode::UNAUTHORIZED, "no").into_response();
                        }
                        Json(serde_json::json!({
                            "user": { "email": "robert@example.com" },
                            "providers": [{
                                "id": "google",
                                "label": "Google",
                                "capabilities": [
                                    { "id": "gmail", "label": "Gmail", "granted": true },
                                    { "id": "drive", "label": "Drive", "granted": false },
                                ],
                            }],
                        }))
                        .into_response()
                    }
                }
            }),
        );

    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Stub { origin, asked, presented, exchanges }
}

fn urlencoding_decode(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        match bytes[at] {
            b'%' if at + 2 < bytes.len() => match u8::from_str_radix(&raw[at + 1..at + 3], 16) {
                Ok(byte) => {
                    out.push(byte);
                    at += 3;
                }
                Err(_) => {
                    out.push(bytes[at]);
                    at += 1;
                }
            },
            b'+' => {
                out.push(b' ');
                at += 1;
            }
            byte => {
                out.push(byte);
                at += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).to_string()
}

/// A place to keep an account file that no other test shares.
fn scratch() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("guaca-account-it-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("account.json")
}

/// Plays the operator's browser: fetches the URL Guaca opened and follows the
/// redirect back to the loopback port, which is what the app is waiting on.
///
/// Spawned rather than awaited, because `sign_in` calls this and *then* starts
/// listening. Doing it inline would deadlock on a redirect nothing is accepting
/// yet, which is also exactly the ordering a real browser has.
fn browse(url: &str) -> Result<(), String> {
    let url = url.to_string();
    tokio::spawn(async move {
        let client =
            reqwest::Client::builder().timeout(std::time::Duration::from_secs(10)).build().unwrap();
        let _ = client.get(&url).send().await;
    });
    Ok(())
}

#[tokio::test]
async fn a_sign_in_completes_and_reports_the_account_it_reached() {
    let stub = serve(Script { expires_in: Some(3600), ..Script::default() }).await;
    let account = Account::open_at(scratch(), &stub.origin);

    let status = account.sign_in(browse).await.expect("the sign-in should complete");

    assert!(status.signed_in);
    assert_eq!(status.email, "robert@example.com", "read from the service, not guessed");
    assert_eq!(status.origin, stub.origin);

    let asked = stub.asked.lock().clone().expect("the browser was sent somewhere");
    assert_eq!(asked.client_id, CLIENT_ID);
    // Everything in one consent screen rather than asking again later.
    for scope in ["openid", "email", "offline_access", "connectors"] {
        assert!(asked.scope.contains(scope), "{scope} was not asked for: {}", asked.scope);
    }

    // The token was spent before anything was written, which is what makes a
    // reported success one that actually works.
    assert_eq!(stub.presented.lock().as_slice(), ["at-1"]);
}

#[tokio::test]
async fn the_sign_in_survives_a_restart() {
    let stub = serve(Script { expires_in: Some(3600), ..Script::default() }).await;
    let path = scratch();
    Account::open_at(path.clone(), &stub.origin).sign_in(browse).await.unwrap();

    // A second `Account` over the same file is what a relaunch is.
    let reopened = Account::open_at(path, &stub.origin);
    assert!(reopened.is_signed_in());
    assert_eq!(reopened.status().email, "robert@example.com");
    assert_eq!(reopened.connectors().await.unwrap().email, "robert@example.com");
}

#[tokio::test]
async fn what_the_account_holds_is_asked_of_the_service() {
    let stub = serve(Script { expires_in: Some(3600), ..Script::default() }).await;
    let account = Account::open_at(scratch(), &stub.origin);
    account.sign_in(browse).await.unwrap();

    let held = account.connectors().await.unwrap();
    let google = held.providers.iter().find(|p| p.id == "google").expect("google");
    assert!(google.capabilities.iter().any(|c| c.id == "gmail" && c.granted));
    assert!(google.capabilities.iter().any(|c| c.id == "drive" && !c.granted));

    // Two calls, not one cached answer. What is authorized changes in a browser
    // rather than here, so a kept copy is a list an agent is told is true.
    assert_eq!(stub.presented.lock().len(), 2);
}

#[tokio::test]
async fn a_refusal_in_the_browser_is_reported_rather_than_waited_out() {
    let stub = serve(Script { deny: true, ..Script::default() }).await;
    let account = Account::open_at(scratch(), &stub.origin);

    match account.sign_in(browse).await {
        Err(AccountError::Refused { error, .. }) => assert_eq!(error, "access_denied"),
        other => panic!("expected a refusal, got {other:?}"),
    }
    assert!(!account.is_signed_in(), "nothing is stored by a refused sign-in");
}

#[tokio::test]
async fn an_answer_that_does_not_match_the_request_is_treated_as_an_attack() {
    // Nothing else can arrive on that port with the wrong state, so this is not
    // a mistake to recover from.
    let stub = serve(Script { wrong_state: true, ..Script::default() }).await;
    let account = Account::open_at(scratch(), &stub.origin);

    assert!(matches!(account.sign_in(browse).await, Err(AccountError::StateMismatch)));
    assert!(!account.is_signed_in());
}

#[tokio::test]
async fn a_service_that_publishes_endpoints_somewhere_else_is_refused() {
    // The one thing discovery could do to move a credential. Refused before a
    // browser is opened, so nothing is sent anywhere.
    let stub = serve(Script { foreign_endpoints: true, ..Script::default() }).await;
    let account = Account::open_at(scratch(), &stub.origin);

    match account.sign_in(browse).await {
        Err(AccountError::ForeignEndpoint { endpoint, .. }) => {
            assert!(endpoint.starts_with("http://127.0.0.1:9"), "{endpoint}");
        }
        other => panic!("expected a refused endpoint, got {other:?}"),
    }
    assert!(stub.asked.lock().is_none(), "the browser was never opened");
}

#[tokio::test]
async fn a_token_the_service_will_not_take_is_not_stored_as_a_sign_in() {
    // Spending it once before writing is what stops "signed in" meaning "held a
    // token the service has never accepted".
    let stub = serve(Script { reject_token: true, ..Script::default() }).await;
    let account = Account::open_at(scratch(), &stub.origin);

    assert!(matches!(account.sign_in(browse).await, Err(AccountError::Expired { .. })));
    assert!(!account.is_signed_in());
}

#[tokio::test]
async fn an_expiring_token_is_refreshed_before_it_is_used() {
    // Already expired as far as the skew is concerned, so the first read of it
    // has to renew rather than hand back something a call would be refused for.
    let stub = serve(Script { expires_in: Some(1), ..Script::default() }).await;
    let account = Account::open_at(scratch(), &stub.origin);
    account.sign_in(browse).await.unwrap();

    account.connectors().await.unwrap();

    let grants: Vec<String> = stub
        .exchanges
        .lock()
        .iter()
        .map(|fields| {
            fields
                .iter()
                .find(|(k, _)| k == "grant_type")
                .map(|(_, v)| v.clone())
                .unwrap_or_default()
        })
        .collect();
    assert_eq!(grants, ["authorization_code", "refresh_token"]);
}

#[tokio::test]
async fn a_token_with_no_stated_expiry_is_used_rather_than_renewed_every_call() {
    // A server that does not say is not a server saying "expired".
    let stub = serve(Script { expires_in: None, ..Script::default() }).await;
    let account = Account::open_at(scratch(), &stub.origin);
    account.sign_in(browse).await.unwrap();

    account.connectors().await.unwrap();
    assert_eq!(stub.exchanges.lock().len(), 1, "no refresh was asked for");
}

#[tokio::test]
async fn signing_out_leaves_nothing_a_later_call_could_present() {
    let stub = serve(Script { expires_in: Some(3600), ..Script::default() }).await;
    let path = scratch();
    let account = Account::open_at(path.clone(), &stub.origin);
    account.sign_in(browse).await.unwrap();

    account.sign_out().unwrap();
    assert!(!account.is_signed_in());
    assert!(!path.exists(), "the file goes, not just the copy in memory");
    assert!(matches!(account.connectors().await, Err(AccountError::NotSignedIn)));
    // And a relaunch finds nothing either.
    assert!(!Account::open_at(path, &stub.origin).is_signed_in());
}

/// Whether the real service still publishes what this build knows how to read.
///
/// The failure no offline test can see: `guaca.bot` moves an endpoint, or stops
/// publishing RFC 8414 metadata at the root of the origin, and every sign-in
/// fails at step one. It authorizes nothing and stores nothing.
///
/// `GUACA_ACCOUNT_ORIGIN` points it at a Worker on this machine instead.
///
/// ```sh
/// cargo test --manifest-path src-tauri/Cargo.toml --test account -- --ignored
/// ```
#[tokio::test]
#[ignore = "reaches the internet"]
async fn the_real_service_still_publishes_where_to_sign_in() {
    let origin = std::env::var("GUACA_ACCOUNT_ORIGIN")
        .unwrap_or_else(|_| guac_lib::account::DEFAULT_ORIGIN.to_string());
    let account = Account::open_at(scratch(), &origin);

    // Not a sign-in: this stops at discovery, which is the half a machine can
    // check. `sign_in` would need a browser and a person.
    match account.connectors().await {
        Err(AccountError::NotSignedIn) => {}
        other => panic!("expected the local check to refuse, got {other:?}"),
    }

    let http = reqwest::Client::new();
    let url = format!("{}/.well-known/oauth-authorization-server", origin.trim_end_matches('/'));
    let body: serde_json::Value =
        http.get(&url).send().await.expect("metadata unreachable").json().await.expect("not json");

    for field in ["authorization_endpoint", "token_endpoint"] {
        let endpoint = body[field].as_str().unwrap_or_default();
        assert!(endpoint.starts_with(origin.trim_end_matches('/')), "{field} is {endpoint}");
    }
    let methods = body["code_challenge_methods_supported"].as_array().cloned().unwrap_or_default();
    assert!(
        methods.iter().any(|m| m == "S256"),
        "S256 is the only challenge this build sends: {methods:?}"
    );
    // An open registration endpoint on this origin would mean the consent
    // screen can name an application a stranger asserted.
    assert!(body.get("registration_endpoint").is_none(), "client registration should be off");
}
