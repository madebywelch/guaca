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
//!
//! It also has the shape of the real service rather than the simplest one. Its
//! authorization server is mounted under `/api/auth`, so the issuer it publishes
//! is not its origin, and it names that issuer in the redirect. A stub at the
//! root agreed with a substitution the app was making, and passed every test in
//! this file while every sign-in against `guaca.bot` was refused.

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
    /// Sit at the root of the origin rather than under a path.
    ///
    /// Off by default, because `guaca.bot` mounts its authorization server at
    /// `/api/auth` and a stub at the root is what let this suite pass while
    /// every real sign-in was refused: the app substituted the origin for the
    /// issuer, and a root-mounted server is the one case where those are the
    /// same string.
    root_mounted: bool,
    /// Publish no `issuer` at all, which RFC 8414 requires and a server can
    /// still omit.
    omit_issuer: bool,
    /// Publish an issuer on another origin while keeping the endpoints here.
    /// Not the same failure as `foreign_endpoints`: the credential goes to the
    /// right place and the answer is then checked against a third party's name.
    foreign_issuer: bool,
    /// Name somebody else as the issuer in the redirect. RFC 9207's mix-up.
    wrong_iss: bool,
    /// Answer a refresh with a status rather than a token, the way a service
    /// does when the presented refresh token has just been rotated out from
    /// under the caller.
    refuse_refresh: bool,
    /// Take long enough over a refresh that everybody who wants a token during
    /// it is inside the same window. What makes a race a test rather than a
    /// coin toss.
    slow_refresh: bool,
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

    // Where the authorization server sits under the origin, which is what makes
    // its issuer identifier something other than that origin.
    let mount = if script.root_mounted { "" } else { "/api/auth" };
    // What the document says. The endpoints hang off it, so a foreign-endpoint
    // script moves all three at once.
    let published = format!("{metadata_origin}{mount}");
    // What the redirect names: this server, wherever the document pointed.
    let issued_by = format!("{origin}{mount}");

    let app = Router::new()
        .route(
            "/.well-known/oauth-authorization-server",
            get({
                let (published, script) = (published.clone(), script.clone());
                move || {
                    let (published, script) = (published.clone(), script.clone());
                    async move {
                        let mut body = serde_json::json!({
                            "issuer": published,
                            "authorization_endpoint": format!("{published}/oauth2/authorize"),
                            "token_endpoint": format!("{published}/oauth2/token"),
                            "code_challenge_methods_supported": ["S256"],
                            "scopes_supported": ["openid", "email", "offline_access", "connectors"],
                        });
                        if script.omit_issuer {
                            body.as_object_mut().expect("an object").remove("issuer");
                        }
                        if script.foreign_issuer {
                            body["issuer"] = "https://elsewhere.example".into();
                        }
                        Json(body)
                    }
                }
            }),
        )
        .route(
            &format!("{mount}/oauth2/authorize"),
            get({
                let asked = asked.clone();
                let script = script.clone();
                let issued_by = issued_by.clone();
                move |Query(query): Query<std::collections::HashMap<String, String>>| {
                    let (asked, script, issued_by) =
                        (asked.clone(), script.clone(), issued_by.clone());
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

                        // RFC 9207, percent-encoded the way a real server sends
                        // it. The app decodes before comparing, so a value that
                        // skipped the encoding would never exercise that.
                        let iss = urlencoding_encode(if script.wrong_iss {
                            "https://not-the-service.example"
                        } else {
                            &issued_by
                        });

                        let to = if script.deny {
                            format!(
                                "{back}?error=access_denied&error_description=Nope\
                                 &state={state}&iss={iss}"
                            )
                        } else {
                            format!("{back}?code=the-code&state={state}&iss={iss}")
                        };
                        Redirect::temporary(&to)
                    }
                }
            }),
        )
        .route(
            &format!("{mount}/oauth2/token"),
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

                        let refreshing = get("grant_type") == "refresh_token";
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

                        if refreshing && script.slow_refresh {
                            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                        }
                        if refreshing && script.refuse_refresh {
                            return (
                                axum::http::StatusCode::BAD_REQUEST,
                                Json(serde_json::json!({
                                    "error": "invalid_grant",
                                    "error_description": "invalid refresh token",
                                })),
                            )
                                .into_response();
                        }

                        // A renewal rotates both, which is what the real
                        // service does and the whole reason two callers must
                        // not present the same refresh token at once.
                        let mut answer = serde_json::json!({
                            "access_token": if refreshing { "at-2" } else { "at-1" },
                            "refresh_token": if refreshing { "rt-2" } else { "rt-1" },
                            "token_type": "Bearer",
                        });
                        if let Some(secs) = script.expires_in {
                            // The scripted expiry is what the *stored* token
                            // gets. A renewal answers with a full hour, because
                            // a test that handed back another expiring token
                            // would have every caller renewing forever and
                            // could never tell one refresh from two.
                            answer["expires_in"] = if refreshing { 3600 } else { secs }.into();
                        }
                        Json(answer).into_response()
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

/// Everything but the unreserved set, which is what a real server sends.
fn urlencoding_encode(raw: &str) -> String {
    raw.bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (byte as char).to_string()
            }
            other => format!("%{other:02X}"),
        })
        .collect()
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
async fn the_answer_is_checked_against_the_issuer_the_service_published() {
    // The live failure. `guaca.bot` mounts its authorization server at
    // `/api/auth`, so the issuer it publishes and returns in `iss` is
    // `https://guaca.bot/api/auth` and its origin is not. The app compared
    // against the origin, so every sign-in reached the consent screen, was
    // issued a code, and was refused on the way back.
    let stub = serve(Script { expires_in: Some(3600), ..Script::default() }).await;
    let account = Account::open_at(scratch(), &stub.origin);

    let status = account.sign_in(browse).await.expect("the sign-in should complete");
    assert!(status.signed_in);

    // And the endpoints it used really were the ones under the path, rather
    // than the flow having quietly stopped checking.
    let asked = stub.asked.lock().clone().expect("the browser was sent somewhere");
    assert!(asked.redirect_uri.starts_with("http://127.0.0.1:"), "{}", asked.redirect_uri);
    assert_eq!(stub.exchanges.lock().len(), 1, "the code was traded at the mounted endpoint");
}

#[tokio::test]
async fn an_answer_naming_another_issuer_is_refused() {
    // RFC 9207's mix-up: a code minted by a server the operator does not use,
    // presented to the one they do. Refused before the code is read, so there
    // is nothing to have traded.
    let stub = serve(Script { wrong_iss: true, ..Script::default() }).await;
    let account = Account::open_at(scratch(), &stub.origin);

    match account.sign_in(browse).await {
        Err(AccountError::IssuerMismatch { expected, named }) => {
            assert_eq!(expected, format!("{}/api/auth", stub.origin));
            assert_eq!(named, "https://not-the-service.example");
        }
        other => panic!("expected an issuer mismatch, got {other:?}"),
    }
    assert!(!account.is_signed_in(), "nothing is stored");
    assert!(stub.exchanges.lock().is_empty(), "the code was never traded");
}

#[tokio::test]
async fn a_service_that_publishes_no_issuer_is_checked_against_its_origin() {
    // RFC 8414 requires the field and a server can still leave it out. The
    // address the document was fetched from is what its absence means: the root
    // well-known path is the address of an issuer with no path. Substituting
    // the origin *unconditionally* is the bug; substituting it when nothing was
    // published is the reading.
    let stub = serve(Script { root_mounted: true, omit_issuer: true, ..Script::default() }).await;
    let account = Account::open_at(scratch(), &stub.origin);

    assert!(account.sign_in(browse).await.expect("the sign-in should complete").signed_in);
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
async fn a_service_that_names_someone_else_as_its_issuer_is_refused() {
    // The issuer is now what the redirect is checked against, so an unchecked
    // one is a value a third party could put there: a code minted anywhere
    // would arrive naming that issuer and be accepted. Refused at discovery,
    // where the endpoints already were.
    let stub = serve(Script { foreign_issuer: true, ..Script::default() }).await;
    let account = Account::open_at(scratch(), &stub.origin);

    match account.sign_in(browse).await {
        Err(AccountError::ForeignEndpoint { endpoint, .. }) => {
            assert_eq!(endpoint, "https://elsewhere.example");
        }
        other => panic!("expected a refused issuer, got {other:?}"),
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
async fn a_refused_refresh_is_reported_as_itself_rather_than_as_a_missing_sign_in() {
    // The failure this exists for. The service answered a renewal with a
    // status, the app read the absence of a token as "no account", and an agent
    // spent its turn telling the operator to sign in to an account that was
    // signed in and renewing normally ten seconds later. What the service said
    // has to survive the trip, because it is the whole difference between a
    // sign-in to redo and a bad few seconds at the token endpoint.
    let stub =
        serve(Script { expires_in: Some(1), refuse_refresh: true, ..Script::default() }).await;
    let account = Account::open_at(scratch(), &stub.origin);
    account.sign_in(browse).await.unwrap();

    let failed = account.connectors().await.expect_err("the renewal was refused");

    match failed {
        AccountError::Upstream { status, message, .. } => {
            assert_eq!(status, 400);
            assert!(message.contains("invalid_grant"), "what the service said: {message}");
        }
        other => panic!("a refused renewal is not a sign-out: {other:?}"),
    }
    // And the sign-in is still here, which is what makes "sign in again" the
    // wrong thing to tell anybody about this.
    assert!(account.is_signed_in());
}

#[tokio::test]
async fn one_renewal_serves_everybody_who_wanted_a_token_at_once() {
    // A crew all reaching for Google in the same second is the ordinary case,
    // and the service rotates the refresh token and revokes the one presented.
    // Two renewals at once is therefore one caller holding a token that has
    // just been thrown away, which comes back as `invalid_grant` and reads like
    // an account nobody is signed in to.
    let stub = serve(Script { expires_in: Some(1), slow_refresh: true, ..Script::default() }).await;
    let account = Account::open_at(scratch(), &stub.origin);
    account.sign_in(browse).await.unwrap();

    let (first, second) = tokio::join!(account.access(), account.access());

    assert_eq!(first.unwrap(), "at-2");
    assert_eq!(second.unwrap(), "at-2", "the second caller took the first one's answer");
    assert_eq!(refreshes(&stub), 1, "one renewal went out, not one per caller");
}

/// How many of the posts to the token endpoint were renewals.
fn refreshes(stub: &Stub) -> usize {
    stub.exchanges
        .lock()
        .iter()
        .filter(|fields| fields.iter().any(|(k, v)| k == "grant_type" && v == "refresh_token"))
        .count()
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

    // The issuer among them, because it is the value the redirect is checked
    // against and the one this suite could not see: the stub used to publish
    // its own origin, agreeing with a substitution the app was making, while
    // the live service published a path under it and every sign-in was refused.
    for field in ["authorization_endpoint", "token_endpoint", "issuer"] {
        let endpoint = body[field].as_str().unwrap_or_default();
        assert!(endpoint.starts_with(origin.trim_end_matches('/')), "{field} is {endpoint}");
    }
    // RFC 9207 is only a check if the service says it sends `iss`. A service
    // that stopped would leave the redirect unverified and nothing would fail.
    assert_eq!(
        body["authorization_response_iss_parameter_supported"].as_bool(),
        Some(true),
        "the service should still name the issuer in its redirect"
    );
    let methods = body["code_challenge_methods_supported"].as_array().cloned().unwrap_or_default();
    assert!(
        methods.iter().any(|m| m == "S256"),
        "S256 is the only challenge this build sends: {methods:?}"
    );
    // An open registration endpoint on this origin would mean the consent
    // screen can name an application a stranger asserted.
    assert!(body.get("registration_endpoint").is_none(), "client registration should be off");
}
