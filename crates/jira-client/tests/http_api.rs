//! Tests wiremock de chaque méthode de `JiraHttp` (méthode, chemin, corps, mapping, erreurs).
//! Aucun accès réseau externe : uniquement `MockServer` sur loopback.

use std::time::Duration;

use jira_client::model::*;
use jira_client::{Error, JiraApi, JiraHttp, RetryPolicy, SecretString};
use serde_json::{Value, json};
use wiremock::matchers::{body_json, body_partial_json, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

macro_rules! fixture {
    ($p:literal) => {
        serde_json::from_str::<Value>(include_str!(concat!("fixtures/", $p))).unwrap()
    };
}

fn fast_retry() -> RetryPolicy {
    RetryPolicy {
        max_attempts: 4,
        base_delay: Duration::from_millis(5),
        max_delay: Duration::from_secs(2),
    }
}

fn client(server: &MockServer) -> JiraHttp {
    JiraHttp::new(
        &server.uri(),
        "alice@example.com",
        SecretString::new("token-de-test"),
    )
    .unwrap()
    .with_retry_policy(fast_retry())
}

fn ok_json(v: Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(v)
}

fn err_json(status: u16, v: Value) -> ResponseTemplate {
    ResponseTemplate::new(status).set_body_json(v)
}

async fn mount_fields(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/rest/api/3/field"))
        .respond_with(ok_json(fixture!("auth/fields.json")))
        .expect(1)
        .mount(server)
        .await;
}

async fn mount_board_config(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/rest/agile/1.0/board/1/configuration"))
        .respond_with(ok_json(fixture!("projects/board_configuration.json")))
        .mount(server)
        .await;
}

/// Version cible renvoyée par `GET /rest/api/3/version/{id}` ; son `self` est sur le même hôte que
/// le serveur de test mais sous `/rest/api/2/` : le client doit le renvoyer tel quel, sans le
/// reconstruire.
fn target_self(server: &MockServer, id: &str) -> String {
    format!("{}/rest/api/2/version/{id}", server.uri())
}

async fn mount_target_version(server: &MockServer, id: &str) {
    let mut v = fixture!("versions/version_released.json");
    v["id"] = json!(id);
    v["self"] = json!(target_self(server, id));
    v["released"] = json!(false);
    Mock::given(method("GET"))
        .and(path(format!("/rest/api/3/version/{id}")))
        .respond_with(ok_json(v))
        .expect(1)
        .mount(server)
        .await;
}

fn keys(v: &[String]) -> Vec<String> {
    v.to_vec()
}

fn issue_keys(issues: &[Issue]) -> Vec<&str> {
    issues.iter().map(|i| i.key.as_str()).collect()
}

/// Monte les deux pages de `search/jql` (fixtures) pour une JQL donnée.
async fn mount_search_pages(server: &MockServer, jql: &str) {
    Mock::given(method("POST"))
        .and(path("/rest/api/3/search/jql"))
        .and(body_partial_json(
            json!({ "jql": jql, "nextPageToken": "CAEaAggCIgIKAA==" }),
        ))
        .respond_with(ok_json(fixture!("issues/search_jql_last.json")))
        .with_priority(1)
        .expect(1)
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(path("/rest/api/3/search/jql"))
        .and(body_partial_json(json!({
            "jql": jql,
            "maxResults": 100,
            "fields": ["summary", "description", "status", "issuetype", "priority", "assignee",
                       "fixVersions", "customfield_10020", "customfield_10016", "customfield_10028"]
        })))
        .respond_with(ok_json(fixture!("issues/search_jql_page1.json")))
        .with_priority(2)
        .expect(1)
        .mount(server)
        .await;
}

fn assert_fixture_issues(issues: &[Issue]) {
    assert_eq!(issue_keys(issues), vec!["DEMO-1", "DEMO-2", "DEMO-3"]);
    let i1 = &issues[0];
    assert_eq!(i1.id, "10000");
    assert_eq!(i1.summary, "Configurer l'authentification par jeton");
    assert_eq!(i1.status.category, StatusCategory::ToDo);
    assert_eq!(i1.issue_type.name, "Story");
    assert_eq!(i1.priority.as_deref(), Some("Medium"));
    assert_eq!(i1.assignee.as_ref().unwrap().display_name, "Alice Martin");
    assert_eq!(i1.story_points, Some(3.0));
    assert_eq!(i1.sprint_id, Some(2));
    assert_eq!(
        i1.fix_versions,
        vec![VersionRef {
            id: "10100".into(),
            name: "1.0.0".into()
        }]
    );
    assert_eq!(i1.description, None);
    assert!(i1.description_is_lossless);
    let i2 = &issues[1];
    assert_eq!(i2.status.category, StatusCategory::InProgress);
    assert_eq!(i2.sprint_id, Some(2), "sprint actif retenu, pas le clôturé");
    assert_eq!(i2.assignee.as_ref().unwrap().email, None);
    let i3 = &issues[2];
    assert_eq!(i3.status.category, StatusCategory::Done);
    assert_eq!(i3.story_points, None);
    assert_eq!(i3.assignee, None);
}

// --- Navigation ---------------------------------------------------------------------------------

#[tokio::test]
async fn projects_paginates_and_maps() {
    let server = MockServer::start().await;
    let mut page1 = fixture!("projects/project_search.json");
    page1["isLast"] = json!(false);
    page1["total"] = json!(2);
    page1["maxResults"] = json!(1);
    let mut page2 = fixture!("projects/project_search.json");
    page2["startAt"] = json!(1);
    page2["values"][0]["id"] = json!("10010");
    page2["values"][0]["key"] = json!("OPS");
    page2["values"][0]["name"] = json!("Opérations");
    Mock::given(method("GET"))
        .and(path("/rest/api/3/project/search"))
        .and(query_param("typeKey", "software"))
        .and(query_param("startAt", "0"))
        .respond_with(ok_json(page1))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/rest/api/3/project/search"))
        .and(query_param("startAt", "1"))
        .respond_with(ok_json(page2))
        .expect(1)
        .mount(&server)
        .await;

    let projects = client(&server).projects().await.unwrap();
    assert_eq!(
        projects,
        vec![
            Project {
                id: "10000".into(),
                key: "DEMO".into(),
                name: "Demo".into()
            },
            Project {
                id: "10010".into(),
                key: "OPS".into(),
                name: "Opérations".into()
            }
        ]
    );
}

#[tokio::test]
async fn projects_unauthorized() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rest/api/3/project/search"))
        .respond_with(err_json(401, fixture!("errors/401.json")))
        .expect(1)
        .mount(&server)
        .await;
    assert!(matches!(
        client(&server).projects().await,
        Err(Error::Unauthorized)
    ));
}

#[tokio::test]
async fn scrum_boards_filters_by_type_and_project() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rest/agile/1.0/board"))
        .and(query_param("type", "simple"))
        .and(query_param("projectKeyOrId", "DEMO"))
        .respond_with(ok_json(
            json!({"startAt":0,"maxResults":50,"isLast":true,"values":[]}),
        ))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/rest/agile/1.0/board"))
        .and(query_param("type", "scrum"))
        .and(query_param("projectKeyOrId", "DEMO"))
        .and(query_param("startAt", "0"))
        .respond_with(ok_json(fixture!("projects/boards_scrum.json")))
        .expect(1)
        .mount(&server)
        .await;
    let boards = client(&server).scrum_boards("DEMO").await.unwrap();
    assert_eq!(
        boards,
        vec![Board {
            id: 1,
            name: "DEMO board".into(),
            project_key: Some("DEMO".into()),
            board_type: Some("scrum".into()),
        }]
    );
}

#[tokio::test]
async fn scrum_boards_forbidden() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rest/agile/1.0/board"))
        .respond_with(err_json(403, fixture!("errors/403.json")))
        .mount(&server)
        .await;
    match client(&server).scrum_boards("DEMO").await {
        Err(Error::Forbidden(m)) => assert!(m.contains("manage sprints")),
        r => panic!("{r:?}"),
    }
}

// --- Sprints ------------------------------------------------------------------------------------

#[tokio::test]
async fn sprints_by_state() {
    let server = MockServer::start().await;
    mount_board_config(&server).await;
    Mock::given(method("GET"))
        .and(path("/rest/agile/1.0/board/1/sprint"))
        .and(query_param("state", "active,future"))
        .and(query_param("startAt", "0"))
        .respond_with(ok_json(fixture!(
            "sprints/board_sprints_active_future.json"
        )))
        .expect(1)
        .mount(&server)
        .await;
    let sprints = client(&server)
        .sprints(1, &[SprintState::Active, SprintState::Future])
        .await
        .unwrap();
    assert_eq!(sprints.len(), 2);
    assert_eq!(
        sprints[0],
        Sprint {
            id: 2,
            board_id: Some(1),
            name: "DEMO Sprint 2".into(),
            state: SprintState::Active,
            goal: Some("Livrer la connexion".into()),
            start_date: Some("2026-11-02T08:00:00.000Z".into()),
            end_date: Some("2026-11-16T17:00:00.000Z".into()),
        }
    );
    assert_eq!(sprints[1].state, SprintState::Future);
    assert_eq!(sprints[1].goal, None, "objectif vide -> None");
    assert_eq!(sprints[1].start_date, None);
}

#[tokio::test]
async fn sprints_paginates_with_is_last_and_board_not_found() {
    let server = MockServer::start().await;
    // Configuration illisible : ignorée pour `sprints`.
    Mock::given(method("GET"))
        .and(path("/rest/agile/1.0/board/1/configuration"))
        .respond_with(ResponseTemplate::new(403))
        .mount(&server)
        .await;
    let mut p1 = fixture!("sprints/board_sprints_active_future.json");
    p1["isLast"] = json!(false);
    p1["values"].as_array_mut().unwrap().pop();
    let mut p2 = fixture!("sprints/board_sprints_active_future.json");
    p2["startAt"] = json!(1);
    p2["values"].as_array_mut().unwrap().remove(0);
    Mock::given(method("GET"))
        .and(path("/rest/agile/1.0/board/1/sprint"))
        .and(query_param("startAt", "0"))
        .respond_with(ok_json(p1))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/rest/agile/1.0/board/1/sprint"))
        .and(query_param("startAt", "1"))
        .respond_with(ok_json(p2))
        .mount(&server)
        .await;
    let c = client(&server);
    let ids: Vec<u64> = c
        .sprints(1, &[])
        .await
        .unwrap()
        .iter()
        .map(|s| s.id)
        .collect();
    assert_eq!(ids, vec![2, 3]);

    Mock::given(method("GET"))
        .and(path("/rest/agile/1.0/board/9/sprint"))
        .respond_with(err_json(404, fixture!("errors/404.json")))
        .mount(&server)
        .await;
    assert!(matches!(c.sprints(9, &[]).await, Err(Error::NotFound(_))));
}

#[tokio::test]
async fn start_sprint_posts_partial_update() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/rest/agile/1.0/sprint/3"))
        .and(body_json(fixture!("sprints/sprint_start_request.json")))
        .respond_with(ok_json(fixture!("sprints/sprint_start_response.json")))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&server)
        .await;
    let s = client(&server)
        .start_sprint(
            3,
            &StartSprint {
                start_date: "2026-11-16T08:00:00.000Z".into(),
                end_date: "2026-11-30T17:00:00.000Z".into(),
                goal: Some("Gestion des versions".into()),
            },
        )
        .await
        .unwrap();
    assert_eq!(s.state, SprintState::Active);
    assert_eq!(s.goal.as_deref(), Some("Gestion des versions"));
}

#[tokio::test]
async fn start_sprint_rejected() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/rest/agile/1.0/sprint/3"))
        .respond_with(err_json(
            400,
            json!({"errorMessages": ["Sprint cannot be started"], "errors": {}}),
        ))
        .mount(&server)
        .await;
    let r = client(&server)
        .start_sprint(3, &StartSprint::default())
        .await;
    assert!(
        matches!(r, Err(Error::Api { status: 400, ref message }) if message.contains("cannot be started"))
    );
}

#[tokio::test]
async fn close_sprint_only_closes() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/rest/agile/1.0/sprint/2"))
        .and(body_json(fixture!("sprints/sprint_close_request.json")))
        .respond_with(ok_json(fixture!("sprints/sprint_close_response.json")))
        .expect(1)
        .mount(&server)
        .await;
    // Aucune autre requête (pas de déplacement d'issues, jamais de PUT).
    Mock::given(wiremock::matchers::any())
        .respond_with(ResponseTemplate::new(500))
        .with_priority(10)
        .expect(0)
        .mount(&server)
        .await;
    let s = client(&server).close_sprint(2).await.unwrap();
    assert_eq!(s.state, SprintState::Closed);
}

#[tokio::test]
async fn close_sprint_forbidden() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/rest/agile/1.0/sprint/2"))
        .respond_with(err_json(403, fixture!("errors/403.json")))
        .mount(&server)
        .await;
    assert!(matches!(
        client(&server).close_sprint(2).await,
        Err(Error::Forbidden(_))
    ));
}

#[tokio::test]
async fn create_sprint_posts_origin_board() {
    let server = MockServer::start().await;
    let mut created = fixture!("sprints/sprint_start_response.json");
    created["id"] = json!(4);
    created["state"] = json!("future");
    created["name"] = json!("DEMO Sprint 4");
    created["goal"] = json!("Objectif");
    created.as_object_mut().unwrap().remove("startDate");
    created.as_object_mut().unwrap().remove("endDate");
    Mock::given(method("POST"))
        .and(path("/rest/agile/1.0/sprint"))
        .and(body_json(
            json!({"name": "DEMO Sprint 4", "originBoardId": 1, "goal": "Objectif"}),
        ))
        .respond_with(ResponseTemplate::new(201).set_body_json(created))
        .expect(1)
        .mount(&server)
        .await;
    let c = client(&server);
    let s = c
        .create_sprint(1, "DEMO Sprint 4", Some("Objectif".into()))
        .await
        .unwrap();
    assert_eq!(
        (s.id, s.state, s.board_id),
        (4, SprintState::Future, Some(1))
    );
    // Nom vide : refusé sans requête.
    assert!(matches!(
        c.create_sprint(1, "  ", None).await,
        Err(Error::Invalid(_))
    ));
}

#[tokio::test]
async fn update_sprint_sends_only_changed_fields() {
    let server = MockServer::start().await;
    let mut resp = fixture!("sprints/board_sprints_active_future.json")["values"][1].clone();
    resp["name"] = json!("Renommé");
    Mock::given(method("POST"))
        .and(path("/rest/agile/1.0/sprint/3"))
        .and(body_json(json!({
            "name": "Renommé",
            "goal": "",
            "endDate": "2026-11-30T17:00:00.000Z"
        })))
        .respond_with(ok_json(resp))
        .expect(1)
        .mount(&server)
        .await;
    let s = client(&server)
        .update_sprint(
            3,
            &SprintUpdate {
                name: Some("Renommé".into()),
                goal: Some(None),
                end_date: Some("2026-11-30T17:00:00.000Z".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(s.name, "Renommé");
    assert_eq!(s.state, SprintState::Future);
}

#[tokio::test]
async fn update_sprint_not_found() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/rest/agile/1.0/sprint/99"))
        .respond_with(err_json(404, fixture!("errors/404.json")))
        .mount(&server)
        .await;
    let r = client(&server)
        .update_sprint(
            99,
            &SprintUpdate {
                name: Some("x".into()),
                ..Default::default()
            },
        )
        .await;
    assert!(matches!(r, Err(Error::NotFound(_))));
}

// --- Lectures d'issues (search/jql) ------------------------------------------------------------

#[tokio::test]
async fn sprint_issues_uses_search_jql_with_explicit_fields() {
    let server = MockServer::start().await;
    mount_fields(&server).await;
    mount_search_pages(&server, "sprint = 2 ORDER BY Rank ASC").await;

    let c = client(&server);
    let issues = c.sprint_issues(2).await.unwrap();
    assert_fixture_issues(&issues);
}

#[tokio::test]
async fn sprint_issues_invalid_jql() {
    let server = MockServer::start().await;
    mount_fields(&server).await;
    Mock::given(method("POST"))
        .and(path("/rest/api/3/search/jql"))
        .respond_with(err_json(400, fixture!("errors/400_jql.json")))
        .expect(1)
        .mount(&server)
        .await;
    match client(&server).sprint_issues(2).await {
        Err(Error::Api {
            status: 400,
            message,
        }) => assert!(message.contains("sprnt")),
        r => panic!("{r:?}"),
    }
}

#[tokio::test]
async fn backlog_issues_uses_board_filter() {
    let server = MockServer::start().await;
    mount_board_config(&server).await;
    mount_fields(&server).await;
    mount_search_pages(
        &server,
        "filter = 10001 AND (sprint is EMPTY OR (sprint not in openSprints() \
         AND sprint not in futureSprints())) AND statusCategory != Done \
         AND issuetype not in subtaskIssueTypes() ORDER BY Rank ASC",
    )
    .await;
    let issues = client(&server).backlog_issues(1).await.unwrap();
    assert_fixture_issues(&issues);
}

#[tokio::test]
async fn backlog_board_estimation_field_wins() {
    let server = MockServer::start().await;
    // Board estimé sur le champ legacy « Story Points » (customfield_10028).
    let mut cfg = fixture!("projects/board_configuration.json");
    cfg["estimation"]["field"]["fieldId"] = json!("customfield_10028");
    Mock::given(method("GET"))
        .and(path("/rest/agile/1.0/board/1/configuration"))
        .respond_with(ok_json(cfg))
        .mount(&server)
        .await;
    mount_fields(&server).await;
    let mut page = fixture!("issues/search_jql_last.json");
    page["issues"][0]["fields"]["customfield_10028"] = json!(8.0);
    page["issues"][0]["fields"]["customfield_10016"] = json!(2.0);
    Mock::given(method("POST"))
        .and(path("/rest/api/3/search/jql"))
        .and(body_partial_json(json!({
            "fields": ["summary", "description", "status", "issuetype", "priority", "assignee",
                       "fixVersions", "customfield_10020", "customfield_10028", "customfield_10016"]
        })))
        .respond_with(ok_json(page))
        .expect(1)
        .mount(&server)
        .await;
    let c = client(&server);
    let issues = c.backlog_issues(1).await.unwrap();
    assert_eq!(issues[0].story_points, Some(8.0));

    // Les écritures utilisent aussi le champ du board.
    Mock::given(method("PUT"))
        .and(path("/rest/api/3/issue/DEMO-3"))
        .and(body_json(json!({ "fields": { "customfield_10028": 5.0 } })))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    c.update_issue(
        "DEMO-3",
        &IssueUpdate {
            story_points: Some(Some(5.0)),
            ..Default::default()
        },
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn backlog_board_not_found() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rest/agile/1.0/board/7/configuration"))
        .respond_with(err_json(404, fixture!("errors/404.json")))
        .mount(&server)
        .await;
    assert!(matches!(
        client(&server).backlog_issues(7).await,
        Err(Error::NotFound(_))
    ));
}

#[tokio::test]
async fn description_adf_is_converted() {
    let server = MockServer::start().await;
    mount_fields(&server).await;
    let mut page = fixture!("issues/search_jql_last.json");
    page["issues"][0]["fields"]["description"] = fixture!("issues/adf_paragraph.json");
    let mut rich = page["issues"][0].clone();
    rich["key"] = json!("DEMO-4");
    rich["fields"]["description"] = json!({"type": "doc", "version": 1, "content": [
        {"type": "bulletList", "content": [{"type": "listItem", "content": [
            {"type": "paragraph", "content": [{"type": "text", "text": "point"}]}]}]}]});
    page["issues"].as_array_mut().unwrap().push(rich);
    Mock::given(method("POST"))
        .and(path("/rest/api/3/search/jql"))
        .respond_with(ok_json(page))
        .mount(&server)
        .await;
    let issues = client(&server).sprint_issues(2).await.unwrap();
    assert_eq!(
        issues[0].description.as_deref(),
        Some(
            "En tant qu'utilisateur, je veux me connecter avec un jeton API.\n\n\
             Critere : le jeton est stocke hors du depot."
        )
    );
    assert!(issues[0].description_is_lossless);
    assert_eq!(issues[1].description.as_deref(), Some("- point"));
    assert!(!issues[1].description_is_lossless);
}

#[tokio::test]
async fn expired_page_token_restarts_once() {
    let server = MockServer::start().await;
    mount_fields(&server).await;
    Mock::given(method("POST"))
        .and(path("/rest/api/3/search/jql"))
        .and(body_partial_json(
            json!({ "nextPageToken": "CAEaAggCIgIKAA==" }),
        ))
        .respond_with(err_json(
            400,
            json!({"errorMessages": ["invalid or expired nextPageToken"]}),
        ))
        .with_priority(1)
        .expect(2)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/rest/api/3/search/jql"))
        .respond_with(ok_json(fixture!("issues/search_jql_page1.json")))
        .with_priority(2)
        .expect(2)
        .mount(&server)
        .await;
    // Une seule reprise : la seconde erreur est remontée.
    assert!(matches!(
        client(&server).sprint_issues(2).await,
        Err(Error::Api { status: 400, .. })
    ));
}

#[tokio::test]
async fn version_issues_jql() {
    let server = MockServer::start().await;
    mount_fields(&server).await;
    mount_search_pages(
        &server,
        "project = \"DEMO\" AND fixVersion = 10100 ORDER BY Rank ASC",
    )
    .await;
    let c = client(&server);
    assert_fixture_issues(&c.version_issues("DEMO", "10100").await.unwrap());
    // Id non numérique : refusé avant tout appel (pas d'injection JQL).
    assert!(matches!(
        c.version_issues("DEMO", "1 OR project = X").await,
        Err(Error::Invalid(_))
    ));
}

// --- Écritures agile ----------------------------------------------------------------------------

#[tokio::test]
async fn move_to_sprint_in_batches_of_50() {
    let server = MockServer::start().await;
    let all: Vec<String> = (1..=120).map(|n| format!("DEMO-{n}")).collect();
    for chunk in all.chunks(50) {
        Mock::given(method("POST"))
            .and(path("/rest/agile/1.0/sprint/3/issue"))
            .and(body_json(json!({ "issues": chunk })))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;
    }
    client(&server).move_to_sprint(3, &all).await.unwrap();
}

#[tokio::test]
async fn move_to_sprint_closed_sprint() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/rest/agile/1.0/sprint/1/issue"))
        .respond_with(err_json(
            400,
            json!({"errorMessages": ["Cannot move issues to a closed sprint"]}),
        ))
        .expect(1)
        .mount(&server)
        .await;
    let r = client(&server)
        .move_to_sprint(1, &keys(&["DEMO-4".into()]))
        .await;
    assert!(matches!(r, Err(Error::Api { status: 400, .. })));
}

#[tokio::test]
async fn move_to_backlog_body() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/rest/agile/1.0/backlog/issue"))
        .and(body_json(fixture!("backlog/move_to_backlog_request.json")))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    client(&server)
        .move_to_backlog(&["DEMO-2".into(), "DEMO-3".into()])
        .await
        .unwrap();
}

#[tokio::test]
async fn move_to_backlog_forbidden_is_not_retried() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/rest/agile/1.0/backlog/issue"))
        .respond_with(err_json(403, fixture!("errors/403.json")))
        .expect(1)
        .mount(&server)
        .await;
    assert!(matches!(
        client(&server).move_to_backlog(&["DEMO-2".into()]).await,
        Err(Error::Forbidden(_))
    ));
}

#[tokio::test]
async fn rank_before() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/rest/agile/1.0/issue/rank"))
        .and(body_json(
            json!({ "issues": ["DEMO-5"], "rankBeforeIssue": "DEMO-4" }),
        ))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    client(&server)
        .rank(&["DEMO-5".into()], &RankPosition::Before("DEMO-4".into()))
        .await
        .unwrap();
}

#[tokio::test]
async fn rank_batches_keep_order() {
    let server = MockServer::start().await;
    let all: Vec<String> = (10..70).map(|n| format!("DEMO-{n}")).collect();
    Mock::given(method("PUT"))
        .and(path("/rest/agile/1.0/issue/rank"))
        .and(body_json(
            json!({ "issues": all[..50], "rankBeforeIssue": "DEMO-1" }),
        ))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/rest/agile/1.0/issue/rank"))
        .and(body_json(
            json!({ "issues": all[50..], "rankAfterIssue": "DEMO-59" }),
        ))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    client(&server)
        .rank(&all, &RankPosition::Before("DEMO-1".into()))
        .await
        .unwrap();
}

#[tokio::test]
async fn rank_partial_207_lists_failed_keys() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/rest/agile/1.0/issue/rank"))
        .and(body_json(
            json!({ "issues": ["DEMO-5", "DEMO-6"], "rankAfterIssue": "DEMO-4" }),
        ))
        .respond_with(
            ResponseTemplate::new(207).set_body_json(fixture!("backlog/rank_partial_207.json")),
        )
        .expect(1)
        .mount(&server)
        .await;
    let c = client(&server);
    match c
        .rank(
            &["DEMO-5".into(), "DEMO-6".into()],
            &RankPosition::After("DEMO-4".into()),
        )
        .await
    {
        Err(Error::Api {
            status: 207,
            message,
        }) => {
            assert!(message.contains("DEMO-6"), "{message}");
            assert!(message.contains("permission"), "{message}");
            assert!(!message.contains("DEMO-5"), "{message}");
        }
        r => panic!("{r:?}"),
    }
    // L'ancre ne peut pas faire partie des issues déplacées (aucune requête).
    assert!(matches!(
        c.rank(&["DEMO-4".into()], &RankPosition::After("DEMO-4".into()))
            .await,
        Err(Error::Invalid(_))
    ));
}

// --- Issues -------------------------------------------------------------------------------------

#[tokio::test]
async fn issue_types_createmeta() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rest/api/3/issue/createmeta/DEMO/issuetypes"))
        .and(query_param("startAt", "0"))
        .respond_with(ok_json(fixture!("issues/createmeta_issuetypes.json")))
        .expect(1)
        .mount(&server)
        .await;
    let types = client(&server).issue_types("DEMO").await.unwrap();
    assert_eq!(
        types.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
        vec!["Epic", "Story", "Bug", "Task"]
    );
    assert_eq!(types[1].id, "10001");
    assert!(!types[1].subtask);
}

#[tokio::test]
async fn issue_types_bad_request() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rest/api/3/issue/createmeta/NOPE/issuetypes"))
        .respond_with(err_json(400, json!({"errorMessages": ["projet invalide"]})))
        .mount(&server)
        .await;
    assert!(matches!(
        client(&server).issue_types("NOPE").await,
        Err(Error::Api { status: 400, .. })
    ));
}

#[tokio::test]
async fn create_issue_body_and_key() {
    let server = MockServer::start().await;
    mount_fields(&server).await;
    let mut expected = fixture!("issues/create_request.json");
    let f = expected["fields"].as_object_mut().unwrap();
    // Hors contrat : assignee, priorité, sprint.
    f.remove("assignee");
    f.remove("priority");
    f.remove("customfield_10020");
    f.insert("customfield_10016".into(), json!(3.0));
    Mock::given(method("POST"))
        .and(path("/rest/api/3/issue"))
        .and(body_json(expected))
        .respond_with(
            ResponseTemplate::new(201).set_body_json(fixture!("issues/create_response.json")),
        )
        .expect(1)
        .mount(&server)
        .await;
    let key = client(&server)
        .create_issue(&NewIssue {
            project_key: "DEMO".into(),
            issue_type_id: "10001".into(),
            summary: "Afficher le sprint actif dans la TUI".into(),
            description: Some("Vue liste des tickets du sprint actif, triee par rang.".into()),
            story_points: Some(3.0),
            fix_version_ids: vec!["10100".into()],
        })
        .await
        .unwrap();
    assert_eq!(key, "DEMO-11");
}

#[tokio::test]
async fn create_issue_field_errors() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/rest/api/3/issue"))
        .and(body_json(json!({"fields": {
            "project": {"key": "DEMO"}, "issuetype": {"id": "10001"}, "summary": "x"
        }})))
        .respond_with(err_json(400, fixture!("errors/400_field_errors.json")))
        .expect(1)
        .mount(&server)
        .await;
    match client(&server)
        .create_issue(&NewIssue {
            project_key: "DEMO".into(),
            issue_type_id: "10001".into(),
            summary: "x".into(),
            description: Some("  \n".into()),
            ..Default::default()
        })
        .await
    {
        Err(Error::Api {
            status: 400,
            message,
        }) => {
            assert!(message.contains("customfield_10016 : Field"), "{message}")
        }
        r => panic!("{r:?}"),
    }
}

#[tokio::test]
async fn create_issue_without_points_field() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rest/api/3/field"))
        .respond_with(ok_json(json!([])))
        .mount(&server)
        .await;
    let r = client(&server)
        .create_issue(&NewIssue {
            project_key: "DEMO".into(),
            issue_type_id: "10001".into(),
            summary: "x".into(),
            story_points: Some(1.0),
            ..Default::default()
        })
        .await;
    assert!(matches!(r, Err(Error::Invalid(_))));
}

#[tokio::test]
async fn update_issue_partial_body() {
    let server = MockServer::start().await;
    mount_fields(&server).await;
    Mock::given(method("PUT"))
        .and(path("/rest/api/3/issue/DEMO-1"))
        .and(body_json(json!({
            "fields": {
                "summary": "Afficher le sprint actif (avec points)",
                "description": null,
                "customfield_10016": null
            },
            "update": { "fixVersions": [ { "set": [ { "id": "10100" } ] } ] }
        })))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    let c = client(&server);
    c.update_issue(
        "DEMO-1",
        &IssueUpdate {
            summary: Some("Afficher le sprint actif (avec points)".into()),
            description: Some(None),
            story_points: Some(None),
            fix_version_ids: Some(vec!["10100".into()]),
        },
    )
    .await
    .unwrap();
    // Mise à jour vide : aucune requête.
    c.update_issue("DEMO-1", &IssueUpdate::default())
        .await
        .unwrap();
}

#[tokio::test]
async fn update_issue_description_as_adf_and_not_found() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/rest/api/3/issue/DEMO-404"))
        .and(body_json(json!({
            "fields": { "description": fixture!("issues/adf_paragraph.json") }
        })))
        .respond_with(err_json(404, fixture!("errors/404.json")))
        .expect(1)
        .mount(&server)
        .await;
    let r = client(&server)
        .update_issue(
            "DEMO-404",
            &IssueUpdate {
                description: Some(Some(
                    "En tant qu'utilisateur, je veux me connecter avec un jeton API.\n\n\
                     Critere : le jeton est stocke hors du depot."
                        .into(),
                )),
                ..Default::default()
            },
        )
        .await;
    match r {
        Err(Error::NotFound(m)) => assert!(m.contains("does not exist")),
        r => panic!("{r:?}"),
    }
}

#[tokio::test]
async fn transitions_with_screen_flag() {
    let server = MockServer::start().await;
    let mut body = fixture!("issues/transitions.json");
    // « In Progress » sans écran mais avec un champ requis -> has_screen.
    body["transitions"][1]["fields"] = json!({
        "resolution": { "required": true, "name": "Resolution" }
    });
    Mock::given(method("GET"))
        .and(path("/rest/api/3/issue/DEMO-1/transitions"))
        .and(query_param("expand", "transitions.fields"))
        .respond_with(ok_json(body))
        .expect(1)
        .mount(&server)
        .await;
    let tr = client(&server).transitions("DEMO-1").await.unwrap();
    assert_eq!(tr.len(), 3);
    assert_eq!(tr[0].id, "11");
    assert!(!tr[0].has_screen);
    assert!(tr[1].has_screen);
    assert_eq!(tr[2].to.category, StatusCategory::Done);
    assert!(tr[2].has_screen);
}

#[tokio::test]
async fn transitions_not_found() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rest/api/3/issue/DEMO-9/transitions"))
        .respond_with(err_json(404, fixture!("errors/404.json")))
        .mount(&server)
        .await;
    assert!(matches!(
        client(&server).transitions("DEMO-9").await,
        Err(Error::NotFound(_))
    ));
}

#[tokio::test]
async fn transition_issue_body() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/rest/api/3/issue/DEMO-1/transitions"))
        .and(body_json(fixture!("issues/transition_request.json")))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    client(&server)
        .transition_issue("DEMO-1", "21")
        .await
        .unwrap();
}

#[tokio::test]
async fn transition_issue_requires_screen() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/rest/api/3/issue/DEMO-1/transitions"))
        .respond_with(err_json(
            400,
            json!({"errorMessages": [], "errors": {"resolution": "Resolution is required."}}),
        ))
        .expect(1)
        .mount(&server)
        .await;
    match client(&server).transition_issue("DEMO-1", "31").await {
        Err(Error::Api {
            status: 400,
            message,
        }) => assert!(message.contains("resolution")),
        r => panic!("{r:?}"),
    }
}

// --- Versions -----------------------------------------------------------------------------------

#[tokio::test]
async fn versions_paginates() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rest/api/3/project/DEMO/version"))
        .and(query_param("startAt", "0"))
        .respond_with(ok_json(fixture!("versions/project_versions_page.json")))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/rest/api/3/project/DEMO/version"))
        .and(query_param("startAt", "2"))
        .respond_with(ok_json(fixture!("versions/project_versions_last.json")))
        .expect(1)
        .mount(&server)
        .await;
    let v = client(&server).versions("DEMO").await.unwrap();
    assert_eq!(
        v.iter().map(|v| v.name.as_str()).collect::<Vec<_>>(),
        vec!["1.1.0", "1.0.0", "0.9.0"]
    );
    assert_eq!(
        v[0],
        Version {
            id: "10101".into(),
            project_id: "10000".into(),
            name: "1.1.0".into(),
            description: None,
            released: false,
            archived: false,
            start_date: Some("2026-12-02".into()),
            release_date: Some("2027-01-15".into()),
        }
    );
    assert!(v[2].released);
}

#[tokio::test]
async fn versions_project_not_found() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rest/api/3/project/NOPE/version"))
        .respond_with(err_json(
            404,
            json!({"errorMessages": ["No project could be found"]}),
        ))
        .mount(&server)
        .await;
    assert!(matches!(
        client(&server).versions("NOPE").await,
        Err(Error::NotFound(_))
    ));
}

#[tokio::test]
async fn create_version_body() {
    let server = MockServer::start().await;
    let mut expected = fixture!("versions/version_create_request.json");
    let o = expected.as_object_mut().unwrap();
    o.remove("released");
    o.remove("archived");
    Mock::given(method("POST"))
        .and(path("/rest/api/3/version"))
        .and(body_json(expected))
        .respond_with(
            ResponseTemplate::new(201).set_body_json(fixture!("versions/version_created.json")),
        )
        .expect(1)
        .mount(&server)
        .await;
    let v = client(&server)
        .create_version(&NewVersion {
            project_id: "10000".into(),
            name: "1.2.0".into(),
            description: Some("Gestion des versions dans la TUI".into()),
            start_date: Some("2027-01-16".into()),
            release_date: Some("2027-02-28".into()),
        })
        .await
        .unwrap();
    assert_eq!(v.id, "10102");
    assert_eq!(v.project_id, "10000");
}

#[tokio::test]
async fn create_version_duplicate_and_bad_project_id() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/rest/api/3/version"))
        .respond_with(err_json(
            400,
            json!({"errors": {"name": "A version with this name already exists in this project."}}),
        ))
        .expect(1)
        .mount(&server)
        .await;
    let c = client(&server);
    let nv = NewVersion {
        project_id: "10000".into(),
        name: "1.0.0".into(),
        ..Default::default()
    };
    assert!(matches!(
        c.create_version(&nv).await,
        Err(Error::Api { status: 400, ref message }) if message.starts_with("name : ")
    ));
    let bad = NewVersion {
        project_id: "DEMO".into(),
        ..nv
    };
    assert!(matches!(
        c.create_version(&bad).await,
        Err(Error::Invalid(_))
    ));
}

#[tokio::test]
async fn update_version_release_with_move() {
    let server = MockServer::start().await;
    let mut expected = fixture!("versions/version_release_request.json");
    expected["moveUnfixedIssuesTo"] = json!(target_self(&server, "10101"));
    expected["archived"] = json!(false);
    mount_target_version(&server, "10101").await;
    Mock::given(method("PUT"))
        .and(path("/rest/api/3/version/10100"))
        .and(body_json(expected))
        .respond_with(ok_json(fixture!("versions/version_released.json")))
        .expect(1)
        .mount(&server)
        .await;
    let v = client(&server)
        .update_version(
            "10100",
            &VersionUpdate {
                released: Some(true),
                release_date: Some(Some("2026-12-01".into())),
                archived: Some(false),
                move_unfixed_issues_to: Some("10101".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(v.released);
    assert_eq!(v.release_date.as_deref(), Some("2026-12-01"));
}

#[tokio::test]
async fn update_version_clears_dates_and_errors() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/rest/api/3/version/10100"))
        .and(body_json(json!({"startDate": null, "description": ""})))
        .respond_with(err_json(404, fixture!("errors/404.json")))
        .expect(1)
        .mount(&server)
        .await;
    let c = client(&server);
    let r = c
        .update_version(
            "10100",
            &VersionUpdate {
                start_date: Some(None),
                description: Some(None),
                ..Default::default()
            },
        )
        .await;
    assert!(matches!(r, Err(Error::NotFound(_))));
    // Cible identique ou non numérique : refusée sans requête.
    for target in ["10100", "../x"] {
        let r = c
            .update_version(
                "10100",
                &VersionUpdate {
                    move_unfixed_issues_to: Some(target.into()),
                    ..Default::default()
                },
            )
            .await;
        assert!(matches!(r, Err(Error::Invalid(_))), "{target}");
    }
}

#[tokio::test]
async fn delete_version_remove_and_swap() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/rest/api/3/version/10100/removeAndSwap"))
        .and(body_json(json!({ "moveFixIssuesTo": 10101 })))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/rest/api/3/version/10099/removeAndSwap"))
        .and(body_json(json!({})))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&server)
        .await;
    let c = client(&server);
    c.delete_version("10100", Some("10101".into()))
        .await
        .unwrap();
    c.delete_version("10099", None).await.unwrap();
}

#[tokio::test]
async fn delete_version_not_found() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/rest/api/3/version/1/removeAndSwap"))
        .respond_with(err_json(404, fixture!("errors/404.json")))
        .mount(&server)
        .await;
    assert!(matches!(
        client(&server).delete_version("1", None).await,
        Err(Error::NotFound(_))
    ));
}

// --- 429 ----------------------------------------------------------------------------------------

#[tokio::test]
async fn get_retries_after_429() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rest/api/3/myself"))
        .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "0"))
        .up_to_n_times(2)
        .with_priority(1)
        .expect(2)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/rest/api/3/myself"))
        .respond_with(ok_json(fixture!("auth/myself.json")))
        .with_priority(2)
        .expect(1)
        .mount(&server)
        .await;
    let me = client(&server).myself().await.unwrap();
    assert_eq!(me.display_name, "Alice Martin");
}

#[tokio::test]
async fn retries_are_bounded_to_four_attempts() {
    let server = MockServer::start().await;
    // Sans Retry-After : backoff exponentiel (rapide dans ce test).
    Mock::given(method("POST"))
        .and(path("/rest/api/3/search/jql"))
        .respond_with(ResponseTemplate::new(429))
        .expect(4)
        .mount(&server)
        .await;
    mount_fields(&server).await;
    assert!(matches!(
        client(&server).sprint_issues(2).await,
        Err(Error::RateLimited { retry_after: None })
    ));
}

#[tokio::test]
async fn http_date_retry_after_is_honoured() {
    let server = MockServer::start().await;
    // Date passée : délai nul, puis succès.
    Mock::given(method("GET"))
        .and(path("/rest/api/3/myself"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("Retry-After", "Sun, 06 Nov 1994 08:49:37 GMT"),
        )
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/rest/api/3/myself"))
        .respond_with(ok_json(fixture!("auth/myself.json")))
        .with_priority(2)
        .mount(&server)
        .await;
    assert!(client(&server).myself().await.is_ok());
}

#[tokio::test]
async fn long_retry_after_fails_fast() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "3600"))
        .expect(1)
        .mount(&server)
        .await;
    match client(&server).myself().await {
        Err(Error::RateLimited { retry_after }) => {
            assert_eq!(retry_after, Some(Duration::from_secs(3600)))
        }
        r => panic!("{r:?}"),
    }
}

#[tokio::test]
async fn writes_are_never_retried() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/rest/api/3/issue/DEMO-1/transitions"))
        .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "0"))
        .expect(1)
        .mount(&server)
        .await;
    assert!(matches!(
        client(&server).transition_issue("DEMO-1", "21").await,
        Err(Error::RateLimited { .. })
    ));
}

// --- Sécurité -----------------------------------------------------------------------------------

#[tokio::test]
async fn redirects_are_not_followed() {
    let server = MockServer::start().await;
    let elsewhere = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(302)
                .insert_header("Location", format!("{}/rest/api/3/myself", elsewhere.uri())),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(wiremock::matchers::any())
        .respond_with(ok_json(fixture!("auth/myself.json")))
        .expect(0)
        .mount(&elsewhere)
        .await;
    let err = client(&server).myself().await.unwrap_err();
    assert!(matches!(err, Error::Api { status: 302, .. }), "{err:?}");
    assert!(!err.to_string().contains("token-de-test"));
}

#[test]
fn plain_http_refused_outside_loopback() {
    let err =
        JiraHttp::new("http://jira.example.test", "a@b.c", SecretString::new("t")).unwrap_err();
    assert!(matches!(err, Error::InvalidUrl(_)), "{err:?}");
    assert!(JiraHttp::new("http://127.0.0.1:9", "a@b.c", SecretString::new("t")).is_ok());
    assert!(JiraHttp::new("http://[::1]:9", "a@b.c", SecretString::new("t")).is_ok());
    assert!(matches!(
        JiraHttp::new("https://user:pw@localhost", "a@b.c", SecretString::new("t")),
        Err(Error::InvalidUrl(_))
    ));
}

#[tokio::test]
async fn dynamic_segments_are_encoded() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/rest/api/3/issue/DEMO-1%2F..%2F..%2Fmyself%3Fx%23y/transitions",
        ))
        .respond_with(ok_json(json!({ "transitions": [] })))
        .expect(1)
        .mount(&server)
        .await;
    let c = client(&server);
    assert!(
        c.transitions("DEMO-1/../../myself?x#y")
            .await
            .unwrap()
            .is_empty()
    );
    // Segments « . » / « .. » / vides refusés sans requête.
    for key in ["..", ".", ""] {
        assert!(
            matches!(c.transitions(key).await, Err(Error::Invalid(_))),
            "{key:?}"
        );
    }
}

#[tokio::test]
async fn html_error_body_is_not_echoed() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(503).set_body_raw(
            "<html><body>\n  Maintenance   en cours\n</body></html>",
            "text/html",
        ))
        .mount(&server)
        .await;
    match client(&server).myself().await {
        Err(Error::Api {
            status: 503,
            message,
        }) => assert_eq!(message, "Service Unavailable"),
        r => panic!("{r:?}"),
    }
}

// --- Fixtures « phase 2b » (agent jira-docs) ----------------------------------------------------

/// Monte une réponse décrite par une enveloppe de test `{status, headers, body}`.
fn envelope(v: Value) -> ResponseTemplate {
    let mut t = ResponseTemplate::new(v["status"].as_u64().unwrap() as u16);
    let mut content_type = "application/json".to_owned();
    for (k, val) in v["headers"].as_object().unwrap() {
        if k.eq_ignore_ascii_case("content-type") {
            content_type = val.as_str().unwrap().to_owned();
        } else {
            t = t.insert_header(k.as_str(), val.as_str().unwrap());
        }
    }
    // `set_body_string` imposerait text/plain : on garde le Content-Type de l'enveloppe.
    t.set_body_raw(v["body"].as_str().unwrap(), &content_type)
}

#[tokio::test]
async fn p2b_create_sprint_minimal_and_board_404() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/rest/agile/1.0/sprint"))
        .and(body_json(fixture!(
            "sprints/p2b_sprint_create_minimal_request.json"
        )))
        .respond_with(
            ResponseTemplate::new(201)
                .set_body_json(fixture!("sprints/p2b_sprint_create_minimal_response.json")),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/rest/agile/1.0/sprint"))
        .and(body_partial_json(json!({ "originBoardId": 99 })))
        .respond_with(err_json(
            404,
            fixture!("sprints/p2b_sprint_create_404_board.json"),
        ))
        .expect(1)
        .mount(&server)
        .await;
    let c = client(&server);
    let s = c.create_sprint(1, "DEMO Sprint 5", None).await.unwrap();
    assert_eq!((s.id, s.state, s.goal), (5, SprintState::Future, None));
    assert!(matches!(
        c.create_sprint(99, "x", None).await,
        Err(Error::NotFound(m)) if m.contains("Board does not exist")
    ));
}

#[tokio::test]
async fn p2b_update_sprint_full_and_invalid_dates() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/rest/agile/1.0/sprint/4"))
        .and(body_json(fixture!(
            "sprints/p2b_sprint_update_request.json"
        )))
        .respond_with(ok_json(fixture!("sprints/p2b_sprint_update_response.json")))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/rest/agile/1.0/sprint/5"))
        .respond_with(err_json(
            400,
            fixture!("sprints/p2b_sprint_400_invalid.json"),
        ))
        .expect(1)
        .mount(&server)
        .await;
    let c = client(&server);
    let s = c
        .update_sprint(
            4,
            &SprintUpdate {
                name: Some("DEMO Sprint 4 - Release 1.1".into()),
                goal: Some(Some("Livrer la 1.1".into())),
                start_date: Some("2026-12-01T08:00:00.000Z".into()),
                end_date: Some("2026-12-15T17:00:00.000Z".into()),
            },
        )
        .await
        .unwrap();
    assert_eq!(s.goal.as_deref(), Some("Livrer la 1.1"));
    assert_eq!(s.end_date.as_deref(), Some("2026-12-15T17:00:00.000Z"));
    let r = c
        .update_sprint(
            5,
            &SprintUpdate {
                end_date: Some("2026-01-01T00:00:00.000Z".into()),
                ..Default::default()
            },
        )
        .await;
    assert!(
        matches!(r, Err(Error::Api { status: 400, ref message }) if message.starts_with("endDate : "))
    );
}

#[tokio::test]
async fn p2b_update_closed_sprint_name_goal() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/rest/agile/1.0/sprint/1"))
        .and(body_json(fixture!(
            "sprints/p2b_sprint_update_closed_request.json"
        )))
        .respond_with(ok_json(fixture!(
            "sprints/p2b_sprint_update_closed_response.json"
        )))
        .expect(1)
        .mount(&server)
        .await;
    // Même date que la réponse (autre décalage horaire, même instant) : pas d'erreur.
    Mock::given(method("POST"))
        .and(path("/rest/agile/1.0/sprint/2"))
        .and(body_json(json!({
            "name": "DEMO Sprint 1 (archive)",
            "endDate": "2026-11-01T18:00:00.000+01:00"
        })))
        .respond_with(ok_json(fixture!(
            "sprints/p2b_sprint_update_closed_response.json"
        )))
        .expect(1)
        .mount(&server)
        .await;
    let c = client(&server);
    // Jira ignore (200) la date de fin d'un sprint clôturé : le client le signale explicitement.
    match c
        .update_sprint(
            1,
            &SprintUpdate {
                name: Some("DEMO Sprint 1 (archive)".into()),
                goal: Some(Some("Socle livre".into())),
                end_date: Some("2026-11-20T17:00:00.000Z".into()),
                ..Default::default()
            },
        )
        .await
    {
        Err(Error::Invalid(m)) => {
            assert!(m.contains("dates ignorées par Jira : sprint fermé"), "{m}")
        }
        r => panic!("{r:?}"),
    }
    let s = c
        .update_sprint(
            2,
            &SprintUpdate {
                name: Some("DEMO Sprint 1 (archive)".into()),
                end_date: Some("2026-11-01T18:00:00.000+01:00".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(s.state, SprintState::Closed);
    assert_eq!(s.end_date.as_deref(), Some("2026-11-01T17:00:00.000Z"));
}

#[tokio::test]
async fn p2b_transitions_with_fields() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rest/api/3/issue/DEMO-5/transitions"))
        .and(query_param("expand", "transitions.fields"))
        .respond_with(ok_json(fixture!("issues/p2b_transitions_with_fields.json")))
        .expect(1)
        .mount(&server)
        .await;
    let tr = client(&server).transitions("DEMO-5").await.unwrap();
    let flags: Vec<(&str, bool)> = tr.iter().map(|t| (t.id.as_str(), t.has_screen)).collect();
    assert_eq!(flags, vec![("21", false), ("31", true), ("41", true)]);
    assert_eq!(tr[2].to.name, "In Review");
}

#[tokio::test]
async fn p2b_transition_errors() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/rest/api/3/issue/DEMO-5/transitions"))
        .and(body_json(json!({ "transition": { "id": "31" } })))
        .respond_with(err_json(
            400,
            fixture!("issues/p2b_transition_400_required_field.json"),
        ))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/rest/api/3/issue/DEMO-6/transitions"))
        .respond_with(err_json(
            400,
            fixture!("issues/p2b_transition_400_invalid_transition.json"),
        ))
        .mount(&server)
        .await;
    let c = client(&server);
    assert!(matches!(
        c.transition_issue("DEMO-5", "31").await,
        Err(Error::Api { status: 400, ref message }) if message == "resolution : Resolution is required."
    ));
    assert!(matches!(
        c.transition_issue("DEMO-6", "31").await,
        Err(Error::Api { status: 400, ref message }) if message.contains("not valid")
    ));
}

#[tokio::test]
async fn p2b_rank_207_shapes() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/rest/agile/1.0/issue/rank"))
        .and(body_partial_json(json!({ "rankBeforeIssue": "DEMO-1" })))
        .respond_with(
            ResponseTemplate::new(207)
                .set_body_json(fixture!("backlog/p2b_rank_partial_207_official_shape.json")),
        )
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/rest/agile/1.0/issue/rank"))
        .and(body_partial_json(json!({ "rankBeforeIssue": "DEMO-2" })))
        .respond_with(
            ResponseTemplate::new(207).set_body_json(fixture!("backlog/p2b_rank_207_all_ok.json")),
        )
        .mount(&server)
        .await;
    let c = client(&server);
    let ks: Vec<String> = vec!["DEMO-4".into(), "DEMO-5".into(), "DEMO-6".into()];
    match c.rank(&ks, &RankPosition::Before("DEMO-1".into())).await {
        Err(Error::Api {
            status: 207,
            message,
        }) => {
            assert!(
                message.contains("DEMO-6 (JIRA Agile cannot execute"),
                "{message}"
            );
            assert!(!message.contains("DEMO-4") && !message.contains("DEMO-5"));
        }
        r => panic!("{r:?}"),
    }
    // 207 dont toutes les entrées sont en succès : OK.
    c.rank(&ks[..2], &RankPosition::Before("DEMO-2".into()))
        .await
        .unwrap();
}

#[tokio::test]
async fn p2b_update_version_full_and_archive() {
    let server = MockServer::start().await;
    let mut expected = fixture!("versions/p2b_version_update_request.json");
    expected["moveUnfixedIssuesTo"] = json!(target_self(&server, "10102"));
    mount_target_version(&server, "10102").await;
    Mock::given(method("PUT"))
        .and(path("/rest/api/3/version/10101"))
        .and(body_json(expected))
        .respond_with(ok_json(fixture!(
            "versions/p2b_version_update_response.json"
        )))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/rest/api/3/version/10100"))
        .and(body_json(fixture!(
            "versions/p2b_version_archive_request.json"
        )))
        .respond_with(ok_json(fixture!(
            "versions/p2b_version_archived_response.json"
        )))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/rest/api/3/version/10199"))
        .respond_with(err_json(404, fixture!("versions/p2b_version_404.json")))
        .mount(&server)
        .await;
    let c = client(&server);
    let v = c
        .update_version(
            "10101",
            &VersionUpdate {
                name: Some("1.1.0".into()),
                description: Some(Some("Correctifs et versions".into())),
                start_date: Some(Some("2026-12-01".into())),
                release_date: Some(Some("2026-12-15".into())),
                released: Some(true),
                archived: Some(false),
                move_unfixed_issues_to: Some("10102".into()),
            },
        )
        .await
        .unwrap();
    assert!(v.released);
    assert_eq!(v.start_date.as_deref(), Some("2026-12-01"));
    let a = c
        .update_version(
            "10100",
            &VersionUpdate {
                archived: Some(true),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(a.archived);
    assert!(matches!(
        c.update_version(
            "10199",
            &VersionUpdate {
                archived: Some(true),
                ..Default::default()
            }
        )
        .await,
        Err(Error::NotFound(m)) if m.contains("10199")
    ));
}

#[tokio::test]
async fn p2b_remove_and_swap() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/rest/api/3/version/10100/removeAndSwap"))
        .and(body_json(fixture!(
            "versions/p2b_version_remove_and_swap_fix_only_request.json"
        )))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/rest/api/3/version/10099/removeAndSwap"))
        .and(body_json(fixture!(
            "versions/p2b_version_remove_and_swap_empty_request.json"
        )))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/rest/api/3/version/10098/removeAndSwap"))
        .respond_with(err_json(
            400,
            fixture!("versions/p2b_version_remove_and_swap_400.json"),
        ))
        .expect(1)
        .mount(&server)
        .await;
    let c = client(&server);
    c.delete_version("10100", Some("10101".into()))
        .await
        .unwrap();
    c.delete_version("10099", None).await.unwrap();
    assert!(matches!(
        c.delete_version("10098", Some("20000".into())).await,
        Err(Error::Api { status: 400, .. })
    ));
}

#[tokio::test]
async fn p2b_envelopes_302_503_429_beta() {
    let server = MockServer::start().await;
    let c = client(&server);

    // 302 : jamais suivi.
    Mock::given(method("GET"))
        .and(path("/rest/api/3/myself"))
        .respond_with(envelope(fixture!("errors/p2b_302_envelope.json")))
        .up_to_n_times(1)
        .with_priority(1)
        .expect(1)
        .mount(&server)
        .await;
    assert!(matches!(
        c.myself().await,
        Err(Error::Api { status: 302, .. })
    ));

    // 429 quota (Retry-After 1847 s) : échec immédiat, sans attente.
    Mock::given(method("GET"))
        .and(path("/rest/api/3/myself"))
        .respond_with(envelope(fixture!("errors/p2b_429_envelope_full.json")))
        .up_to_n_times(1)
        .with_priority(1)
        .expect(1)
        .mount(&server)
        .await;
    assert!(matches!(
        c.myself().await,
        Err(Error::RateLimited { retry_after: Some(d) }) if d == Duration::from_secs(1847)
    ));

    // 503 HTML avec Retry-After (10 s > plafond de test) : échec immédiat, corps HTML non recopié.
    Mock::given(method("GET"))
        .and(path("/rest/api/3/myself"))
        .respond_with(envelope(fixture!(
            "errors/p2b_503_retry_after_envelope.json"
        )))
        .up_to_n_times(1)
        .with_priority(1)
        .expect(1)
        .mount(&server)
        .await;
    assert!(matches!(
        c.myself().await,
        Err(Error::Api { status: 503, ref message }) if message == "Service Unavailable"
    ));

    // 200 avec en-têtes bêta (Beta-Retry-After) : simple succès.
    Mock::given(method("GET"))
        .and(path("/rest/api/3/myself"))
        .respond_with(envelope(fixture!(
            "errors/p2b_200_beta_headers_envelope.json"
        )))
        .with_priority(2)
        .expect(1)
        .mount(&server)
        .await;
    assert_eq!(c.myself().await.unwrap().display_name, "alice");
}

#[tokio::test]
async fn p2b_update_version_move_target_errors() {
    let server = MockServer::start().await;
    // Cible inconnue : 404 sur la relecture, aucune écriture.
    Mock::given(method("GET"))
        .and(path("/rest/api/3/version/10199"))
        .respond_with(err_json(404, fixture!("versions/p2b_version_404.json")))
        .expect(1)
        .mount(&server)
        .await;
    // Cible sans `self` exploitable : erreur de décodage, aucune écriture.
    let mut no_self = fixture!("versions/version_released.json");
    no_self["id"] = json!("10101");
    no_self.as_object_mut().unwrap().remove("self");
    Mock::given(method("GET"))
        .and(path("/rest/api/3/version/10101"))
        .respond_with(ok_json(no_self))
        .expect(1)
        .mount(&server)
        .await;
    let mut wrong_self = fixture!("versions/version_released.json");
    wrong_self["id"] = json!("10102");
    Mock::given(method("GET"))
        .and(path("/rest/api/3/version/10102"))
        .respond_with(ok_json(wrong_self))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&server)
        .await;
    let c = client(&server);
    let upd = |t: &str| VersionUpdate {
        released: Some(true),
        move_unfixed_issues_to: Some(t.into()),
        ..Default::default()
    };
    assert!(matches!(
        c.update_version("10100", &upd("10199")).await,
        Err(Error::NotFound(m)) if m.contains("10199")
    ));
    assert!(matches!(
        c.update_version("10100", &upd("10101")).await,
        Err(Error::Decode(m)) if m.contains("self")
    ));
    // `self` pointant vers une autre version (10100) que l'id demandé (10102).
    assert!(matches!(
        c.update_version("10099", &upd("10102")).await,
        Err(Error::Decode(m)) if m.contains("self")
    ));
}

#[tokio::test]
async fn p2b_transitions_looped_both_spellings() {
    // La spec nomme le champ `looped`, les réponses observées `isLooped` : le client ne le lit pas,
    // les deux formes doivent être acceptées.
    let server = MockServer::start().await;
    let with_is_looped = fixture!("issues/transitions.json");
    let raw = serde_json::to_string(&with_is_looped).unwrap();
    assert!(raw.contains("\"isLooped\""));
    let with_looped: Value =
        serde_json::from_str(&raw.replace("\"isLooped\"", "\"looped\"")).unwrap();
    Mock::given(method("GET"))
        .and(path("/rest/api/3/issue/DEMO-1/transitions"))
        .respond_with(ok_json(with_is_looped))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/rest/api/3/issue/DEMO-2/transitions"))
        .respond_with(ok_json(with_looped))
        .mount(&server)
        .await;
    let c = client(&server);
    let a = c.transitions("DEMO-1").await.unwrap();
    let b = c.transitions("DEMO-2").await.unwrap();
    assert!(!a.is_empty());
    assert_eq!(a, b);
}

#[tokio::test]
async fn p2b_maybe_permission_atypical_codes() {
    let server = MockServer::start().await;
    let perm = |m: &str| json!({ "errorMessages": [m], "errors": {} });
    Mock::given(method("PUT"))
        .and(path("/rest/api/3/version/10100"))
        .respond_with(err_json(
            400,
            perm("You do not have permission to edit this version."),
        ))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/rest/api/3/version/10100/removeAndSwap"))
        .respond_with(err_json(
            404,
            perm("The version does not exist or you do not have the required Permissions."),
        ))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/rest/api/3/issue/DEMO-5/transitions"))
        .respond_with(err_json(
            400,
            perm("You do not have permission to transition this issue."),
        ))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/rest/api/3/issue/DEMO-6/transitions"))
        .respond_with(err_json(
            400,
            fixture!("issues/p2b_transition_400_required_field.json"),
        ))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/rest/agile/1.0/sprint/3"))
        .respond_with(err_json(403, fixture!("errors/403.json")))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/rest/api/3/myself"))
        .respond_with(err_json(401, fixture!("errors/401.json")))
        .mount(&server)
        .await;
    let c = client(&server);
    let e = c
        .update_version(
            "10100",
            &VersionUpdate {
                archived: Some(true),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert!(
        matches!(e, Error::Api { status: 400, .. }) && e.maybe_permission(),
        "{e:?}"
    );
    let e = c.delete_version("10100", None).await.unwrap_err();
    assert!(
        matches!(e, Error::NotFound(_)) && e.maybe_permission(),
        "{e:?}"
    );
    let e = c.transition_issue("DEMO-5", "31").await.unwrap_err();
    assert!(e.maybe_permission(), "{e:?}");
    let e = c.transition_issue("DEMO-6", "31").await.unwrap_err();
    assert!(!e.maybe_permission(), "{e:?}");
    let e = c
        .update_sprint(
            3,
            &SprintUpdate {
                name: Some("x".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(e, Error::Forbidden(_)) && e.maybe_permission());
    let e = c.myself().await.unwrap_err();
    assert!(matches!(e, Error::Unauthorized) && e.maybe_permission());
}

// --- Relecture avant production : boards team-managed, réconciliation, champ par board, divers ---

const FIELDS_BASE: [&str; 8] = [
    "summary",
    "description",
    "status",
    "issuetype",
    "priority",
    "assignee",
    "fixVersions",
    "customfield_10020",
];

fn search_fields(points: &[&str]) -> Value {
    json!(FIELDS_BASE.iter().chain(points).collect::<Vec<_>>())
}

/// Issue minimale de `search/jql` (statut « To Do », type Story) ; `extra` complète `fields`.
fn raw_issue(id: &str, key: &str, extra: Value) -> Value {
    let mut fields = json!({
        "summary": format!("Issue {key}"),
        "status": { "id": "10000", "name": "To Do", "statusCategory": { "key": "new" } },
        "issuetype": { "id": "10001", "name": "Story", "subtask": false },
    });
    for (k, v) in extra.as_object().unwrap() {
        fields[k] = v.clone();
    }
    json!({ "id": id, "key": key, "fields": fields })
}

fn last_page(issues: Vec<Value>) -> Value {
    json!({ "issues": issues, "isLast": true })
}

fn board_config_with(board: u64, filter: u64, points_field: &str) -> Value {
    let mut cfg = fixture!("projects/board_configuration.json");
    cfg["id"] = json!(board);
    cfg["filter"]["id"] = json!(filter.to_string());
    cfg["estimation"]["field"]["fieldId"] = json!(points_field);
    cfg
}

/// Corps des `POST search/jql` reçus, dans l'ordre.
async fn search_bodies(server: &MockServer) -> Vec<Value> {
    server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|r| r.method.as_str() == "POST" && r.url.path() == "/rest/api/3/search/jql")
        .map(|r| serde_json::from_slice(&r.body).unwrap())
        .collect()
}

#[tokio::test]
async fn simple_boards_are_listed_and_tolerate_missing_sprints() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rest/agile/1.0/board"))
        .and(query_param("type", "scrum"))
        .and(query_param("projectKeyOrId", "TEAM"))
        .respond_with(ok_json(
            json!({"startAt":0,"maxResults":50,"isLast":true,"values":[]}),
        ))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/rest/agile/1.0/board"))
        .and(query_param("type", "simple"))
        .and(query_param("projectKeyOrId", "TEAM"))
        .respond_with(ok_json(json!({
            "startAt": 0, "maxResults": 50, "isLast": true,
            "values": [
                { "id": 5, "name": "TEAM board", "type": "simple",
                  "location": { "projectId": 10050, "projectKey": "TEAM" } },
                { "id": 6, "name": "TEAM board 2", "type": "simple",
                  "location": { "projectId": 10050, "projectKey": "TEAM" } }
            ]
        })))
        .expect(1)
        .mount(&server)
        .await;
    // Configuration illisible : ignorée.
    Mock::given(method("GET"))
        .and(path_regex_config())
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/rest/agile/1.0/board/5/sprint"))
        .respond_with(err_json(
            400,
            json!({ "errorMessages": ["The board does not support sprints"], "errors": {} }),
        ))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/rest/agile/1.0/board/6/sprint"))
        .respond_with(err_json(404, fixture!("errors/404.json")))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/rest/agile/1.0/board/7/sprint"))
        .respond_with(err_json(404, fixture!("errors/404.json")))
        .expect(1)
        .mount(&server)
        .await;
    let c = client(&server);
    let boards = c.scrum_boards("TEAM").await.unwrap();
    assert_eq!(boards.len(), 2);
    assert_eq!(boards[0].board_type.as_deref(), Some("simple"));
    assert_eq!(boards[0].project_key.as_deref(), Some("TEAM"));
    // 400 « pas de sprints » : liste vide.
    assert!(
        c.sprints(5, &[SprintState::Active])
            .await
            .unwrap()
            .is_empty()
    );
    // 404 sur un board connu comme `simple` : liste vide.
    assert!(c.sprints(6, &[]).await.unwrap().is_empty());
    // 404 sur un board inconnu : erreur conservée.
    assert!(matches!(c.sprints(7, &[]).await, Err(Error::NotFound(_))));
}

fn path_regex_config() -> wiremock::matchers::PathRegexMatcher {
    wiremock::matchers::path_regex(r"^/rest/agile/1\.0/board/\d+/configuration$")
}

#[tokio::test]
async fn backlog_excludes_future_sprints_explicitly() {
    let server = MockServer::start().await;
    mount_board_config(&server).await;
    mount_fields(&server).await;
    Mock::given(method("POST"))
        .and(path("/rest/api/3/search/jql"))
        .and(body_partial_json(json!({
            "jql": "filter = 10001 AND (sprint is EMPTY OR (sprint not in openSprints() \
                    AND sprint not in futureSprints())) AND statusCategory != Done \
                    AND issuetype not in subtaskIssueTypes() ORDER BY Rank ASC"
        })))
        .respond_with(ok_json(last_page(vec![])))
        .expect(1)
        .mount(&server)
        .await;
    assert!(client(&server).backlog_issues(1).await.unwrap().is_empty());
}

#[tokio::test]
async fn written_issues_are_reconciled_in_following_searches() {
    let server = MockServer::start().await;
    mount_fields(&server).await;
    let page = last_page(vec![
        raw_issue("10000", "DEMO-1", json!({})),
        raw_issue("10001", "DEMO-2", json!({})),
        raw_issue("10002", "DEMO-3", json!({})),
    ]);
    // Recherche après écritures : ids réconciliés, dans l'ordre des écritures (le plus récent en
    // dernier, sans doublon).
    Mock::given(method("POST"))
        .and(path("/rest/api/3/search/jql"))
        .and(body_partial_json(
            json!({ "reconcileIssues": [10001, 10002, 10000] }),
        ))
        .respond_with(ok_json(page.clone()))
        .with_priority(1)
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/rest/api/3/search/jql"))
        .respond_with(ok_json(page))
        .with_priority(5)
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/rest/api/3/issue/DEMO-1"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/rest/api/3/issue/DEMO-2/transitions"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/rest/agile/1.0/backlog/issue"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let c = client(&server);
    c.sprint_issues(2).await.unwrap();
    c.update_issue(
        "DEMO-1",
        &IssueUpdate {
            summary: Some("x".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    c.transition_issue("DEMO-2", "21").await.unwrap();
    // DEMO-99 jamais lue (id inconnu) : ignorée ; DEMO-1 repasse en dernier.
    c.move_to_backlog(&keys(&["DEMO-3".into(), "DEMO-99".into(), "DEMO-1".into()]))
        .await
        .unwrap();
    c.sprint_issues(2).await.unwrap();

    let bodies = search_bodies(&server).await;
    assert_eq!(bodies.len(), 2);
    assert!(
        bodies[0].get("reconcileIssues").is_none(),
        "aucune écriture avant la première recherche : {}",
        bodies[0]
    );
}

#[tokio::test]
async fn reconciliation_expires_and_respects_read_only() {
    let server = MockServer::start().await;
    // Deux clients : le catalogue est lu une fois par client.
    Mock::given(method("GET"))
        .and(path("/rest/api/3/field"))
        .respond_with(ok_json(fixture!("auth/fields.json")))
        .expect(2)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/rest/api/3/search/jql"))
        .respond_with(ok_json(last_page(vec![raw_issue(
            "10000",
            "DEMO-1",
            json!({}),
        )])))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    let upd = IssueUpdate {
        summary: Some("x".into()),
        ..Default::default()
    };
    // Fenêtre nulle : rien n'est réconcilié.
    let c = client(&server).with_reconcile_window(Duration::ZERO);
    c.sprint_issues(2).await.unwrap();
    c.update_issue("DEMO-1", &upd).await.unwrap();
    c.sprint_issues(2).await.unwrap();
    let bodies = search_bodies(&server).await;
    assert!(bodies.iter().all(|b| b.get("reconcileIssues").is_none()));

    // Lecture seule : l'écriture est refusée sans requête, la recherche (avec réconciliation,
    // inoffensive) reste autorisée.
    let ro = client(&server).with_read_only(true);
    ro.sprint_issues(2).await.unwrap();
    assert!(matches!(
        ro.update_issue("DEMO-1", &upd).await,
        Err(Error::ReadOnly(_))
    ));
    ro.sprint_issues(2).await.unwrap();
    let puts = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.method.as_str() == "PUT")
        .count();
    assert_eq!(
        puts, 1,
        "seule l'écriture du client non restreint est partie"
    );
}

#[tokio::test]
async fn story_points_field_is_resolved_per_board() {
    let server = MockServer::start().await;
    mount_fields(&server).await;
    // Board 1 (DEMO) : champ JSW 10016 ; board 2 (OPS) : champ legacy 10028.
    Mock::given(method("GET"))
        .and(path("/rest/agile/1.0/board/1/configuration"))
        .respond_with(ok_json(board_config_with(1, 10001, "customfield_10016")))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/rest/agile/1.0/board/2/configuration"))
        .respond_with(ok_json(board_config_with(2, 10002, "customfield_10028")))
        .expect(1)
        .mount(&server)
        .await;
    let both = json!({ "customfield_10016": 1.0, "customfield_10028": 5.0 });
    Mock::given(method("POST"))
        .and(path("/rest/api/3/search/jql"))
        .and(body_partial_json(json!({
            "fields": search_fields(&["customfield_10016", "customfield_10028"])
        })))
        .and(wiremock::matchers::body_string_contains("filter = 10001"))
        .respond_with(ok_json(last_page(vec![raw_issue(
            "10000",
            "DEMO-1",
            both.clone(),
        )])))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/rest/api/3/search/jql"))
        .and(body_partial_json(json!({
            "fields": search_fields(&["customfield_10028", "customfield_10016"])
        })))
        .and(wiremock::matchers::body_string_contains("filter = 10002"))
        .respond_with(ok_json(last_page(vec![raw_issue(
            "20001",
            "OPS-1",
            both.clone(),
        )])))
        .expect(1)
        .mount(&server)
        .await;
    // Issues d'un sprint du board 2 : champ du board 2 en tête.
    Mock::given(method("GET"))
        .and(path("/rest/agile/1.0/board/2/sprint"))
        .respond_with(ok_json(json!({
            "startAt": 0, "maxResults": 50, "isLast": true,
            "values": [ { "id": 20, "name": "OPS Sprint 1", "state": "active", "originBoardId": 2 } ]
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/rest/api/3/search/jql"))
        .and(body_partial_json(json!({
            "jql": "sprint = 20 ORDER BY Rank ASC",
            "fields": search_fields(&["customfield_10028", "customfield_10016"])
        })))
        .respond_with(ok_json(last_page(vec![raw_issue("20002", "OPS-2", both)])))
        .expect(1)
        .mount(&server)
        .await;

    let c = client(&server);
    let demo = c.backlog_issues(1).await.unwrap();
    assert_eq!(demo[0].story_points, Some(1.0));
    let ops = c.backlog_issues(2).await.unwrap();
    assert_eq!(ops[0].story_points, Some(5.0));
    c.sprints(2, &[]).await.unwrap();
    let sprint = c.sprint_issues(20).await.unwrap();
    assert_eq!(sprint[0].story_points, Some(5.0));

    // Écritures : champ du board où l'issue (ou, à défaut, son projet) a été lue.
    for (key, field) in [
        ("DEMO-1", "customfield_10016"),
        ("OPS-1", "customfield_10028"),
        ("OPS-77", "customfield_10028"),
    ] {
        Mock::given(method("PUT"))
            .and(path(format!("/rest/api/3/issue/{key}")))
            .and(body_json(json!({ "fields": { field: 2.0 } })))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;
        c.update_issue(
            key,
            &IssueUpdate {
                story_points: Some(Some(2.0)),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    }
    // Création : dernier board lu du projet ; projet jamais lu -> catalogue global (JSW).
    for (project, field, created) in [
        ("OPS", "customfield_10028", "OPS-3"),
        ("NEW", "customfield_10016", "NEW-1"),
    ] {
        Mock::given(method("POST"))
            .and(path("/rest/api/3/issue"))
            .and(body_partial_json(
                json!({ "fields": { "project": { "key": project }, field: 3.0 } }),
            ))
            .respond_with(
                ResponseTemplate::new(201).set_body_json(json!({ "id": "30000", "key": created })),
            )
            .expect(1)
            .mount(&server)
            .await;
        let key = c
            .create_issue(&NewIssue {
                project_key: project.into(),
                issue_type_id: "10001".into(),
                summary: "x".into(),
                story_points: Some(3.0),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(key, created);
    }
}

#[tokio::test]
async fn reads_retry_on_503_with_retry_after_but_writes_do_not() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rest/api/3/myself"))
        .respond_with(ResponseTemplate::new(503).insert_header("Retry-After", "0"))
        .up_to_n_times(1)
        .with_priority(1)
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/rest/api/3/myself"))
        .respond_with(ok_json(fixture!("auth/myself.json")))
        .with_priority(2)
        .expect(1)
        .mount(&server)
        .await;
    let c = client(&server);
    c.myself().await.unwrap();

    // 503 sans Retry-After : pas de nouvel essai.
    Mock::given(method("GET"))
        .and(path("/rest/api/3/project/DEMO/version"))
        .respond_with(ResponseTemplate::new(503))
        .expect(1)
        .mount(&server)
        .await;
    assert!(matches!(
        c.versions("DEMO").await,
        Err(Error::Api { status: 503, .. })
    ));

    // Écriture : jamais retentée.
    Mock::given(method("POST"))
        .and(path("/rest/api/3/issue/DEMO-1/transitions"))
        .respond_with(ResponseTemplate::new(503).insert_header("Retry-After", "0"))
        .expect(1)
        .mount(&server)
        .await;
    assert!(matches!(
        c.transition_issue("DEMO-1", "21").await,
        Err(Error::Api { status: 503, .. })
    ));
}

#[tokio::test]
async fn redirect_error_names_location_host_only() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rest/api/3/myself"))
        .respond_with(envelope(fixture!("errors/p2b_302_envelope.json")))
        .expect(1)
        .mount(&server)
        .await;
    match client(&server).myself().await {
        Err(Error::Api {
            status: 302,
            message,
        }) => {
            assert!(message.contains("jira-renamed.example.test"), "{message}");
            assert!(!message.contains("/rest/api/3/myself"), "{message}");
        }
        r => panic!("{r:?}"),
    }
}

#[test]
fn base_url_with_credentials_is_rejected_without_echo() {
    for url in [
        "https://alice:s3cr3t@jira.example.test",
        "https://alice:s3cr3t@[::bad",
        "https:alice:s3cr3t@jira.example.test",
        "http://alice:s3cr3t@jira.example.test",
        "ftp://alice:s3cr3t@jira.example.test",
    ] {
        let err = JiraHttp::new(url, "a@b.c", SecretString::new("t")).unwrap_err();
        assert!(matches!(err, Error::InvalidUrl(_)), "{url} : {err:?}");
        let msg = err.to_string();
        assert!(msg.contains("identifiants"), "{url} : {msg}");
        assert!(!msg.contains("s3cr3t") && !msg.contains("alice"), "{msg}");
    }
    // Les autres refus n'affichent que l'origine (jamais chemin ni requête).
    for url in [
        "http://jira.example.test/x?token=s3cr3t",
        "ftp://jira.example.test/?token=s3cr3t",
        "pas une url s3cr3t",
    ] {
        let err = JiraHttp::new(url, "a@b.c", SecretString::new("t")).unwrap_err();
        assert!(matches!(err, Error::InvalidUrl(_)), "{url} : {err:?}");
        assert!(!err.to_string().contains("s3cr3t"), "{err}");
    }
}

#[tokio::test]
async fn malformed_issues_are_degraded_not_fatal() {
    let server = MockServer::start().await;
    mount_fields(&server).await;
    let mut broken = raw_issue(
        "10000",
        "DEMO-1",
        json!({
            "assignee": { "accountId": "712020:0000-alice" },
            "priority": 42,
            "fixVersions": "pas une liste"
        }),
    );
    let f = broken["fields"].as_object_mut().unwrap();
    f.remove("status");
    f.remove("issuetype");
    Mock::given(method("POST"))
        .and(path("/rest/api/3/search/jql"))
        .respond_with(ok_json(last_page(vec![
            broken,
            raw_issue("10001", "DEMO-2", json!({ "status": "illisible" })),
            raw_issue("10002", "DEMO-3", json!({})),
        ])))
        .expect(1)
        .mount(&server)
        .await;
    let issues = client(&server).sprint_issues(2).await.unwrap();
    assert_eq!(issue_keys(&issues), vec!["DEMO-1", "DEMO-2", "DEMO-3"]);
    let i1 = &issues[0];
    assert_eq!(i1.status.category, StatusCategory::ToDo);
    assert_eq!(i1.status.name, "(inconnu)");
    assert_eq!(i1.issue_type.name, "(inconnu)");
    assert!(!i1.issue_type.subtask);
    let a = i1.assignee.as_ref().unwrap();
    assert_eq!(a.account_id, "712020:0000-alice");
    assert_eq!(a.display_name, "(utilisateur inconnu)");
    assert_eq!(i1.priority, None);
    assert!(i1.fix_versions.is_empty());
    assert_eq!(issues[1].status.name, "(inconnu)");
    assert_eq!(issues[2].status.name, "To Do");
}

#[tokio::test]
async fn move_unfixed_target_on_other_host_is_refused() {
    let server = MockServer::start().await;
    let mut v = fixture!("versions/version_released.json");
    v["id"] = json!("10101");
    v["self"] = json!("https://elsewhere.example.test/rest/api/3/version/10101");
    Mock::given(method("GET"))
        .and(path("/rest/api/3/version/10101"))
        .respond_with(ok_json(v))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&server)
        .await;
    let r = client(&server)
        .update_version(
            "10100",
            &VersionUpdate {
                released: Some(true),
                move_unfixed_issues_to: Some("10101".into()),
                ..Default::default()
            },
        )
        .await;
    assert!(
        matches!(&r, Err(Error::Decode(m)) if m.contains("hôte") && !m.contains("elsewhere")),
        "{r:?}"
    );
}
