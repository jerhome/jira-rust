//! Tests d'intégration : serveur sur un port éphémère de 127.0.0.1, requêtes reqwest.

use std::time::Duration;

use jira_fake::Options;
use reqwest::{Client, Method, StatusCode};
use serde_json::{Value, json};

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../jira-client/tests/fixtures");

fn fixture(path: &str) -> Value {
    let text = std::fs::read_to_string(format!("{FIXTURES}/{path}"))
        .unwrap_or_else(|e| panic!("fixture {path} : {e}"));
    serde_json::from_str(&text).unwrap()
}

/// Toutes les clés de premier niveau de la fixture sont présentes dans la réponse.
#[track_caller]
fn assert_keys(actual: &Value, expected: &Value) {
    let a = actual.as_object().expect("objet attendu");
    for k in expected.as_object().expect("fixture objet").keys() {
        assert!(a.contains_key(k), "clé « {k} » absente de {actual:#}");
    }
}

struct Fake {
    base: String,
    http: Client,
}

impl Fake {
    async fn start(opts: Options) -> Self {
        let listener = jira_fake::bind(0).await.unwrap();
        let addr = listener.local_addr().unwrap();
        assert!(addr.ip().is_loopback());
        tokio::spawn(jira_fake::serve(
            listener,
            Options {
                quiet: true,
                ..opts
            },
            std::future::pending(),
        ));
        Self {
            base: format!("http://{addr}"),
            http: Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .unwrap(),
        }
    }

    async fn default() -> Self {
        Self::start(Options::default()).await
    }

    async fn call(&self, method: Method, path: &str, body: Option<Value>) -> (StatusCode, Value) {
        let mut req = self
            .http
            .request(method, format!("{}{path}", self.base))
            .basic_auth("alice@example.com", Some("jeton-factice"))
            .header("Accept", "application/json");
        if let Some(b) = body {
            req = req.json(&b);
        }
        let resp = req.send().await.unwrap();
        let status = resp.status();
        let text = resp.text().await.unwrap();
        let v = if text.is_empty() {
            Value::Null
        } else {
            serde_json::from_str(&text).unwrap_or_else(|e| panic!("JSON invalide ({e}) : {text}"))
        };
        (status, v)
    }

    async fn get(&self, path: &str) -> (StatusCode, Value) {
        self.call(Method::GET, path, None).await
    }
    async fn post(&self, path: &str, body: Value) -> (StatusCode, Value) {
        self.call(Method::POST, path, Some(body)).await
    }
    async fn put(&self, path: &str, body: Value) -> (StatusCode, Value) {
        self.call(Method::PUT, path, Some(body)).await
    }

    /// Toutes les pages d'une recherche JQL ; renvoie les clés.
    async fn search_keys(&self, jql: &str, page: u64) -> Vec<String> {
        let mut keys = Vec::new();
        let mut token: Option<String> = None;
        for _ in 0..50 {
            let mut body = json!({ "jql": jql, "fields": ["summary"], "maxResults": page });
            if let Some(t) = &token {
                body["nextPageToken"] = json!(t);
            }
            let (s, v) = self.post("/rest/api/3/search/jql", body).await;
            assert_eq!(s, StatusCode::OK, "{v}");
            for i in v["issues"].as_array().unwrap() {
                keys.push(i["key"].as_str().unwrap().to_owned());
            }
            if v["isLast"] == json!(true) {
                assert!(v.get("nextPageToken").is_none());
                return keys;
            }
            token = Some(v["nextPageToken"].as_str().unwrap().to_owned());
        }
        panic!("pagination sans fin");
    }
}

fn is_error_body(v: &Value) -> bool {
    v["errorMessages"].is_array() && v["errors"].is_object()
}

// --- Comportements transverses ------------------------------------------------------------------

#[tokio::test]
async fn auth_required() {
    let f = Fake::default().await;
    let resp = f
        .http
        .get(format!("{}/rest/api/3/myself", f.base))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    let v: Value = resp.json().await.unwrap();
    assert!(is_error_body(&v));
    assert_keys(&v, &fixture("errors/401.json"));

    let resp = f
        .http
        .get(format!("{}/rest/api/3/myself", f.base))
        .bearer_auth("x")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn unknown_endpoint_is_json_404() {
    let f = Fake::default().await;
    let (s, v) = f.get("/rest/api/3/search").await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    assert!(is_error_body(&v));
}

#[tokio::test]
async fn fail_rate_returns_429_with_retry_after() {
    let f = Fake::start(Options {
        fail_rate: 1.0,
        retry_after_secs: 1,
        ..Options::default()
    })
    .await;
    let resp = f
        .http
        .get(format!("{}/rest/api/3/myself", f.base))
        .basic_auth("a", Some("b"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(resp.headers()["retry-after"], "1");
}

#[tokio::test]
async fn forbid_writes_returns_403_but_search_works() {
    let f = Fake::start(Options {
        forbid_writes: true,
        ..Options::default()
    })
    .await;
    let (s, v) = f
        .post(
            "/rest/agile/1.0/backlog/issue",
            json!({ "issues": ["DEMO-1"] }),
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    assert!(is_error_body(&v));
    let (s, _) = f
        .put("/rest/api/3/version/101", json!({ "released": true }))
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _) = f
        .post(
            "/rest/api/3/search/jql",
            json!({ "jql": "sprint = 10 ORDER BY Rank ASC" }),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
}

// --- Auth / projets / boards --------------------------------------------------------------------

#[tokio::test]
async fn myself_and_fields() {
    let f = Fake::default().await;
    let (s, v) = f.get("/rest/api/3/myself").await;
    assert_eq!(s, StatusCode::OK);
    assert_keys(&v, &fixture("auth/myself.json"));

    let (s, v) = f.get("/rest/api/3/field").await;
    assert_eq!(s, StatusCode::OK);
    let list = v.as_array().unwrap();
    assert_keys(&list[0], &fixture("auth/fields.json")[0]);
    let custom = |kind: &str| {
        list.iter()
            .find(|f| f["schema"]["custom"] == json!(format!("com.pyxis.greenhopper.jira:{kind}")))
            .map(|f| f["id"].as_str().unwrap().to_owned())
    };
    assert_eq!(
        custom("jsw-story-points").as_deref(),
        Some("customfield_10016")
    );
    assert_eq!(custom("gh-sprint").as_deref(), Some("customfield_10020"));
}

#[tokio::test]
async fn project_search_paginates() {
    let f = Fake::default().await;
    let (s, v) = f.get("/rest/api/3/project/search").await;
    assert_eq!(s, StatusCode::OK);
    let fx = fixture("projects/project_search.json");
    assert_keys(&v, &fx);
    assert_keys(&v["values"][0], &fx["values"][0]);
    assert_eq!(v["isLast"], json!(true));
    assert_eq!(v["values"][0]["key"], "DEMO");
    assert!(v["values"][0]["id"].is_string());

    let (_, p1) = f.get("/rest/api/3/project/search?maxResults=1").await;
    assert_eq!(p1["values"].as_array().unwrap().len(), 1);
    assert_eq!(p1["isLast"], json!(false));
    assert!(p1["nextPage"].as_str().unwrap().contains("startAt=1"));
    let (_, p2) = f
        .get("/rest/api/3/project/search?maxResults=1&startAt=1")
        .await;
    assert_eq!(p2["isLast"], json!(true));
    assert_eq!(p2["values"][0]["key"], "OPS");
}

#[tokio::test]
async fn boards_configuration_and_sprints() {
    let f = Fake::default().await;
    let (s, v) = f
        .get("/rest/agile/1.0/board?type=scrum&projectKeyOrId=DEMO")
        .await;
    assert_eq!(s, StatusCode::OK);
    let fx = fixture("projects/boards_scrum.json");
    assert_keys(&v, &fx);
    assert_keys(&v["values"][0], &fx["values"][0]);
    assert_keys(&v["values"][0]["location"], &fx["values"][0]["location"]);
    assert_eq!(v["values"].as_array().unwrap().len(), 1);
    assert_eq!(v["values"][0]["id"], json!(1));
    assert!(v["values"][0]["location"]["projectId"].is_u64());

    let (s, v) = f.get("/rest/agile/1.0/board/1/configuration").await;
    assert_eq!(s, StatusCode::OK);
    assert_keys(&v, &fixture("projects/board_configuration.json"));
    assert_eq!(v["filter"]["id"], "10001");
    assert_eq!(v["estimation"]["field"]["fieldId"], "customfield_10016");
    assert_eq!(v["ranking"]["rankCustomFieldId"], json!(10019));

    let (s, v) = f
        .get("/rest/agile/1.0/board/1/sprint?state=active,future")
        .await;
    assert_eq!(s, StatusCode::OK);
    let fx = fixture("sprints/board_sprints_active_future.json");
    assert_keys(&v, &fx);
    assert_keys(&v["values"][0], &fx["values"][0]);
    let ids: Vec<u64> = v["values"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["id"].as_u64().unwrap())
        .collect();
    assert_eq!(ids, [10, 11]);

    let (_, all) = f.get("/rest/agile/1.0/board/1/sprint?maxResults=2").await;
    assert_eq!(all["isLast"], json!(false));
    let (s, v) = f.get("/rest/agile/1.0/board/1/sprint?state=bogus").await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(is_error_body(&v));
    let (s, _) = f.get("/rest/agile/1.0/board/99/sprint").await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

// --- Recherche JQL ------------------------------------------------------------------------------

#[tokio::test]
async fn search_sprint_with_pagination_and_fields() {
    let f = Fake::default().await;
    let fx = fixture("issues/search_jql_page1.json");
    let fields = fixture("issues/search_jql_request.json")["fields"].clone();
    let (s, v) = f
        .post(
            "/rest/api/3/search/jql",
            json!({ "jql": "sprint = 10 ORDER BY Rank ASC", "fields": fields, "maxResults": 2 }),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_keys(&v, &fx);
    let issue = &v["issues"][0];
    assert_keys(issue, &fx["issues"][0]);
    assert_keys(&issue["fields"], &fx["issues"][0]["fields"]);
    assert!(
        issue["fields"].get("description").is_none(),
        "champ non demandé"
    );
    assert_eq!(issue["fields"]["customfield_10020"][0]["boardId"], json!(1));
    assert_eq!(v["isLast"], json!(false));

    // Description en ADF quand demandée.
    let (_, v) = f
        .post(
            "/rest/api/3/search/jql",
            json!({ "jql": "sprint = 10", "fields": ["description"], "maxResults": 1 }),
        )
        .await;
    assert_eq!(v["issues"][0]["fields"]["description"]["type"], "doc");
    assert_eq!(v["issues"][0]["fields"].as_object().unwrap().len(), 1);

    // Sans `fields` : id seulement.
    let (_, v) = f
        .post("/rest/api/3/search/jql", json!({ "jql": "sprint = 10" }))
        .await;
    assert!(v["issues"][0]["fields"].as_object().unwrap().is_empty());
    assert!(v["issues"][0]["id"].is_string());

    let keys = f.search_keys("sprint = 10 ORDER BY Rank ASC", 2).await;
    assert_eq!(keys, ["DEMO-1", "DEMO-2", "DEMO-3", "DEMO-4", "DEMO-5"]);
}

/// JQL du backlog émise par jira-rust.
const BACKLOG_JQL: &str = "filter = 10001 AND (sprint is EMPTY OR (sprint not in openSprints() \
     AND sprint not in futureSprints())) AND statusCategory != Done \
     AND issuetype not in subtaskIssueTypes() ORDER BY Rank ASC";

#[tokio::test]
async fn search_backlog_and_version() {
    let f = Fake::default().await;
    let expected = ["DEMO-8", "DEMO-9", "DEMO-10", "DEMO-11", "DEMO-12"];
    assert_eq!(f.search_keys(BACKLOG_JQL, 50).await, expected);
    assert_eq!(f.search_keys(BACKLOG_JQL, 3).await, expected);
    // Forme historique : `openSprints()` ne couvre que le sprint actif (Jira Cloud), les issues du
    // sprint futur 11 (DEMO-6, DEMO-7) restent dans le backlog -> doublon avec le sprint futur.
    let legacy = "filter = 10001 AND (sprint is EMPTY OR sprint not in openSprints()) \
                  AND statusCategory != Done ORDER BY Rank ASC";
    assert_eq!(
        f.search_keys(legacy, 50).await,
        [
            "DEMO-6", "DEMO-7", "DEMO-8", "DEMO-9", "DEMO-10", "DEMO-11", "DEMO-12"
        ]
    );

    let version = "project = \"DEMO\" AND fixVersion = 102 ORDER BY Rank ASC";
    assert_eq!(
        f.search_keys(version, 50).await,
        ["DEMO-6", "DEMO-7", "DEMO-8", "DEMO-11"]
    );
}

#[tokio::test]
async fn search_rejects_unsupported_jql_and_bad_token() {
    let f = Fake::default().await;
    for jql in [
        "project = DEMO AND sprint in openSprints() ORDER BY Rank ASC",
        "assignee = currentUser()",
        "",
    ] {
        let (s, v) = f
            .post("/rest/api/3/search/jql", json!({ "jql": jql }))
            .await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{jql}");
        assert!(is_error_body(&v));
        assert!(!v["errorMessages"].as_array().unwrap().is_empty());
    }
    let (s, _) = f
        .post(
            "/rest/api/3/search/jql",
            json!({ "jql": "sprint = 10", "nextPageToken": "n'importe quoi" }),
        )
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _) = f
        .post("/rest/api/3/search/jql", json!({ "jql": "sprint = 999" }))
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}

// --- Écritures agile ----------------------------------------------------------------------------

#[tokio::test]
async fn move_and_rank() {
    let f = Fake::default().await;
    let (s, v) = f
        .post(
            "/rest/agile/1.0/sprint/11/issue",
            json!({ "issues": ["DEMO-9", "DEMO-12"], "rankAfterIssue": "DEMO-7" }),
        )
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT, "{v}");
    assert_eq!(
        f.search_keys("sprint = 11", 50).await,
        ["DEMO-6", "DEMO-7", "DEMO-9", "DEMO-12"]
    );

    let (s, _) = f
        .post(
            "/rest/agile/1.0/backlog/issue",
            json!({ "issues": ["DEMO-6"] }),
        )
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert_eq!(
        f.search_keys("sprint = 11", 50).await,
        ["DEMO-7", "DEMO-9", "DEMO-12"]
    );

    // Clôturé -> 400 ; inconnu -> 404 ; plus de 50 -> 400.
    let (s, _) = f
        .post(
            "/rest/agile/1.0/sprint/9/issue",
            json!({ "issues": ["DEMO-8"] }),
        )
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _) = f
        .post(
            "/rest/agile/1.0/sprint/999/issue",
            json!({ "issues": ["DEMO-8"] }),
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let many: Vec<String> = (0..51).map(|i| format!("DEMO-{i}")).collect();
    let (s, v) = f
        .post("/rest/agile/1.0/backlog/issue", json!({ "issues": many }))
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(is_error_body(&v));

    // Rang : 204 puis 207 partiel.
    let (s, _) = f
        .put(
            "/rest/agile/1.0/issue/rank",
            json!({ "issues": ["DEMO-12"], "rankBeforeIssue": "DEMO-7", "rankCustomFieldId": 10019 }),
        )
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert_eq!(
        f.search_keys("sprint = 11", 50).await,
        ["DEMO-12", "DEMO-7", "DEMO-9"]
    );
    let (s, v) = f
        .put(
            "/rest/agile/1.0/issue/rank",
            json!({ "issues": ["DEMO-9", "NOPE-1"], "rankBeforeIssue": "DEMO-12" }),
        )
        .await;
    assert_eq!(s, StatusCode::MULTI_STATUS);
    assert_keys(&v, &fixture("backlog/rank_partial_207.json"));
    let entries = v["entries"].as_array().unwrap();
    assert_eq!(entries[0]["status"], json!(200));
    assert_eq!(entries[0]["issueId"], json!(20009));
    assert_eq!(entries[1]["status"], json!(404));
    assert!(entries[1]["errors"].is_array());
    let (s, _) = f
        .put(
            "/rest/agile/1.0/issue/rank",
            json!({ "issues": ["DEMO-9"], "rankBeforeIssue": "DEMO-1", "rankAfterIssue": "DEMO-2" }),
        )
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn sprint_lifecycle() {
    let f = Fake::default().await;
    let (s, v) = f
        .post(
            "/rest/agile/1.0/sprint",
            json!({ "name": "DEMO Sprint 6", "originBoardId": 1, "goal": "Objectif" }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    assert_eq!(v["state"], "future");
    assert_keys(
        &v,
        &fixture("sprints/board_sprints_active_future.json")["values"][1],
    );
    let new_id = v["id"].as_u64().unwrap();

    // Mise à jour partielle.
    let (s, v) = f
        .post(
            &format!("/rest/agile/1.0/sprint/{new_id}"),
            json!({ "goal": "Autre" }),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["goal"], "Autre");
    assert_eq!(v["name"], "DEMO Sprint 6");
    let (s, _) = f
        .post(
            &format!("/rest/agile/1.0/sprint/{new_id}"),
            json!({ "bogus": 1 }),
        )
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    // Démarrage refusé : un sprint est déjà actif ; future -> closed interdit.
    let start = fixture("sprints/sprint_start_request.json");
    let (s, v) = f.post("/rest/agile/1.0/sprint/11", start.clone()).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(is_error_body(&v));
    let (s, _) = f
        .post("/rest/agile/1.0/sprint/11", json!({ "state": "closed" }))
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    // Démarrage sans dates -> 400.
    let (s, _) = f
        .post(
            &format!("/rest/agile/1.0/sprint/{new_id}"),
            json!({ "state": "active" }),
        )
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    // Clôture du sprint actif puis démarrage du suivant.
    let (s, v) = f
        .post(
            "/rest/agile/1.0/sprint/10",
            fixture("sprints/sprint_close_request.json"),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_keys(&v, &fixture("sprints/sprint_close_response.json"));
    assert_eq!(v["state"], "closed");
    let (s, v) = f.post("/rest/agile/1.0/sprint/11", start).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_keys(&v, &fixture("sprints/sprint_start_response.json"));
    assert_eq!(v["state"], "active");
    assert_eq!(v["goal"], "Gestion des versions");

    // Réouverture interdite ; les issues non terminées du sprint 10 reviennent au backlog.
    let (s, _) = f
        .post("/rest/agile/1.0/sprint/10", json!({ "state": "active" }))
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let backlog = f.search_keys(BACKLOG_JQL, 50).await;
    assert!(backlog.contains(&"DEMO-2".to_owned()));
    assert!(
        !backlog.contains(&"DEMO-1".to_owned()),
        "DEMO-1 est terminée"
    );

    let (s, v) = f.get("/rest/agile/1.0/sprint/11").await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["state"], "active");
}

// --- Issues -------------------------------------------------------------------------------------

#[tokio::test]
async fn createmeta() {
    let f = Fake::default().await;
    let (s, v) = f.get("/rest/api/3/issue/createmeta/DEMO/issuetypes").await;
    assert_eq!(s, StatusCode::OK);
    let fx = fixture("issues/createmeta_issuetypes.json");
    assert_keys(&v, &fx);
    assert_keys(&v["issueTypes"][0], &fx["issueTypes"][0]);
    assert_eq!(v["total"], json!(3));

    let (s, v) = f
        .get("/rest/api/3/issue/createmeta/DEMO/issuetypes/10001")
        .await;
    assert_eq!(s, StatusCode::OK);
    let fx = fixture("issues/createmeta_fields_story.json");
    assert_keys(&v, &fx);
    assert_keys(&v["fields"][0], &fx["fields"][0]);

    let (s, _) = f.get("/rest/api/3/issue/createmeta/NOPE/issuetypes").await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn create_edit_issue() {
    let f = Fake::default().await;
    let mut req = fixture("issues/create_request.json");
    // Version et sprint du jeu de données du faux serveur.
    req["fields"]["fixVersions"] = json!([{ "id": "101" }]);
    req["fields"]["customfield_10020"] = json!(10);
    let (s, v) = f.post("/rest/api/3/issue", req).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    assert_keys(&v, &fixture("issues/create_response.json"));
    let key = v["key"].as_str().unwrap().to_owned();
    assert_eq!(key, "DEMO-13");
    assert!(
        f.search_keys("sprint = 10", 50).await.contains(&key),
        "créée dans le sprint 10"
    );

    // Erreurs par champ.
    let (s, v) = f
        .post(
            "/rest/api/3/issue",
            json!({ "fields": {
                "project": { "key": "DEMO" }, "issuetype": { "id": "10001" },
                "summary": "", "description": "texte brut", "customfield_10028": 3
            }}),
        )
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert_keys(&v, &fixture("errors/400_field_errors.json"));
    for k in ["summary", "description", "customfield_10028"] {
        assert!(v["errors"].get(k).is_some(), "erreur {k} attendue : {v}");
    }

    // Édition (fixture) : fields + update.fixVersions.
    let (s, v) = f
        .put(
            &format!("/rest/api/3/issue/{key}"),
            fixture("issues/edit_request.json"),
        )
        .await;
    assert_eq!(
        s,
        StatusCode::BAD_REQUEST,
        "version 10100 inconnue du faux serveur : {v}"
    );
    let (s, v) = f
        .put(
            &format!("/rest/api/3/issue/{key}"),
            json!({
                "fields": { "summary": "Nouveau titre", "customfield_10016": 5, "description": null },
                "update": { "fixVersions": [ { "set": [ { "id": "102" } ] } ] }
            }),
        )
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT, "{v}");
    let (s, v) = f.get(&format!("/rest/api/3/issue/{key}")).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["fields"]["summary"], "Nouveau titre");
    assert_eq!(v["fields"]["customfield_10016"], json!(5.0));
    assert!(v["fields"]["description"].is_null());
    assert_eq!(v["fields"]["fixVersions"][0]["id"], "102");

    let (s, v) = f
        .put("/rest/api/3/issue/NOPE-1", json!({ "fields": {} }))
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    assert_keys(&v, &fixture("errors/404.json"));
}

#[tokio::test]
async fn transitions() {
    let f = Fake::default().await;
    // DEMO-4 est « À faire ».
    let (s, v) = f
        .get("/rest/api/3/issue/DEMO-4/transitions?expand=transitions.fields")
        .await;
    assert_eq!(s, StatusCode::OK);
    let fx = fixture("issues/transitions.json");
    assert_keys(&v, &fx);
    assert_keys(&v["transitions"][0], &fx["transitions"][0]);
    let list = v["transitions"].as_array().unwrap();
    let done = list
        .iter()
        .find(|t| t["to"]["statusCategory"]["key"] == "done")
        .unwrap();
    assert_eq!(done["hasScreen"], json!(true));
    assert_eq!(done["fields"]["resolution"]["required"], json!(true));
    let done_id = done["id"].as_str().unwrap().to_owned();
    let doing_id = list.iter().find(|t| t["to"]["name"] == "En cours").unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();

    // « À faire » -> « Terminé » : résolution requise.
    let (s, v) = f
        .post(
            "/rest/api/3/issue/DEMO-4/transitions",
            json!({ "transition": { "id": done_id } }),
        )
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(v["errors"]["resolution"].is_string());

    // Transition inconnue -> 400.
    let (s, _) = f
        .post(
            "/rest/api/3/issue/DEMO-4/transitions",
            json!({ "transition": { "id": "999" } }),
        )
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    // En cours puis Terminé : OK.
    for id in [&doing_id, &done_id] {
        let (s, v) = f
            .post(
                "/rest/api/3/issue/DEMO-4/transitions",
                json!({ "transition": { "id": id } }),
            )
            .await;
        assert_eq!(s, StatusCode::NO_CONTENT, "{v}");
    }
    let (_, v) = f.get("/rest/api/3/issue/DEMO-4?fields=status").await;
    assert_eq!(v["fields"]["status"]["statusCategory"]["key"], "done");

    // Avec résolution fournie : OK directement.
    let (s, _) = f
        .post(
            "/rest/api/3/issue/DEMO-5/transitions",
            json!({ "transition": { "id": done_id }, "fields": { "resolution": { "name": "Terminé" } } }),
        )
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
}

// --- Versions -----------------------------------------------------------------------------------

#[tokio::test]
async fn versions_crud() {
    let f = Fake::default().await;
    let (s, v) = f
        .get("/rest/api/3/project/DEMO/version?maxResults=2&startAt=0")
        .await;
    assert_eq!(s, StatusCode::OK);
    let fx = fixture("versions/project_versions_page.json");
    assert_keys(&v, &fx);
    assert_keys(&v["values"][1], &fx["values"][1]);
    assert_eq!(v["isLast"], json!(false));
    assert_eq!(v["total"], json!(3));
    assert!(v["values"][0]["id"].is_string());
    assert!(v["values"][0]["projectId"].is_u64());
    let (_, v) = f
        .get("/rest/api/3/project/DEMO/version?maxResults=2&startAt=2")
        .await;
    assert_keys(&v, &fixture("versions/project_versions_last.json"));
    assert_eq!(v["isLast"], json!(true));
    let (_, v) = f
        .get("/rest/api/3/project/DEMO/version?status=unreleased&orderBy=-releaseDate")
        .await;
    let names: Vec<&str> = v["values"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["2.0.0", "1.1.0"]);
    let (s, _) = f.get("/rest/api/3/project/NOPE/version").await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    // Création (fixture) puis doublon.
    let req = fixture("versions/version_create_request.json");
    let (s, v) = f.post("/rest/api/3/version", req.clone()).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    assert_keys(&v, &fixture("versions/version_created.json"));
    assert_eq!(v["userReleaseDate"], "28/Feb/27");
    let new_id = v["id"].as_str().unwrap().to_owned();
    let (s, v) = f.post("/rest/api/3/version", req).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(v["errors"]["name"].is_string());

    // Release avec déplacement des issues non terminées vers la nouvelle version.
    let (s, v) = f
        .put(
            "/rest/api/3/version/101",
            json!({
                "released": true, "releaseDate": "2026-12-01",
                "moveUnfixedIssuesTo": format!("{}/rest/api/3/version/{new_id}", f.base)
            }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_keys(&v, &fixture("versions/version_released.json"));
    assert_eq!(v["released"], json!(true));
    // DEMO-1 (terminée) reste sur 1.1.0.
    assert_eq!(
        f.search_keys(
            "project = \"DEMO\" AND fixVersion = 101 ORDER BY Rank ASC",
            50
        )
        .await,
        ["DEMO-1"]
    );
    assert_eq!(
        f.search_keys(&format!("project = DEMO AND fixVersion = {new_id}"), 50)
            .await,
        ["DEMO-2", "DEMO-3", "DEMO-4", "DEMO-5"]
    );

    // Suppression avec report sur 2.0.0.
    let (s, _) = f
        .post(
            &format!("/rest/api/3/version/{new_id}/removeAndSwap"),
            json!({ "moveFixIssuesTo": 102 }),
        )
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, _) = f.get(&format!("/rest/api/3/version/{new_id}")).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    assert!(
        f.search_keys("project = DEMO AND fixVersion = 102", 50)
            .await
            .contains(&"DEMO-2".to_owned())
    );
}

#[tokio::test]
async fn search_validates_reconcile_issues() {
    let f = Fake::default().await;
    let (s, v) = f
        .post(
            "/rest/api/3/search/jql",
            json!({ "jql": BACKLOG_JQL, "reconcileIssues": [20001, 20002] }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let too_many: Vec<u64> = (1..=51).collect();
    for bad in [json!(["DEMO-1"]), json!(too_many), json!("20001")] {
        let (s, v) = f
            .post(
                "/rest/api/3/search/jql",
                json!({ "jql": BACKLOG_JQL, "reconcileIssues": bad }),
            )
            .await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
        assert!(v["errors"]["reconcileIssues"].is_string(), "{v}");
    }
}
