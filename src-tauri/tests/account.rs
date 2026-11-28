use std::sync::Arc;

use axum::extract::Query;
use axum::response::{IntoResponse, Redirect};
use axum::routing::{get, post};
use axum::{Json, Router};
use parking_lot::Mutex;
use sha2::{Digest, Sha256};

use guac_lib::account::{Account, AccountError, CLIENT_ID, REMOTE_CLIENT_ID, REMOTE_RETURN_PATH};
use guac_lib::oauth::{Callbacks, Landing};

#[tokio::test]
async fn broker_handoff_and_status_use_the_configured_bearer() {
    let presented = Arc::new(Mutex::new(Vec::new()));
    let app = Router::new()
        .route(
            "/api/selfhost/handoff",
            post({
                let presented = presented.clone();
                move |headers: axum::http::HeaderMap| {
                    let presented = presented.clone();
                    async move {
                        presented
                            .lock()
                            .push(headers["authorization"].to_str().unwrap().to_string());
                        Json(serde_json::json!({"code": "a+b /"}))
                    }
                }
            }),
        )
        .route(
            "/api/connectors",
            get(|headers: axum::http::HeaderMap| async move {
                assert_eq!(headers["authorization"], "Bearer secret");
                Json(serde_json::json!({"user":{"email":"person@example.test"},"providers":[]}))
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let path = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
    let account = Account::open_at(path.clone(), &origin).with_broker("secret".into());
    let status = account.checked_status().await;
    assert!(status.signed_in && status.broker);
    let opened = Mutex::new(String::new());
    account
        .open_broker(|url| {
            *opened.lock() = url.to_string();
            Ok(())
        })
        .await
        .unwrap();
    assert_eq!(*opened.lock(), format!("{origin}/enter?code=a%2Bb%20%2F"));
    assert_eq!(*presented.lock(), vec!["Bearer secret"]);
    assert!(!path.exists());
    assert!(account.sign_out().unwrap_err().to_string().contains("GUACA_BROKER_BEARER"));
}

#[tokio::test]
async fn broker_requires_safe_explicit_origin_and_a_live_service() {
    let path = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
    for account in [
        Account::open(path.clone()).with_broker("secret".into()),
        Account::open_at(path.clone(), "http://example.test").with_broker("secret".into()),
        Account::open_at(path.clone(), "https://name:password@example.test")
            .with_broker("secret".into()),
        Account::open_at(path.clone(), "http://127.0.0.1:9").with_broker("secret".into()),
    ] {
        assert!(!account.checked_status().await.signed_in);
        assert!(account.open_broker(|_| panic!("must not open browser")).await.is_err());
    }
}

#[tokio::test]
async fn unavailable_and_invalid_brokers_remain_configured() {
    let app = Router::new()
        .route("/api/connectors", get(|| async { (axum::http::StatusCode::UNAUTHORIZED, "no") }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    for origin in [&origin[..], "http://127.0.0.1:9"] {
        let account = Account::open_at(scratch(), origin).with_broker("secret".into());
        let status = account.checked_status().await;
        assert!(status.broker);
        assert!(!status.signed_in);
        assert!(account.has_broker());
        assert!(matches!(account.sign_out(), Err(AccountError::BrokerConfigured)));
    }
}

#[tokio::test]
async fn redirects_remain_available_to_normal_accounts_but_not_brokers() {
    let app = Router::new()
        .route("/api/connectors", get(|| async { Redirect::temporary("/target") }))
        .route(
            "/target",
            get(|| async {
                Json(serde_json::json!({"user":{"email":"person@example.test"},"providers":[]}))
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let path = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
    std::fs::write(
        &path,
        serde_json::json!({
            "access_token": "test", "token_endpoint": format!("{origin}/token"), "origin": origin
        })
        .to_string(),
    )
    .unwrap();
    let normal = Account::open_at(path.clone(), &origin);
    assert!(normal.connectors().await.is_ok());
    let broker = Account::open_at(path.clone(), &origin).with_broker("secret".into());
    assert!(!broker.checked_status().await.signed_in);
    assert!(broker.connectors().await.is_err());
    std::fs::remove_file(path).unwrap();
}

#[derive(Debug, Clone, Default)]
struct Asked {
    client_id: String,
    redirect_uri: String,
    scope: String,
    state: String,
    challenge: String,
    method: String,
    return_to: Option<String>,
}

#[derive(Clone)]
struct Served(Landing);

impl std::fmt::Debug for Served {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Served")
    }
}

type Form = Vec<(String, String)>;
type BlockGrant = (
    bool,
    Arc<tokio::sync::Semaphore>,
    Arc<tokio::sync::Semaphore>,
    Arc<std::sync::atomic::AtomicBool>,
);

struct Stub {
    origin: String,
    asked: Arc<Mutex<Option<Asked>>>,
    presented: Arc<Mutex<Vec<String>>>,
    exchanges: Arc<Mutex<Vec<Form>>>,
}

#[derive(Debug, Clone, Default)]
struct Script {
    block_grant: Option<BlockGrant>,
    deny: bool,
    wrong_state: bool,
    foreign_endpoints: bool,
    reject_token: bool,
    expires_in: Option<i64>,
    root_mounted: bool,
    omit_issuer: bool,
    foreign_issuer: bool,
    wrong_iss: bool,
    refuse_refresh: bool,
    slow_refresh: bool,
    served: Option<Served>,
    secret_tokens: bool,
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

    let mount = if script.root_mounted { "" } else { "/api/auth" };
    let published = format!("{metadata_origin}{mount}");
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
                let back_to = format!("{origin}{REMOTE_RETURN_PATH}");
                move |Query(query): Query<std::collections::HashMap<String, String>>| {
                    let (asked, script, issued_by, back_to) =
                        (asked.clone(), script.clone(), issued_by.clone(), back_to.clone());
                    async move {
                        let seen = Asked {
                            client_id: query.get("client_id").cloned().unwrap_or_default(),
                            redirect_uri: query.get("redirect_uri").cloned().unwrap_or_default(),
                            scope: query.get("scope").cloned().unwrap_or_default(),
                            state: query.get("state").cloned().unwrap_or_default(),
                            challenge: query.get("code_challenge").cloned().unwrap_or_default(),
                            method: query.get("code_challenge_method").cloned().unwrap_or_default(),
                            return_to: query.get("return_to").cloned(),
                        };

                        assert_eq!(seen.method, "S256", "only S256 is sent");
                        assert!(!seen.challenge.is_empty(), "a challenge is always sent");
                        if seen.client_id == REMOTE_CLIENT_ID {
                            assert_eq!(
                                seen.redirect_uri, back_to,
                                "a remote sign-in returns to the service's own page"
                            );
                            let Some(Served(landing)) = script.served.clone() else {
                                panic!("a remote sign-in with no served landing");
                            };
                            let fields = vec![
                                ("code".to_string(), "the-code".to_string()),
                                ("state".to_string(), seen.state.clone()),
                                ("iss".to_string(), issued_by.clone()),
                            ];
                            *asked.lock() = Some(seen);
                            tokio::spawn(async move { landing.deliver(fields).await });
                            return "approved on the service's page".into_response();
                        }
                        assert!(
                            seen.redirect_uri.starts_with("http://127.0.0.1:"),
                            "the redirect must be a loopback port: {}",
                            seen.redirect_uri
                        );
                        assert_eq!(seen.return_to, None, "a loopback sign-in names no host");

                        let back = seen.redirect_uri.clone();
                        let state = if script.wrong_state {
                            "not-the-state".to_string()
                        } else {
                            seen.state.clone()
                        };
                        *asked.lock() = Some(seen);

                        let iss = urlencoding_encode(if script.wrong_iss {
                            "https://not-the-service.example"
                        } else {
                            &issued_by
                        });

                        if let Some(Served(landing @ Landing::Served { .. })) = script.served {
                            let fields = vec![
                                ("code".to_string(), "the-code".to_string()),
                                ("state".to_string(), state),
                                ("iss".to_string(), issued_by.clone()),
                            ];
                            tokio::spawn(async move { landing.deliver(fields).await });
                            return "answered at the workspace".into_response();
                        }

                        let to = if script.deny {
                            format!(
                                "{back}?error=access_denied&error_description=Nope\
                                 &state={state}&iss={iss}"
                            )
                        } else {
                            format!("{back}?code=the-code&state={state}&iss={iss}")
                        };
                        Redirect::temporary(&to).into_response()
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
                        let signed_in_as = asked.lock().as_ref().map(|a| a.client_id.clone());
                        if let Some(client) = &signed_in_as {
                            assert_eq!(&get("client_id"), client, "one client from start to renew");
                        }
                        if get("grant_type") == "authorization_code" {
                            let (challenge, redirect_uri) = asked
                                .lock()
                                .as_ref()
                                .map(|a| (a.challenge.clone(), a.redirect_uri.clone()))
                                .unwrap_or_default();
                            assert_eq!(get("redirect_uri"), redirect_uri);
                            let verifier = get("code_verifier");
                            assert!(!verifier.is_empty(), "a verifier is always presented");
                            assert_eq!(
                                base64url(&Sha256::digest(verifier.as_bytes())),
                                challenge,
                                "the verifier must hash to the challenge that was sent"
                            );
                        }

                        exchanges.lock().push(fields);

                        if let Some((block_refresh, entered, release, blocked)) =
                            &script.block_grant
                        {
                            if refreshing == *block_refresh
                                && !blocked.swap(true, std::sync::atomic::Ordering::SeqCst)
                            {
                                entered.add_permits(1);
                                release.acquire().await.unwrap().forget();
                            }
                        }

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

                        let (access, refresh) = match (script.secret_tokens, refreshing) {
                            (true, false) => (SECRET_ACCESS, SECRET_REFRESH),
                            (true, true) => (SECRET_ACCESS_2, SECRET_REFRESH_2),
                            (false, false) => ("at-1", "rt-1"),
                            (false, true) => ("at-2", "rt-2"),
                        };
                        let mut answer = serde_json::json!({
                            "access_token": access,
                            "refresh_token": refresh,
                            "token_type": "Bearer",
                        });
                        if let Some(secs) = script.expires_in {
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

fn scratch() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("guaca-account-it-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("account.json")
}

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

    let status =
        account.sign_in(browse, &Landing::Loopback).await.expect("the sign-in should complete");

    assert!(status.signed_in);
    assert_eq!(status.email, "robert@example.com", "read from the service, not guessed");
    assert_eq!(status.origin, stub.origin);

    let asked = stub.asked.lock().clone().expect("the browser was sent somewhere");
    assert_eq!(asked.client_id, CLIENT_ID);
    for scope in ["openid", "email", "offline_access", "connectors"] {
        assert!(asked.scope.contains(scope), "{scope} was not asked for: {}", asked.scope);
    }

    assert_eq!(stub.presented.lock().as_slice(), ["at-1"]);
}

#[tokio::test]
async fn the_sign_in_survives_a_restart() {
    let stub = serve(Script { expires_in: Some(3600), ..Script::default() }).await;
    let path = scratch();
    Account::open_at(path.clone(), &stub.origin).sign_in(browse, &Landing::Loopback).await.unwrap();

    let reopened = Account::open_at(path, &stub.origin);
    assert!(reopened.is_signed_in());
    assert_eq!(reopened.status().email, "robert@example.com");
    assert_eq!(reopened.connectors().await.unwrap().email, "robert@example.com");
}

#[tokio::test]
async fn what_the_account_holds_is_asked_of_the_service() {
    let stub = serve(Script { expires_in: Some(3600), ..Script::default() }).await;
    let account = Account::open_at(scratch(), &stub.origin);
    account.sign_in(browse, &Landing::Loopback).await.unwrap();

    let held = account.connectors().await.unwrap();
    let google = held.providers.iter().find(|p| p.id == "google").expect("google");
    assert!(google.capabilities.iter().any(|c| c.id == "gmail" && c.granted));
    assert!(google.capabilities.iter().any(|c| c.id == "drive" && !c.granted));

    assert_eq!(stub.presented.lock().len(), 2);
}

#[tokio::test]
async fn a_refusal_in_the_browser_is_reported_rather_than_waited_out() {
    let stub = serve(Script { deny: true, ..Script::default() }).await;
    let account = Account::open_at(scratch(), &stub.origin);

    match account.sign_in(browse, &Landing::Loopback).await {
        Err(AccountError::Refused { error, .. }) => assert_eq!(error, "access_denied"),
        other => panic!("expected a refusal, got {other:?}"),
    }
    assert!(!account.is_signed_in(), "nothing is stored by a refused sign-in");
}

#[tokio::test]
async fn a_wrong_state_is_rejected_before_a_valid_answer_completes_signin() {
    let stub = serve(Script::default()).await;
    let account = Account::open_at(scratch(), &stub.origin);
    let exchanges = stub.exchanges.clone();
    let (done, browser) = tokio::sync::oneshot::channel();
    let result = account
        .sign_in(
            move |url| {
                let url = url.to_owned();
                tokio::spawn(async move {
                    let client = reqwest::Client::builder()
                        .redirect(reqwest::redirect::Policy::none())
                        .build()
                        .unwrap();
                    let response = client.get(url).send().await.unwrap();
                    let valid = response.headers().get("location").unwrap().to_str().unwrap();
                    let mut forged = reqwest::Url::parse(valid).unwrap();
                    let query: Vec<_> = forged
                        .query_pairs()
                        .map(|(k, v)| (k.into_owned(), v.into_owned()))
                        .collect();
                    forged.set_query(None);
                    for (key, value) in query {
                        forged
                            .query_pairs_mut()
                            .append_pair(&key, if key == "state" { "wrong" } else { &value });
                    }
                    let rejected = client.get(forged).send().await.unwrap();
                    assert_eq!(rejected.status(), reqwest::StatusCode::BAD_REQUEST);
                    assert!(exchanges.lock().is_empty(), "the forged code must not be exchanged");
                    let accepted = client.get(valid).send().await.unwrap();
                    assert_eq!(accepted.status(), reqwest::StatusCode::OK);
                    done.send(()).unwrap();
                });
                Ok(())
            },
            &Landing::Loopback,
        )
        .await;
    browser.await.unwrap();
    assert!(result.unwrap().signed_in);
    assert_eq!(stub.exchanges.lock().len(), 1);
}

#[tokio::test]
async fn the_answer_is_checked_against_the_issuer_the_service_published() {
    let stub = serve(Script { expires_in: Some(3600), ..Script::default() }).await;
    let account = Account::open_at(scratch(), &stub.origin);

    let status =
        account.sign_in(browse, &Landing::Loopback).await.expect("the sign-in should complete");
    assert!(status.signed_in);

    let asked = stub.asked.lock().clone().expect("the browser was sent somewhere");
    assert!(asked.redirect_uri.starts_with("http://127.0.0.1:"), "{}", asked.redirect_uri);
    assert_eq!(stub.exchanges.lock().len(), 1, "the code was traded at the mounted endpoint");
}

#[tokio::test]
async fn an_answer_naming_another_issuer_is_refused() {
    let stub = serve(Script { wrong_iss: true, ..Script::default() }).await;
    let account = Account::open_at(scratch(), &stub.origin);

    match account.sign_in(browse, &Landing::Loopback).await {
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
    let stub = serve(Script { root_mounted: true, omit_issuer: true, ..Script::default() }).await;
    let account = Account::open_at(scratch(), &stub.origin);

    assert!(
        account
            .sign_in(browse, &Landing::Loopback)
            .await
            .expect("the sign-in should complete")
            .signed_in
    );
}

#[tokio::test]
async fn a_service_that_publishes_endpoints_somewhere_else_is_refused() {
    let stub = serve(Script { foreign_endpoints: true, ..Script::default() }).await;
    let account = Account::open_at(scratch(), &stub.origin);

    match account.sign_in(browse, &Landing::Loopback).await {
        Err(AccountError::ForeignEndpoint { endpoint, .. }) => {
            assert!(endpoint.starts_with("http://127.0.0.1:9"), "{endpoint}");
        }
        other => panic!("expected a refused endpoint, got {other:?}"),
    }
    assert!(stub.asked.lock().is_none(), "the browser was never opened");
}

#[tokio::test]
async fn a_service_that_names_someone_else_as_its_issuer_is_refused() {
    let stub = serve(Script { foreign_issuer: true, ..Script::default() }).await;
    let account = Account::open_at(scratch(), &stub.origin);

    match account.sign_in(browse, &Landing::Loopback).await {
        Err(AccountError::ForeignEndpoint { endpoint, .. }) => {
            assert_eq!(endpoint, "https://elsewhere.example");
        }
        other => panic!("expected a refused issuer, got {other:?}"),
    }
    assert!(stub.asked.lock().is_none(), "the browser was never opened");
}

#[tokio::test]
async fn a_token_the_service_will_not_take_is_not_stored_as_a_sign_in() {
    let stub = serve(Script { reject_token: true, ..Script::default() }).await;
    let account = Account::open_at(scratch(), &stub.origin);

    assert!(matches!(
        account.sign_in(browse, &Landing::Loopback).await,
        Err(AccountError::Expired { .. })
    ));
    assert!(!account.is_signed_in());
}

#[tokio::test]
async fn an_expiring_token_is_refreshed_before_it_is_used() {
    let stub = serve(Script { expires_in: Some(1), ..Script::default() }).await;
    let account = Account::open_at(scratch(), &stub.origin);
    account.sign_in(browse, &Landing::Loopback).await.unwrap();

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
    let stub =
        serve(Script { expires_in: Some(1), refuse_refresh: true, ..Script::default() }).await;
    let account = Account::open_at(scratch(), &stub.origin);
    account.sign_in(browse, &guac_lib::oauth::Landing::Loopback).await.unwrap();

    let failed = account.connectors().await.expect_err("the renewal was refused");

    match failed {
        AccountError::Upstream { status, message, .. } => {
            assert_eq!(status, 400);
            assert!(message.contains("invalid_grant"), "what the service said: {message}");
        }
        other => panic!("a refused renewal is not a sign-out: {other:?}"),
    }
    assert!(account.is_signed_in());
}

#[tokio::test]
async fn one_renewal_serves_everybody_who_wanted_a_token_at_once() {
    let stub = serve(Script { expires_in: Some(1), slow_refresh: true, ..Script::default() }).await;
    let account = Account::open_at(scratch(), &stub.origin);
    account.sign_in(browse, &guac_lib::oauth::Landing::Loopback).await.unwrap();

    let (first, second) = tokio::join!(account.access(), account.access());

    assert_eq!(first.unwrap().expose(), "at-2");
    assert_eq!(second.unwrap().expose(), "at-2", "the second caller took the first one's answer");
    assert_eq!(refreshes(&stub), 1, "one renewal went out, not one per caller");
}

fn refreshes(stub: &Stub) -> usize {
    stub.exchanges
        .lock()
        .iter()
        .filter(|fields| fields.iter().any(|(k, v)| k == "grant_type" && v == "refresh_token"))
        .count()
}

#[tokio::test]
async fn a_token_with_no_stated_expiry_is_used_rather_than_renewed_every_call() {
    let stub = serve(Script { expires_in: None, ..Script::default() }).await;
    let account = Account::open_at(scratch(), &stub.origin);
    account.sign_in(browse, &Landing::Loopback).await.unwrap();

    account.connectors().await.unwrap();
    assert_eq!(stub.exchanges.lock().len(), 1, "no refresh was asked for");
}

#[tokio::test]
async fn signing_out_leaves_nothing_a_later_call_could_present() {
    let stub = serve(Script { expires_in: Some(3600), ..Script::default() }).await;
    let path = scratch();
    let account = Account::open_at(path.clone(), &stub.origin);
    account.sign_in(browse, &Landing::Loopback).await.unwrap();

    account.sign_out().unwrap();
    assert!(!account.is_signed_in());
    assert!(!path.exists(), "the file goes, not just the copy in memory");
    assert!(matches!(account.connectors().await, Err(AccountError::NotSignedIn)));
    assert!(!Account::open_at(path, &stub.origin).is_signed_in());
}

async fn sign_out_wins_over_blocked_operation(refreshing: bool) {
    let entered = Arc::new(tokio::sync::Semaphore::new(0));
    let release = Arc::new(tokio::sync::Semaphore::new(0));
    let stub = serve(Script {
        expires_in: Some(1),
        block_grant: Some((
            refreshing,
            entered.clone(),
            release.clone(),
            Arc::new(std::sync::atomic::AtomicBool::new(false)),
        )),
        ..Script::default()
    })
    .await;
    let path = scratch();
    let account = Arc::new(Account::open_at(path.clone(), &stub.origin));
    if refreshing {
        account.sign_in(browse, &Landing::Loopback).await.unwrap();
    }
    let pending = {
        let account = account.clone();
        tokio::spawn(async move {
            if refreshing {
                account.access().await.map(|_| ())
            } else {
                account.sign_in(browse, &Landing::Loopback).await.map(|_| ())
            }
        })
    };
    entered.acquire().await.unwrap().forget();
    account.sign_out().unwrap();
    release.add_permits(1);
    let _ = pending.await.unwrap();
    assert!(!account.is_signed_in(), "stale operation restored the account");
    assert!(!path.exists());
    assert!(!Account::open_at(path, &stub.origin).is_signed_in());
}

#[tokio::test]
async fn sign_out_wins_over_blocked_refresh() {
    sign_out_wins_over_blocked_operation(true).await;
}

#[tokio::test]
async fn sign_out_wins_over_blocked_exchange() {
    sign_out_wins_over_blocked_operation(false).await;
}

#[tokio::test]
async fn newer_sign_in_wins_over_blocked_refresh() {
    let entered = Arc::new(tokio::sync::Semaphore::new(0));
    let release = Arc::new(tokio::sync::Semaphore::new(0));
    let stub = serve(Script {
        expires_in: Some(1),
        block_grant: Some((
            true,
            entered.clone(),
            release.clone(),
            Arc::new(std::sync::atomic::AtomicBool::new(false)),
        )),
        ..Script::default()
    })
    .await;
    let path = scratch();
    let account = Arc::new(Account::open_at(path.clone(), &stub.origin));
    account.sign_in(browse, &Landing::Loopback).await.unwrap();
    let pending = {
        let account = account.clone();
        tokio::spawn(async move { account.access().await })
    };
    entered.acquire().await.unwrap().forget();
    account.sign_in(browse, &Landing::Loopback).await.unwrap();
    release.add_permits(1);
    let _ = pending.await.unwrap();
    let stored = std::fs::read_to_string(&path).unwrap();
    assert_eq!(serde_json::from_str::<serde_json::Value>(&stored).unwrap()["access_token"], "at-1");
}

const SECRET_ACCESS: &str = "remote-access-6f1c0e";
const SECRET_REFRESH: &str = "remote-refresh-9b27d4";
const SECRET_ACCESS_2: &str = "remote-access-renewed-41aa";
const SECRET_REFRESH_2: &str = "remote-refresh-renewed-07ce";

const SPACE: &str = "https://my-space.fly.dev";

fn served(origin: &str) -> Landing {
    let callbacks: Callbacks = Default::default();
    Landing::Served { origin: origin.to_string(), callbacks }
}

fn field(fields: &[(String, String)], name: &str) -> String {
    fields.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone()).unwrap_or_default()
}

static LOGS: std::sync::OnceLock<Arc<std::sync::Mutex<Vec<u8>>>> = std::sync::OnceLock::new();

fn logs() -> Arc<std::sync::Mutex<Vec<u8>>> {
    LOGS.get_or_init(|| {
        let buffer = Arc::new(std::sync::Mutex::new(Vec::new()));
        let writer = buffer.clone();
        let _ = tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::new("trace"))
            .with_ansi(false)
            .with_writer(move || Lines(writer.clone()))
            .try_init();
        buffer
    })
    .clone()
}

struct Lines(Arc<std::sync::Mutex<Vec<u8>>>);

impl std::io::Write for Lines {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn a_host_reached_over_the_network_signs_in_through_the_services_own_page() {
    let landing = served(SPACE);
    let stub = serve(Script {
        expires_in: Some(3600),
        served: Some(Served(landing.clone())),
        ..Script::default()
    })
    .await;
    let account = Account::open_at(scratch(), &stub.origin);

    let status = account.sign_in(browse, &landing).await.expect("the sign-in should complete");

    assert!(status.signed_in);
    assert_eq!(status.email, "robert@example.com");
    let asked = stub.asked.lock().clone().expect("the browser was sent somewhere");
    assert_eq!(asked.client_id, REMOTE_CLIENT_ID);
    assert_eq!(asked.redirect_uri, format!("{}{REMOTE_RETURN_PATH}", stub.origin));
    assert_eq!(asked.return_to.as_deref(), Some("https://my-space.fly.dev/v1/oauth/callback"));
    let exchanges = stub.exchanges.lock().clone();
    assert_eq!(field(&exchanges[0], "client_id"), REMOTE_CLIENT_ID);
    assert_eq!(field(&exchanges[0], "redirect_uri"), asked.redirect_uri);
}

#[tokio::test]
async fn a_remote_sign_in_renews_as_the_client_it_signed_in_as_after_a_restart() {
    let landing = served(SPACE);
    let stub = serve(Script {
        expires_in: Some(1),
        served: Some(Served(landing.clone())),
        ..Script::default()
    })
    .await;
    let path = scratch();
    Account::open_at(path.clone(), &stub.origin).sign_in(browse, &landing).await.unwrap();

    let reopened = Account::open_at(path, &stub.origin);
    assert_eq!(reopened.access().await.unwrap().expose(), "at-2");
    let exchanges = stub.exchanges.lock().clone();
    assert_eq!(field(&exchanges[1], "grant_type"), "refresh_token");
    assert_eq!(field(&exchanges[1], "client_id"), REMOTE_CLIENT_ID);
}

#[tokio::test]
async fn a_loopback_sign_in_asks_and_stores_exactly_as_before() {
    for landing in [Landing::Loopback, served("http://localhost:8788")] {
        let stub = serve(Script {
            expires_in: Some(1),
            served: Some(Served(landing.clone())),
            ..Script::default()
        })
        .await;
        let path = scratch();
        let account = Account::open_at(path.clone(), &stub.origin);
        let opened = Arc::new(Mutex::new(String::new()));
        let seen = opened.clone();
        account
            .sign_in(
                move |url| {
                    *seen.lock() = url.to_string();
                    browse(url)
                },
                &landing,
            )
            .await
            .unwrap();
        if matches!(landing, Landing::Served { .. }) {
            let callback =
                reqwest::Url::parse(&stub.asked.lock().clone().unwrap().redirect_uri).unwrap();
            assert_eq!(callback.host_str(), Some("127.0.0.1"));
            assert_eq!(callback.path(), "/v1/oauth/callback");
        }
        account.access().await.unwrap();

        let asked = stub.asked.lock().clone().unwrap();
        assert_eq!(asked.client_id, CLIENT_ID);
        assert!(!opened.lock().contains("return_to"), "{}", opened.lock());
        let exchanges = stub.exchanges.lock().clone();
        assert_eq!(field(&exchanges[0], "client_id"), CLIENT_ID);
        assert_eq!(field(&exchanges[1], "client_id"), CLIENT_ID);
        let written: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let keys: Vec<&String> = written.as_object().unwrap().keys().collect();
        assert_eq!(
            keys,
            ["access_token", "email", "expires_at", "origin", "refresh_token", "token_endpoint"]
        );
    }
}

#[tokio::test]
async fn a_remote_sign_in_is_kept_where_a_loopback_one_is_and_shown_nowhere() {
    let logs = logs();
    let landing = served(SPACE);
    let stub = serve(Script {
        expires_in: Some(1),
        secret_tokens: true,
        served: Some(Served(landing.clone())),
        ..Script::default()
    })
    .await;
    let path = scratch();
    let account = Account::open_at(path.clone(), &stub.origin);
    let opened = Arc::new(Mutex::new(String::new()));
    let seen = opened.clone();
    let status = account
        .sign_in(
            move |url| {
                *seen.lock() = url.to_string();
                browse(url)
            },
            &landing,
        )
        .await
        .unwrap();
    account.connectors().await.unwrap();

    let written = std::fs::read_to_string(&path).unwrap();
    assert!(written.contains(SECRET_ACCESS_2) && written.contains(SECRET_REFRESH_2));
    assert!(written.contains(&format!("\"client_id\": \"{REMOTE_CLIENT_ID}\"")));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    let verifier = field(&stub.exchanges.lock()[0], "code_verifier");
    let shown = [
        serde_json::to_string(&status).unwrap(),
        serde_json::to_string(&account.status()).unwrap(),
        opened.lock().clone(),
        String::from_utf8_lossy(&logs.lock().unwrap()).into_owned(),
    ];
    for (place, text) in ["status", "settings read", "opened page", "log"].iter().zip(&shown) {
        for secret in [SECRET_ACCESS, SECRET_REFRESH, SECRET_ACCESS_2, SECRET_REFRESH_2, &verifier]
        {
            assert!(!text.contains(secret), "{place} carries a secret: {text}");
        }
    }
}

#[tokio::test]
async fn an_answer_for_a_sign_in_this_host_did_not_start_is_not_taken() {
    let landing = served(SPACE);
    let forged = vec![
        ("code".to_string(), "someone-elses-code".to_string()),
        ("state".to_string(), "never-started-here".to_string()),
    ];
    assert_eq!(landing.deliver(forged.clone()).await, None, "nothing is waiting");

    let stub = serve(Script { served: Some(Served(landing.clone())), ..Script::default() }).await;
    let account = Account::open_at(scratch(), &stub.origin);
    let forger = landing.clone();
    let status = account
        .sign_in(
            move |url| {
                let url = url.to_string();
                tokio::spawn(async move {
                    assert_eq!(forger.deliver(forged).await, None, "a stranger's state is unknown");
                    let _ = browse(&url);
                });
                Ok(())
            },
            &landing,
        )
        .await
        .unwrap();
    assert!(status.signed_in);
    let exchanges = stub.exchanges.lock().clone();
    assert_eq!(exchanges.len(), 1);
    assert_eq!(field(&exchanges[0], "code"), "the-code");
}

#[tokio::test]
#[ignore = "reaches the internet"]
async fn the_real_service_still_publishes_where_to_sign_in() {
    let origin = std::env::var("GUACA_ACCOUNT_ORIGIN")
        .unwrap_or_else(|_| guac_lib::account::DEFAULT_ORIGIN.to_string());
    let account = Account::open_at(scratch(), &origin);

    match account.connectors().await {
        Err(AccountError::NotSignedIn) => {}
        other => panic!("expected the local check to refuse, got {other:?}"),
    }

    let http = reqwest::Client::new();
    let url = format!("{}/.well-known/oauth-authorization-server", origin.trim_end_matches('/'));
    let body: serde_json::Value =
        http.get(&url).send().await.expect("metadata unreachable").json().await.expect("not json");

    for field in ["authorization_endpoint", "token_endpoint", "issuer"] {
        let endpoint = body[field].as_str().unwrap_or_default();
        assert!(endpoint.starts_with(origin.trim_end_matches('/')), "{field} is {endpoint}");
    }
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
    assert!(body.get("registration_endpoint").is_none(), "client registration should be off");
}

#[test]
fn the_service_seeds_the_client_the_app_signs_in_as() {
    let seed = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../connectors/google/migrations/0001_oauth_provider.sql"
    ))
    .unwrap();
    let insert = seed.split("INSERT INTO oauthClient (").nth(1).unwrap();
    let (columns, values) = insert.split_once(") VALUES (").unwrap();
    let at = columns.split(',').position(|column| column.trim() == "clientId").unwrap();
    assert_eq!(
        values.split(',').nth(at).unwrap().trim(),
        format!("'{CLIENT_ID}'"),
        "the service's migration seeds the only client that can sign in, and it has to be the one the app asks as"
    );
}

#[test]
fn the_service_trusts_the_client_the_app_signs_in_as() {
    let auth = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../connectors/google/worker/auth.ts"
    ))
    .unwrap();
    let declared = auth
        .lines()
        .find_map(|line| line.trim().strip_prefix("export const DESKTOP_CLIENT_ID = "))
        .expect("auth.ts declares DESKTOP_CLIENT_ID on one line");
    assert_eq!(
        declared,
        format!("\"{CLIENT_ID}\";"),
        "the service holds DESKTOP_CLIENT_ID as its trusted client, and it has to be the client the app asks as"
    );
}

#[test]
fn the_service_names_the_remote_client_and_its_page_as_the_app_does() {
    let auth = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../connectors/google/worker/auth.ts"
    ))
    .unwrap();
    let declared = |name: &str| {
        auth.lines()
            .find_map(|line| line.trim().strip_prefix(&format!("export const {name} = ")))
            .map(str::to_string)
            .unwrap_or_else(|| panic!("auth.ts declares {name} on one line"))
    };
    assert_eq!(declared("REMOTE_CLIENT_ID"), format!("\"{REMOTE_CLIENT_ID}\";"));
    assert_eq!(declared("REMOTE_PATH"), format!("\"{REMOTE_RETURN_PATH}\";"));
}
