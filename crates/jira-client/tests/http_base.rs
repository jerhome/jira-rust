//! Tests du socle HTTP (auth, erreurs) contre wiremock. Aucun accès réseau externe.

use std::time::Duration;

use jira_client::{Error, JiraApi, JiraHttp, SecretString};
use serde_json::json;
use wiremock::matchers::{basic_auth, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn client(server: &MockServer) -> JiraHttp {
    JiraHttp::new(
        &server.uri(),
        "alice@example.com",
        SecretString::new("token-de-test"),
    )
    .unwrap()
}

#[tokio::test]
async fn myself_sends_basic_auth_and_parses_user() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rest/api/3/myself"))
        .and(basic_auth("alice@example.com", "token-de-test"))
        .and(header("accept", "application/json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "accountId": "5b10a2844c20165700ede21g",
            "displayName": "Alice Martin",
            "emailAddress": "alice@example.com",
            "active": true,
            "timeZone": "Europe/Paris"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let me = client(&server).await.myself().await.unwrap();
    assert_eq!(me.display_name, "Alice Martin");
    assert_eq!(me.account_id, "5b10a2844c20165700ede21g");
    assert_eq!(me.email.as_deref(), Some("alice@example.com"));
}

#[tokio::test]
async fn base_url_with_path_prefix_is_respected() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/jira/rest/api/3/myself"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "accountId": "x", "displayName": "X"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let c = JiraHttp::new(
        &format!("{}/jira", server.uri()),
        "a@b.c",
        SecretString::new("t"),
    )
    .unwrap();
    assert_eq!(c.myself().await.unwrap().email, None);
}

#[tokio::test]
async fn unauthorized_maps_to_error_without_leaking_token() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;

    let err = client(&server).await.myself().await.unwrap_err();
    assert!(matches!(err, Error::Unauthorized));
    assert!(!err.to_string().contains("token-de-test"));
    assert!(!format!("{err:?}").contains("token-de-test"));
}

#[tokio::test]
async fn rate_limited_reads_retry_after() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "7"))
        .mount(&server)
        .await;

    let c = client(&server)
        .await
        .with_retry_policy(jira_client::RetryPolicy::none());
    match c.myself().await.unwrap_err() {
        Error::RateLimited { retry_after } => assert_eq!(retry_after, Some(Duration::from_secs(7))),
        e => panic!("erreur inattendue : {e:?}"),
    }
}

#[tokio::test]
async fn jira_error_body_is_summarised() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "errorMessages": ["Requête invalide"],
            "errors": {"summary": "Le résumé est obligatoire"}
        })))
        .mount(&server)
        .await;

    match client(&server).await.myself().await.unwrap_err() {
        Error::Api { status, message } => {
            assert_eq!(status, 400);
            assert!(message.contains("Requête invalide"));
            assert!(message.contains("summary : Le résumé est obligatoire"));
        }
        e => panic!("erreur inattendue : {e:?}"),
    }
}

#[tokio::test]
async fn not_found_and_garbage_body() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(404).set_body_string("<html>nope</html>"))
        .mount(&server)
        .await;

    match client(&server).await.myself().await.unwrap_err() {
        Error::NotFound(m) => assert!(m.contains("nope")),
        e => panic!("erreur inattendue : {e:?}"),
    }
}

#[tokio::test]
async fn undecodable_success_body_is_decode_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_string("pas du json"))
        .mount(&server)
        .await;

    assert!(matches!(
        client(&server).await.myself().await.unwrap_err(),
        Error::Decode(_)
    ));
}

#[test]
fn remote_host_is_blocked_in_tests() {
    // Les tests tournent en build debug et/ou avec JIRA_RUST_OFFLINE=1 : tout hôte non local est refusé.
    let err =
        JiraHttp::new("https://jira.example.com", "a@b.c", SecretString::new("t")).unwrap_err();
    assert!(matches!(err, Error::OfflineGuard { .. }), "{err:?}");
}

#[test]
fn invalid_urls_are_rejected() {
    assert!(matches!(
        JiraHttp::new("pas une url", "a@b.c", SecretString::new("t")),
        Err(Error::InvalidUrl(_))
    ));
    assert!(matches!(
        JiraHttp::new("ftp://localhost", "a@b.c", SecretString::new("t")),
        Err(Error::InvalidUrl(_))
    ));
}
