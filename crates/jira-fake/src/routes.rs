//! Handlers HTTP (format Jira Cloud REST v3 / Agile 1.0).

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};

use axum::Router;
use axum::body::Bytes;
use axum::extract::{Path, Query, State as AxState};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post, put};
use serde_json::{Map, Value, json};

use crate::jql::{self, Query as Jql};
use crate::state::{
    Category, F_POINTS, F_SPRINT, FieldSelection, ISSUE_TYPES, Issue, PRIORITIES, RANK_FIELD_ID,
    STATUSES, Sprint, SprintState, State, USERS, Version, status, user,
};
use crate::time::{is_date, now_iso};
use crate::{ApiError, ApiResult, App};

type Params = Query<HashMap<String, String>>;

/// Taille de page maximale des listes paginées par offset.
const OFFSET_MAX: usize = 50;
/// Taille de page maximale de `search/jql` (Jira renvoie moins quand beaucoup de champs sont demandés).
const SEARCH_MAX: usize = 100;
/// Lots agile (sprint/backlog/rank).
const BATCH_MAX: usize = 50;

pub(crate) fn routes() -> Router<App> {
    Router::new()
        // Auth / découverte
        .route("/rest/api/3/myself", get(myself))
        .route("/rest/api/3/field", get(fields))
        // Projets / boards
        .route("/rest/api/3/project/search", get(project_search))
        .route("/rest/agile/1.0/board", get(boards))
        .route(
            "/rest/agile/1.0/board/{id}/configuration",
            get(board_config),
        )
        .route("/rest/agile/1.0/board/{id}/sprint", get(board_sprints))
        // Sprints
        .route("/rest/agile/1.0/sprint", post(create_sprint))
        .route(
            "/rest/agile/1.0/sprint/{id}",
            get(get_sprint).post(update_sprint),
        )
        .route("/rest/agile/1.0/sprint/{id}/issue", post(move_to_sprint))
        // Backlog / rang
        .route("/rest/agile/1.0/backlog/issue", post(move_to_backlog))
        .route("/rest/agile/1.0/issue/rank", put(rank))
        // Issues
        .route("/rest/api/3/search/jql", post(search_post).get(search_get))
        .route("/rest/api/3/issue", post(create_issue))
        .route("/rest/api/3/issue/{key}", get(get_issue).put(edit_issue))
        .route(
            "/rest/api/3/issue/{key}/transitions",
            get(transitions).post(do_transition),
        )
        .route(
            "/rest/api/3/issue/createmeta/{project}/issuetypes",
            get(createmeta_types),
        )
        .route(
            "/rest/api/3/issue/createmeta/{project}/issuetypes/{type_id}",
            get(createmeta_fields),
        )
        // Versions
        .route("/rest/api/3/project/{key}/version", get(project_versions))
        .route("/rest/api/3/version", post(create_version))
        .route(
            "/rest/api/3/version/{id}",
            get(get_version).put(update_version),
        )
        .route(
            "/rest/api/3/version/{id}/removeAndSwap",
            post(remove_and_swap),
        )
}

// --- Utilitaires --------------------------------------------------------------------------------

fn ok(v: Value) -> ApiResult {
    Ok((StatusCode::OK, axum::Json(v)).into_response())
}

fn created(v: Value) -> ApiResult {
    Ok((StatusCode::CREATED, axum::Json(v)).into_response())
}

fn no_content() -> ApiResult {
    Ok(StatusCode::NO_CONTENT.into_response())
}

fn parse_body(body: &Bytes) -> Result<Map<String, Value>, ApiError> {
    if body.iter().all(u8::is_ascii_whitespace) {
        return Err(ApiError::bad("Corps de requête JSON requis."));
    }
    match serde_json::from_slice::<Value>(body) {
        Ok(Value::Object(m)) => Ok(m),
        Ok(_) => Err(ApiError::bad(
            "Le corps de la requête doit être un objet JSON.",
        )),
        Err(e) => Err(ApiError::bad(format!("JSON invalide : {e}"))),
    }
}

/// Id numérique (nombre JSON ou chaîne numérique).
fn as_id(v: &Value) -> Option<u64> {
    match v {
        Value::Number(n) => n.as_u64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn path_id(raw: &str, what: &str) -> Result<u64, ApiError> {
    raw.trim()
        .parse()
        .map_err(|_| ApiError::not_found(format!("{what} « {raw} » introuvable.")))
}

fn param_usize(q: &HashMap<String, String>, name: &str, default: usize) -> Result<usize, ApiError> {
    match q.get(name) {
        None => Ok(default),
        Some(v) => v
            .trim()
            .parse()
            .map_err(|_| ApiError::bad(format!("Paramètre {name} invalide : « {v} »."))),
    }
}

/// `(startAt, maxResults)` d'une pagination offset.
fn offset(q: &HashMap<String, String>) -> Result<(usize, usize), ApiError> {
    let start = param_usize(q, "startAt", 0)?;
    let max = param_usize(q, "maxResults", OFFSET_MAX)?.clamp(1, OFFSET_MAX);
    Ok((start, max))
}

fn slice<T: Clone>(all: &[T], start: usize, max: usize) -> (Vec<T>, bool) {
    let page: Vec<T> = all.iter().skip(start).take(max).cloned().collect();
    let is_last = start + page.len() >= all.len();
    (page, is_last)
}

/// URL d'une page (champ `self` / `nextPage` des PageBean v3).
fn page_url(
    base: &str,
    path: &str,
    start: usize,
    max: usize,
    q: &HashMap<String, String>,
) -> String {
    let mut keys: Vec<&String> = q
        .keys()
        .filter(|k| *k != "startAt" && *k != "maxResults")
        .collect();
    keys.sort();
    let mut url = format!("{base}{path}?maxResults={max}&startAt={start}");
    for k in keys {
        url.push_str(&format!("&{k}={}", q[k]));
    }
    url
}

/// Liste `issues` d'un corps agile : non vide, au plus 50 entrées.
fn issue_list(body: &Map<String, Value>) -> Result<Vec<String>, ApiError> {
    let list = body
        .get("issues")
        .and_then(Value::as_array)
        .ok_or_else(|| ApiError::field("issues", "Le champ « issues » (tableau) est requis."))?;
    if list.is_empty() {
        return Err(ApiError::field("issues", "Au moins une issue est requise."));
    }
    if list.len() > BATCH_MAX {
        return Err(ApiError::field(
            "issues",
            format!(
                "Au plus {BATCH_MAX} issues par requête ({} reçues).",
                list.len()
            ),
        ));
    }
    list.iter()
        .map(|v| match v {
            Value::String(s) => Ok(s.clone()),
            Value::Number(n) => Ok(n.to_string()),
            _ => Err(ApiError::field("issues", "Clés ou ids d'issue attendus.")),
        })
        .collect()
}

fn check_keys(body: &Map<String, Value>, allowed: &[&str]) -> Result<(), ApiError> {
    match body.keys().find(|k| !allowed.contains(&k.as_str())) {
        Some(k) => Err(ApiError::bad(format!(
            "Unrecognized field \"{k}\" (champ non autorisé dans ce corps)."
        ))),
        None => Ok(()),
    }
}

fn opt_str<'a>(body: &'a Map<String, Value>, key: &str) -> Result<Option<&'a str>, ApiError> {
    match body.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s)),
        Some(_) => Err(ApiError::field(key, "Chaîne attendue.")),
    }
}

fn is_datetime(s: &str) -> bool {
    s.len() >= 10 && s.is_char_boundary(10) && is_date(&s[..10])
}

// --- Auth / champs ------------------------------------------------------------------------------

async fn myself(AxState(app): AxState<App>) -> ApiResult {
    let st = app.lock();
    let mut v = st.user_json(&USERS[0]);
    v["locale"] = json!("fr_FR");
    v["groups"] = json!({ "size": 2, "items": [] });
    v["applicationRoles"] = json!({ "size": 1, "items": [] });
    v["expand"] = json!("groups,applicationRoles");
    ok(v)
}

fn field_def(
    id: &str,
    name: &str,
    custom: Option<(&str, u64)>,
    schema: Value,
    clauses: &[&str],
) -> Value {
    let mut schema = schema;
    if let Some((kind, cid)) = custom {
        schema["custom"] = json!(kind);
        schema["customId"] = json!(cid);
    } else {
        schema["system"] = json!(id);
    }
    json!({
        "id": id, "key": id, "name": name, "untranslatedName": name,
        "custom": custom.is_some(), "orderable": true, "navigable": true, "searchable": true,
        "clauseNames": clauses, "schema": schema
    })
}

async fn fields() -> ApiResult {
    const GH: &str = "com.pyxis.greenhopper.jira";
    const FLOAT: &str = "com.atlassian.jira.plugin.system.customfieldtypes:float";
    let v = vec![
        field_def(
            "summary",
            "Summary",
            None,
            json!({"type": "string"}),
            &["summary"],
        ),
        field_def(
            "description",
            "Description",
            None,
            json!({"type": "string"}),
            &["description"],
        ),
        field_def(
            "status",
            "Status",
            None,
            json!({"type": "status"}),
            &["status"],
        ),
        field_def(
            "issuetype",
            "Issue Type",
            None,
            json!({"type": "issuetype"}),
            &["issuetype", "type"],
        ),
        field_def(
            "priority",
            "Priority",
            None,
            json!({"type": "priority"}),
            &["priority"],
        ),
        field_def(
            "assignee",
            "Assignee",
            None,
            json!({"type": "user"}),
            &["assignee"],
        ),
        field_def(
            "project",
            "Project",
            None,
            json!({"type": "project"}),
            &["project"],
        ),
        field_def(
            "fixVersions",
            "Fix versions",
            None,
            json!({"type": "array", "items": "version"}),
            &["fixVersion"],
        ),
        field_def(
            F_SPRINT,
            "Sprint",
            Some((&format!("{GH}:gh-sprint"), 10020)),
            json!({"type": "array", "items": "json"}),
            &["cf[10020]", "Sprint", "sprint"],
        ),
        field_def(
            F_POINTS,
            "Story point estimate",
            Some((&format!("{GH}:jsw-story-points"), 10016)),
            json!({"type": "number"}),
            &["cf[10016]", "Story point estimate"],
        ),
        field_def(
            "customfield_10028",
            "Story Points",
            Some((FLOAT, 10028)),
            json!({"type": "number"}),
            &["cf[10028]", "Story Points"],
        ),
        field_def(
            "customfield_10019",
            "Rank",
            Some((&format!("{GH}:gh-lexo-rank"), RANK_FIELD_ID)),
            json!({"type": "any"}),
            &["cf[10019]", "Rank"],
        ),
        field_def(
            "customfield_10050",
            "Estimation interne",
            Some((FLOAT, 10050)),
            json!({"type": "number"}),
            &["cf[10050]", "Estimation interne"],
        ),
    ];
    ok(Value::Array(v))
}

// --- Projets / boards ---------------------------------------------------------------------------

async fn project_search(AxState(app): AxState<App>, Query(q): Params) -> ApiResult {
    let (start, max) = offset(&q)?;
    let st = app.lock();
    let needle = q.get("query").map(|s| s.to_lowercase());
    let keys: Option<Vec<String>> = q
        .get("keys")
        .map(|k| k.split(',').map(|s| s.trim().to_uppercase()).collect());
    let type_ok = q.get("typeKey").is_none_or(|t| t == "software");
    let all: Vec<Value> = st
        .projects
        .iter()
        .filter(|_| type_ok)
        .filter(|p| {
            needle.as_ref().is_none_or(|n| {
                p.key.to_lowercase().contains(n) || p.name.to_lowercase().contains(n)
            })
        })
        .filter(|p| keys.as_ref().is_none_or(|k| k.contains(&p.key)))
        .map(|p| st.project_json(p))
        .collect();
    let (values, is_last) = slice(&all, start, max);
    let path = "/rest/api/3/project/search";
    let mut out = json!({
        "self": page_url(&st.base, path, start, max, &q),
        "maxResults": max, "startAt": start, "total": all.len(), "isLast": is_last,
        "values": values,
    });
    if !is_last {
        out["nextPage"] = json!(page_url(&st.base, path, start + max, max, &q));
    }
    ok(out)
}

async fn boards(AxState(app): AxState<App>, Query(q): Params) -> ApiResult {
    let (start, max) = offset(&q)?;
    let st = app.lock();
    let types_ok = q
        .get("type")
        .is_none_or(|t| t.split(',').any(|t| t.trim() == "scrum"));
    let project = q.get("projectKeyOrId").map(|p| p.trim().to_owned());
    if let Some(p) = &project
        && st.project(p).is_none()
    {
        return Err(ApiError::bad(format!(
            "No project could be found with key or id '{p}'."
        )));
    }
    let all: Vec<Value> = st
        .boards
        .iter()
        .filter(|_| types_ok)
        .filter(|b| {
            project
                .as_ref()
                .is_none_or(|p| st.project(p).is_some_and(|p| p.key == b.project_key))
        })
        .map(|b| st.board_json(b))
        .collect();
    let (values, is_last) = slice(&all, start, max);
    ok(json!({
        "maxResults": max, "startAt": start, "total": all.len(), "isLast": is_last, "values": values
    }))
}

async fn board_config(AxState(app): AxState<App>, Path(id): Path<String>) -> ApiResult {
    let st = app.lock();
    let b = path_id(&id, "Board")
        .ok()
        .and_then(|id| st.board(id))
        .ok_or_else(|| {
            ApiError::not_found(format!(
                "Board {id} does not exist or you do not have permission to see it."
            ))
        })?;
    let pid = st.project(&b.project_key).map_or(0, |p| p.id);
    let base = &st.base;
    let col = |name: &str, ids: &[&str]| {
        let statuses: Vec<Value> = ids
            .iter()
            .map(|s| json!({ "id": s, "self": format!("{base}/rest/api/2/status/{s}") }))
            .collect();
        json!({ "name": name, "statuses": statuses })
    };
    ok(json!({
        "id": b.id,
        "name": b.name,
        "type": "scrum",
        "self": format!("{base}/rest/agile/1.0/board/{}/configuration", b.id),
        "location": { "type": "project", "id": pid.to_string() },
        "filter": { "id": b.filter_id.to_string(), "self": format!("{base}/rest/api/2/filter/{}", b.filter_id) },
        "columnConfig": {
            "columns": [col("À faire", &["1"]), col("En cours", &["3", "4"]), col("Terminé", &["5"])],
            "constraintType": "issueCount"
        },
        "estimation": { "type": "field", "field": { "fieldId": F_POINTS, "displayName": "Story point estimate" } },
        "ranking": { "rankCustomFieldId": RANK_FIELD_ID }
    }))
}

async fn board_sprints(
    AxState(app): AxState<App>,
    Path(id): Path<String>,
    Query(q): Params,
) -> ApiResult {
    let (start, max) = offset(&q)?;
    let st = app.lock();
    let board = path_id(&id, "Board")
        .ok()
        .and_then(|id| st.board(id))
        .ok_or_else(|| {
            ApiError::not_found(format!(
                "Board {id} does not exist or you do not have permission to see it."
            ))
        })?;
    let states: Option<Vec<SprintState>> = match q.get("state") {
        None => None,
        Some(csv) => Some(
            csv.split(',')
                .filter(|s| !s.trim().is_empty())
                .map(|s| {
                    SprintState::parse(s).ok_or_else(|| {
                        ApiError::bad(format!(
                            "Valeur de state invalide : « {s} » (future, active, closed)."
                        ))
                    })
                })
                .collect::<Result<_, _>>()?,
        ),
    };
    let all: Vec<Value> = st
        .sprints
        .iter()
        .filter(|s| s.origin_board_id == board.id)
        .filter(|s| {
            states
                .as_ref()
                .is_none_or(|w| w.is_empty() || w.contains(&s.state))
        })
        .map(|s| st.sprint_json(s))
        .collect();
    let (values, is_last) = slice(&all, start, max);
    ok(json!({ "maxResults": max, "startAt": start, "isLast": is_last, "values": values }))
}

// --- Sprints ------------------------------------------------------------------------------------

fn sprint_not_found(id: &str) -> ApiError {
    ApiError::not_found(format!(
        "Sprint {id} does not exist or you do not have permission to see it."
    ))
}

fn sprint_index(st: &State, raw: &str) -> Result<usize, ApiError> {
    let id = path_id(raw, "Sprint").map_err(|_| sprint_not_found(raw))?;
    st.sprints
        .iter()
        .position(|s| s.id == id)
        .ok_or_else(|| sprint_not_found(raw))
}

fn validate_sprint_name(name: &str) -> Result<(), ApiError> {
    if name.trim().is_empty() {
        return Err(ApiError::field("name", "Le nom du sprint est requis."));
    }
    if name.chars().count() > 30 {
        return Err(ApiError::field(
            "name",
            "Sprint name too long (30 caractères maximum).",
        ));
    }
    Ok(())
}

fn validate_dates(start: Option<&str>, end: Option<&str>) -> Result<(), ApiError> {
    for (k, v) in [("startDate", start), ("endDate", end)] {
        if let Some(v) = v
            && !is_datetime(v)
        {
            return Err(ApiError::field(k, format!("Date invalide : « {v} »")));
        }
    }
    if let (Some(s), Some(e)) = (start, end)
        && e < s
    {
        return Err(ApiError::field(
            "endDate",
            "La date de fin doit être postérieure à la date de début.",
        ));
    }
    Ok(())
}

async fn get_sprint(AxState(app): AxState<App>, Path(id): Path<String>) -> ApiResult {
    let st = app.lock();
    let i = sprint_index(&st, &id)?;
    ok(st.sprint_json(&st.sprints[i]))
}

async fn create_sprint(AxState(app): AxState<App>, body: Bytes) -> ApiResult {
    let body = parse_body(&body)?;
    check_keys(
        &body,
        &["name", "originBoardId", "goal", "startDate", "endDate"],
    )?;
    let mut st = app.lock();
    let name = opt_str(&body, "name")?.unwrap_or_default().to_owned();
    validate_sprint_name(&name)?;
    let board = body
        .get("originBoardId")
        .and_then(as_id)
        .ok_or_else(|| ApiError::field("originBoardId", "originBoardId est requis."))?;
    if st.board(board).is_none() {
        return Err(ApiError::field(
            "originBoardId",
            format!("Board {board} introuvable."),
        ));
    }
    let start = opt_str(&body, "startDate")?.map(str::to_owned);
    let end = opt_str(&body, "endDate")?.map(str::to_owned);
    validate_dates(start.as_deref(), end.as_deref())?;
    let id = st.next_sprint_id;
    st.next_sprint_id += 1;
    let sprint = Sprint {
        id,
        name,
        state: SprintState::Future,
        goal: opt_str(&body, "goal")?.unwrap_or_default().to_owned(),
        start_date: start,
        end_date: end,
        complete_date: None,
        created_date: now_iso(),
        origin_board_id: board,
    };
    let v = st.sprint_json(&sprint);
    st.sprints.push(sprint);
    created(v)
}

async fn update_sprint(
    AxState(app): AxState<App>,
    Path(id): Path<String>,
    body: Bytes,
) -> ApiResult {
    let body = parse_body(&body)?;
    check_keys(
        &body,
        &[
            "name",
            "goal",
            "state",
            "startDate",
            "endDate",
            "id",
            "self",
            "originBoardId",
            "createdDate",
            "completeDate",
        ],
    )?;
    let mut st = app.lock();
    let idx = sprint_index(&st, &id)?;
    let current = st.sprints[idx].clone();
    let name = opt_str(&body, "name")?.map(str::to_owned);
    if let Some(n) = &name {
        validate_sprint_name(n)?;
    }
    let goal = match body.get("goal") {
        None => None,
        Some(Value::Null) => Some(String::new()),
        Some(Value::String(s)) => Some(s.clone()),
        Some(_) => return Err(ApiError::field("goal", "Chaîne attendue.")),
    };
    let new_state = match opt_str(&body, "state")? {
        None => None,
        Some(s) => Some(
            SprintState::parse(s)
                .ok_or_else(|| ApiError::field("state", format!("État inconnu : « {s} »")))?,
        ),
    };

    let mut next = current.clone();
    if let Some(n) = name {
        next.name = n;
    }
    if let Some(g) = goal {
        next.goal = g;
    }

    if current.state == SprintState::Closed {
        // Sprint clôturé : seuls name et goal sont modifiables, le reste est ignoré.
        if new_state.is_some_and(|s| s != SprintState::Closed) {
            return Err(ApiError::bad("Un sprint clôturé ne peut pas être rouvert."));
        }
    } else {
        let start = opt_str(&body, "startDate")?.map(str::to_owned);
        let end = opt_str(&body, "endDate")?.map(str::to_owned);
        if start.is_some() {
            next.start_date = start;
        }
        if end.is_some() {
            next.end_date = end;
        }
        validate_dates(next.start_date.as_deref(), next.end_date.as_deref())?;
        match (current.state, new_state) {
            (_, None) => {}
            (a, Some(b)) if a == b => {}
            (SprintState::Future, Some(SprintState::Active)) => {
                if next.start_date.is_none() || next.end_date.is_none() {
                    return Err(ApiError::bad(
                        "Le sprint doit avoir une date de début et une date de fin pour être démarré.",
                    ));
                }
                if st.sprints.iter().any(|s| {
                    s.origin_board_id == current.origin_board_id && s.state == SprintState::Active
                }) {
                    return Err(ApiError::bad(
                        "Un sprint est déjà actif sur ce board (sprints parallèles désactivés).",
                    ));
                }
                next.state = SprintState::Active;
            }
            (SprintState::Active, Some(SprintState::Closed)) => {
                next.state = SprintState::Closed;
                next.complete_date = Some(now_iso());
            }
            (a, Some(b)) => {
                return Err(ApiError::bad(format!(
                    "Transition d'état de sprint interdite : {} -> {}.",
                    a.as_str(),
                    b.as_str()
                )));
            }
        }
    }
    let v = st.sprint_json(&next);
    st.sprints[idx] = next;
    ok(v)
}

/// Retire les sprints ouverts de l'issue (les sprints clôturés restent dans l'historique).
fn drop_open_sprints(st: &mut State, idx: usize) {
    let open: Vec<u64> = st.issues[idx]
        .sprints
        .iter()
        .copied()
        .filter(|s| st.sprint_is_open(*s))
        .collect();
    st.issues[idx].sprints.retain(|s| !open.contains(s));
}

fn resolve_issues(st: &State, keys: &[String]) -> Result<Vec<usize>, ApiError> {
    keys.iter()
        .map(|k| {
            st.issue_index(k).ok_or_else(|| {
                ApiError::bad(format!(
                    "Issue '{k}' does not exist or you do not have permission to see it."
                ))
            })
        })
        .collect()
}

async fn move_to_sprint(
    AxState(app): AxState<App>,
    Path(id): Path<String>,
    body: Bytes,
) -> ApiResult {
    let body = parse_body(&body)?;
    check_keys(
        &body,
        &[
            "issues",
            "rankBeforeIssue",
            "rankAfterIssue",
            "rankCustomFieldId",
        ],
    )?;
    let keys = issue_list(&body)?;
    let mut st = app.lock();
    let sidx = sprint_index(&st, &id)?;
    let sprint = st.sprints[sidx].clone();
    if sprint.state == SprintState::Closed {
        return Err(ApiError::bad(format!(
            "Le sprint {} est clôturé : impossible d'y ajouter des issues.",
            sprint.id
        )));
    }
    let idxs = resolve_issues(&st, &keys)?;
    let before = opt_str(&body, "rankBeforeIssue")?.map(str::to_owned);
    let after = opt_str(&body, "rankAfterIssue")?.map(str::to_owned);
    if before.is_some() && after.is_some() {
        return Err(ApiError::bad(
            "rankBeforeIssue et rankAfterIssue sont exclusifs.",
        ));
    }
    for i in idxs {
        drop_open_sprints(&mut st, i);
        st.issues[i].sprints.push(sprint.id);
    }
    if before.is_some() || after.is_some() {
        let entries = apply_rank(&mut st, &keys, before.as_deref(), after.as_deref())?;
        if entries.iter().any(|e| e.failed) {
            return Err(ApiError::bad("Rang non appliqué pour certaines issues."));
        }
    }
    no_content()
}

async fn move_to_backlog(AxState(app): AxState<App>, body: Bytes) -> ApiResult {
    let body = parse_body(&body)?;
    check_keys(&body, &["issues"])?;
    let keys = issue_list(&body)?;
    let mut st = app.lock();
    for i in resolve_issues(&st, &keys)? {
        drop_open_sprints(&mut st, i);
    }
    no_content()
}

struct RankEntry {
    id: Option<u64>,
    key: String,
    failed: bool,
    status: u16,
    error: Option<String>,
}

/// Place `keys` avant/après l'ancre ; les issues inconnues (ou l'ancre elle-même) échouent
/// individuellement (réponse 207).
fn apply_rank(
    st: &mut State,
    keys: &[String],
    before: Option<&str>,
    after: Option<&str>,
) -> Result<Vec<RankEntry>, ApiError> {
    let (anchor, is_before) = match (before, after) {
        (Some(b), None) => (b, true),
        (None, Some(a)) => (a, false),
        _ => {
            return Err(ApiError::bad(
                "Indiquer exactement un de rankBeforeIssue / rankAfterIssue.",
            ));
        }
    };
    let anchor_id = st
        .issue_index(anchor)
        .map(|i| st.issues[i].id)
        .ok_or_else(|| {
            ApiError::bad(format!(
                "Issue '{anchor}' does not exist or you do not have permission to see it."
            ))
        })?;
    let mut entries = Vec::new();
    let mut moved: Vec<Issue> = Vec::new();
    for k in keys {
        match st.issue_index(k) {
            None => entries.push(RankEntry {
                id: None,
                key: k.clone(),
                failed: true,
                status: 404,
                error: Some(format!("Issue '{k}' does not exist.")),
            }),
            Some(i) if st.issues[i].id == anchor_id => entries.push(RankEntry {
                id: Some(anchor_id),
                key: st.issues[i].key.clone(),
                failed: true,
                status: 400,
                error: Some("Cannot rank an issue relative to itself.".into()),
            }),
            Some(i) => {
                let issue = st.issues.remove(i);
                entries.push(RankEntry {
                    id: Some(issue.id),
                    key: issue.key.clone(),
                    failed: false,
                    status: 200,
                    error: None,
                });
                moved.push(issue);
            }
        }
    }
    let pos = st
        .issues
        .iter()
        .position(|i| i.id == anchor_id)
        .unwrap_or(st.issues.len());
    let at = if is_before { pos } else { pos + 1 };
    st.issues.splice(at..at, moved);
    Ok(entries)
}

async fn rank(AxState(app): AxState<App>, body: Bytes) -> ApiResult {
    let body = parse_body(&body)?;
    check_keys(
        &body,
        &[
            "issues",
            "rankBeforeIssue",
            "rankAfterIssue",
            "rankCustomFieldId",
        ],
    )?;
    let keys = issue_list(&body)?;
    if let Some(v) = body.get("rankCustomFieldId")
        && !v.is_null()
        && as_id(v) != Some(RANK_FIELD_ID)
    {
        return Err(ApiError::field(
            "rankCustomFieldId",
            format!("Champ de rang inconnu : {v}."),
        ));
    }
    let before = opt_str(&body, "rankBeforeIssue")?;
    let after = opt_str(&body, "rankAfterIssue")?;
    let mut st = app.lock();
    let entries = apply_rank(&mut st, &keys, before, after)?;
    if !entries.iter().any(|e| e.failed) {
        return no_content();
    }
    let list: Vec<Value> = entries
        .into_iter()
        .map(|e| {
            let mut o = json!({ "issueKey": e.key, "status": e.status });
            if let Some(id) = e.id {
                o["issueId"] = json!(id);
            }
            if let Some(err) = e.error {
                o["errors"] = json!([err]);
            }
            o
        })
        .collect();
    Ok((
        StatusCode::MULTI_STATUS,
        axum::Json(json!({ "entries": list })),
    )
        .into_response())
}

// --- Recherche JQL ------------------------------------------------------------------------------

fn jql_hash(jql: &str) -> u64 {
    let mut h = DefaultHasher::new();
    jql.hash(&mut h);
    h.finish()
}

fn encode_token(jql: &str, offset: usize) -> String {
    format!("{:016x}{:08x}", jql_hash(jql), offset)
}

fn decode_token(jql: &str, token: &str) -> Option<usize> {
    if token.len() != 24 || !token.is_ascii() {
        return None;
    }
    let (h, o) = token.split_at(16);
    (u64::from_str_radix(h, 16).ok()? == jql_hash(jql))
        .then(|| usize::from_str_radix(o, 16).ok())
        .flatten()
}

fn jql_error(msg: impl Into<String>) -> ApiError {
    ApiError::bad(msg)
}

fn run_jql(st: &State, jql: &str) -> Result<Vec<usize>, ApiError> {
    let q = jql::parse(jql).ok_or_else(|| {
        jql_error(format!(
            "jira-fake : JQL non supportée : « {jql} ». Requêtes acceptées : {}",
            jql::SUPPORTED
        ))
    })?;
    let matches = |pred: &dyn Fn(&Issue) -> bool| -> Vec<usize> {
        st.issues
            .iter()
            .enumerate()
            .filter(|(_, i)| pred(i))
            .map(|(n, _)| n)
            .collect()
    };
    match q {
        Jql::Sprint(id) => {
            if st.sprint(id).is_none() {
                return Err(jql_error(format!(
                    "Sprint with id {id} does not exist or you do not have permission to see it."
                )));
            }
            Ok(matches(&|i| i.sprints.contains(&id)))
        }
        Jql::Backlog {
            filter: fid,
            exclude_future,
            // Aucun type sous-tâche dans le jeu de données : la clause est sans effet.
            exclude_subtasks: _,
        } => {
            let board = st
                .boards
                .iter()
                .find(|b| b.filter_id == fid)
                .ok_or_else(|| {
                    jql_error(format!(
                        "A filter with id '{fid}' does not exist or you do not have permission to see it."
                    ))
                })?;
            Ok(matches(&|i| {
                i.project_key == board.project_key
                    && !i.sprints.iter().any(|s| {
                        st.sprint_is_active(*s) || (exclude_future && st.sprint_is_future(*s))
                    })
                    && status(i.status_id).is_none_or(|s| s.category != Category::Done)
            }))
        }
        Jql::Version { project, version } => {
            let p = st.project(&project).ok_or_else(|| {
                jql_error(format!(
                    "The value '{project}' does not exist for the field 'project'."
                ))
            })?;
            if st.version(version).is_none_or(|v| v.project_id != p.id) {
                return Err(jql_error(format!(
                    "The value '{version}' does not exist for the field 'fixVersion'."
                )));
            }
            let key = p.key.clone();
            Ok(matches(&|i| {
                i.project_key == key && i.fix_versions.contains(&version)
            }))
        }
    }
}

fn search(
    st: &State,
    jql: Option<&str>,
    fields: FieldSelection,
    max: Option<usize>,
    token: Option<&str>,
) -> ApiResult {
    let jql = jql
        .map(str::trim)
        .filter(|j| !j.is_empty())
        .ok_or_else(|| jql_error("Unbounded JQL queries are not allowed here. Please add a search restriction to your query."))?;
    let hits = run_jql(st, jql)?;
    let start = match token.filter(|t| !t.is_empty()) {
        None => 0,
        Some(t) => decode_token(jql, t)
            .ok_or_else(|| jql_error("The provided next page token is invalid or expired."))?,
    };
    let max = max.unwrap_or(50).clamp(1, SEARCH_MAX);
    let (page, is_last) = slice(&hits, start, max);
    let issues: Vec<Value> = page.iter().map(|i| st.issue_json(*i, &fields)).collect();
    let mut out = json!({ "issues": issues, "isLast": is_last });
    if !is_last {
        out["nextPageToken"] = json!(encode_token(jql, start + page.len()));
    }
    ok(out)
}

async fn search_post(AxState(app): AxState<App>, body: Bytes) -> ApiResult {
    let body = parse_body(&body)?;
    let jql = opt_str(&body, "jql")?;
    let fields = match body.get("fields") {
        None | Some(Value::Null) => FieldSelection::default(),
        Some(Value::Array(a)) => FieldSelection::parse(a.iter().filter_map(Value::as_str)),
        Some(Value::String(s)) => FieldSelection::parse(s.split(',')),
        Some(_) => return Err(ApiError::field("fields", "Tableau de chaînes attendu.")),
    };
    let max = match body.get("maxResults") {
        None | Some(Value::Null) => None,
        Some(v) => Some(
            v.as_u64()
                .ok_or_else(|| ApiError::field("maxResults", "Entier attendu."))?
                as usize,
        ),
    };
    let token = opt_str(&body, "nextPageToken")?;
    // Cohérence immédiate dans le faux serveur : `reconcileIssues` est seulement validé
    // (tableau d'au plus 50 ids numériques, comme Jira).
    match body.get("reconcileIssues") {
        None | Some(Value::Null) => {}
        Some(Value::Array(ids)) if ids.len() <= 50 && ids.iter().all(|v| v.as_u64().is_some()) => {}
        Some(_) => {
            return Err(ApiError::field(
                "reconcileIssues",
                "Tableau d'au plus 50 identifiants d'issue numériques attendu.",
            ));
        }
    }
    let st = app.lock();
    search(&st, jql, fields, max, token)
}

async fn search_get(AxState(app): AxState<App>, Query(q): Params) -> ApiResult {
    let fields = q
        .get("fields")
        .map(|f| FieldSelection::parse(f.split(',')))
        .unwrap_or_default();
    let max = match q.get("maxResults") {
        None => None,
        Some(_) => Some(param_usize(&q, "maxResults", 50)?),
    };
    let st = app.lock();
    search(
        &st,
        q.get("jql").map(String::as_str),
        fields,
        max,
        q.get("nextPageToken").map(String::as_str),
    )
}

// --- Issues -------------------------------------------------------------------------------------

fn issue_not_found(key: &str) -> ApiError {
    ApiError::not_found(format!(
        "Issue {key} does not exist or you do not have permission to see it."
    ))
}

const NOT_ON_SCREEN: &str = "cannot be set. It is not on the appropriate screen, or unknown.";

/// Applique les champs (`fields` de création/édition) à une issue ; erreurs par champ cumulées.
fn apply_fields(
    st: &State,
    issue: &mut Issue,
    fields: &Map<String, Value>,
    creating: bool,
    errors: &mut Map<String, Value>,
) {
    let project_id = st.project(&issue.project_key).map_or(0, |p| p.id);
    let mut err = |k: &str, m: String| {
        errors.insert(k.to_owned(), json!(m));
    };
    for (k, v) in fields {
        match k.as_str() {
            "project" if creating => {}
            "issuetype" => {
                let id = v.get("id").and_then(as_id).map(|i| i.to_string()).or_else(|| {
                    let name = v.get("name")?.as_str()?;
                    ISSUE_TYPES
                        .iter()
                        .find(|t| t.1.eq_ignore_ascii_case(name))
                        .map(|t| t.0.to_owned())
                });
                match id.filter(|id| ISSUE_TYPES.iter().any(|t| t.0 == id)) {
                    Some(id) => issue.type_id = id,
                    None => err(k, "Specify a valid issue type".into()),
                }
            }
            "summary" => match v.as_str().map(str::trim) {
                Some(s) if !s.is_empty() && s.chars().count() <= 255 => {
                    issue.summary = s.to_owned()
                }
                Some(s) if !s.is_empty() => {
                    err(k, "Summary must be less than 255 characters.".into())
                }
                _ => err(k, "You must specify a summary of the issue.".into()),
            },
            "description" => match v {
                Value::Null => issue.description = None,
                Value::Object(o)
                    if o.get("type").and_then(Value::as_str) == Some("doc")
                        && o.get("version").and_then(Value::as_u64) == Some(1)
                        && o.get("content").is_some_and(Value::is_array) =>
                {
                    issue.description = Some(v.clone())
                }
                _ => err(
                    k,
                    "Operation value must be an Atlassian Document (see the Atlassian Document Format)"
                        .into(),
                ),
            },
            "assignee" => match v {
                Value::Null => issue.assignee = None,
                _ => match v.get("accountId").and_then(Value::as_str).and_then(user) {
                    Some(u) => issue.assignee = Some(u.account_id.to_owned()),
                    None => err(k, "Specified user does not exist or you do not have required permissions".into()),
                },
            },
            "priority" => match v {
                Value::Null => issue.priority_id = None,
                _ => match v
                    .get("id")
                    .and_then(as_id)
                    .map(|i| i.to_string())
                    .filter(|id| PRIORITIES.iter().any(|p| p.0 == id))
                {
                    Some(id) => issue.priority_id = Some(id),
                    None => err(k, "Specify a valid priority id".into()),
                },
            },
            "fixVersions" => match v.as_array() {
                Some(list) => match version_ids(st, project_id, list) {
                    Ok(ids) => issue.fix_versions = ids,
                    Err(m) => err(k, m),
                },
                None => err(k, "Tableau de versions attendu.".into()),
            },
            F_POINTS => match v {
                Value::Null => issue.points = None,
                Value::Number(n) => issue.points = n.as_f64(),
                _ => err(k, "Operation value must be a number".into()),
            },
            F_SPRINT => match v {
                Value::Null => {
                    issue.sprints.retain(|s| !st.sprint_is_open(*s));
                }
                _ => match as_id(v).and_then(|id| st.sprint(id)) {
                    Some(s) if s.state != SprintState::Closed => {
                        issue.sprints.retain(|x| !st.sprint_is_open(*x));
                        issue.sprints.push(s.id);
                    }
                    Some(_) => err(k, "Impossible d'affecter un sprint clôturé.".into()),
                    None => err(k, "Number value expected as the Sprint id.".into()),
                },
            },
            _ => err(k, format!("Field '{k}' {NOT_ON_SCREEN}")),
        }
    }
}

fn version_ids(st: &State, project_id: u64, list: &[Value]) -> Result<Vec<u64>, String> {
    let mut out = Vec::new();
    for v in list {
        let found = match (
            v.get("id").and_then(as_id),
            v.get("name").and_then(Value::as_str),
        ) {
            (Some(id), _) => st.version(id).filter(|x| x.project_id == project_id),
            (None, Some(n)) => st
                .versions
                .iter()
                .find(|x| x.project_id == project_id && x.name == n),
            _ => None,
        };
        match found {
            Some(x) if !out.contains(&x.id) => out.push(x.id),
            Some(_) => {}
            None => return Err(format!("Version {v} introuvable dans ce projet.")),
        }
    }
    Ok(out)
}

async fn create_issue(AxState(app): AxState<App>, body: Bytes) -> ApiResult {
    let body = parse_body(&body)?;
    let fields = body
        .get("fields")
        .and_then(Value::as_object)
        .ok_or_else(|| ApiError::bad("Le corps doit contenir « fields »."))?;
    let mut st = app.lock();
    let project = fields
        .get("project")
        .and_then(|p| {
            p.get("key")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .or_else(|| p.get("id").and_then(as_id).map(|i| i.to_string()))
        })
        .and_then(|k| st.project(&k).cloned())
        .ok_or_else(|| ApiError::field("project", "Specify a valid project ID or key"))?;
    let mut errors = Map::new();
    if !fields.contains_key("issuetype") {
        errors.insert("issuetype".into(), json!("Specify an issue type"));
    }
    if !fields.contains_key("summary") {
        errors.insert(
            "summary".into(),
            json!("You must specify a summary of the issue."),
        );
    }
    let n = st
        .issues
        .iter()
        .filter(|i| i.project_key == project.key)
        .filter_map(|i| i.key.rsplit_once('-')?.1.parse::<u64>().ok())
        .max()
        .unwrap_or(0)
        + 1;
    let mut issue = Issue {
        id: st.next_issue_id,
        key: format!("{}-{n}", project.key),
        project_key: project.key.clone(),
        summary: String::new(),
        description: None,
        status_id: STATUSES[0].id,
        type_id: String::new(),
        priority_id: Some("3".into()),
        assignee: None,
        points: None,
        fix_versions: vec![],
        sprints: vec![],
    };
    apply_fields(&st, &mut issue, fields, true, &mut errors);
    if !errors.is_empty() {
        return Err(ApiError::fields(errors));
    }
    st.next_issue_id += 1;
    let out = json!({
        "id": issue.id.to_string(),
        "key": issue.key,
        "self": format!("{}/rest/api/3/issue/{}", st.base, issue.id),
    });
    // Nouvelle issue : en bas du classement.
    st.issues.push(issue);
    created(out)
}

async fn get_issue(
    AxState(app): AxState<App>,
    Path(key): Path<String>,
    Query(q): Params,
) -> ApiResult {
    let st = app.lock();
    let i = st.issue_index(&key).ok_or_else(|| issue_not_found(&key))?;
    let fields = q
        .get("fields")
        .map_or_else(FieldSelection::all, |f| FieldSelection::parse(f.split(',')));
    ok(st.issue_json(i, &fields))
}

async fn edit_issue(
    AxState(app): AxState<App>,
    Path(key): Path<String>,
    Query(q): Params,
    body: Bytes,
) -> ApiResult {
    let body = parse_body(&body)?;
    let mut st = app.lock();
    let idx = st.issue_index(&key).ok_or_else(|| issue_not_found(&key))?;
    let empty = Map::new();
    let fields = match body.get("fields") {
        None => &empty,
        Some(Value::Object(f)) => f,
        Some(_) => return Err(ApiError::bad("« fields » doit être un objet.")),
    };
    let update = match body.get("update") {
        None => &empty,
        Some(Value::Object(u)) => u,
        Some(_) => return Err(ApiError::bad("« update » doit être un objet.")),
    };
    let mut errors = Map::new();
    for k in update.keys() {
        if fields.contains_key(k) {
            errors.insert(
                k.clone(),
                json!(format!(
                    "Field '{k}' cannot be set in both fields and update."
                )),
            );
        }
    }
    if fields.contains_key("project") {
        errors.insert(
            "project".into(),
            json!(format!("Field 'project' {NOT_ON_SCREEN}")),
        );
    }
    let mut issue = st.issues[idx].clone();
    let project_id = st.project(&issue.project_key).map_or(0, |p| p.id);
    apply_fields(&st, &mut issue, fields, false, &mut errors);
    for (k, ops) in update {
        if k != "fixVersions" {
            errors.insert(k.clone(), json!(format!("Field '{k}' {NOT_ON_SCREEN}")));
            continue;
        }
        for op in ops.as_array().map(Vec::as_slice).unwrap_or_default() {
            let res = match (op.get("set"), op.get("add"), op.get("remove")) {
                (Some(Value::Array(list)), None, None) => {
                    version_ids(&st, project_id, list).map(|ids| issue.fix_versions = ids)
                }
                (None, Some(v), None) => {
                    version_ids(&st, project_id, std::slice::from_ref(v)).map(|ids| {
                        for id in ids {
                            if !issue.fix_versions.contains(&id) {
                                issue.fix_versions.push(id);
                            }
                        }
                    })
                }
                (None, None, Some(v)) => version_ids(&st, project_id, std::slice::from_ref(v))
                    .map(|ids| issue.fix_versions.retain(|x| !ids.contains(x))),
                _ => Err("Opération attendue : set, add ou remove.".into()),
            };
            if let Err(m) = res {
                errors.insert(k.clone(), json!(m));
            }
        }
    }
    if !errors.is_empty() {
        return Err(ApiError::fields(errors));
    }
    st.issues[idx] = issue;
    if q.get("returnIssue").is_some_and(|v| v == "true") {
        return ok(st.issue_json(idx, &FieldSelection::all()));
    }
    no_content()
}

/// Transition « Terminé » depuis « À faire » : écran avec résolution obligatoire.
fn needs_resolution(from: &str, to: &str) -> bool {
    status(from).is_some_and(|s| s.category == Category::New)
        && status(to).is_some_and(|s| s.category == Category::Done)
}

fn transition_id(status_id: &str) -> String {
    format!("{status_id}1")
}

async fn transitions(
    AxState(app): AxState<App>,
    Path(key): Path<String>,
    Query(q): Params,
) -> ApiResult {
    let st = app.lock();
    let i = st.issue_index(&key).ok_or_else(|| issue_not_found(&key))?;
    let current = st.issues[i].status_id;
    let with_fields = q
        .get("expand")
        .is_some_and(|e| e.split(',').any(|x| x.trim() == "transitions.fields"));
    let list: Vec<Value> = STATUSES
        .iter()
        .filter(|s| s.id != current)
        .map(|s| {
            let screen = needs_resolution(current, s.id);
            let mut t = json!({
                "id": transition_id(s.id),
                "name": s.name,
                "to": st.status_json(s),
                "hasScreen": screen,
                "isGlobal": true,
                "isInitial": false,
                "isAvailable": true,
                "isConditional": false,
                "isLooped": false,
            });
            if with_fields {
                t["fields"] = if screen {
                    json!({ "resolution": {
                        "required": true,
                        "schema": { "type": "resolution", "system": "resolution" },
                        "name": "Résolution", "key": "resolution",
                        "hasDefaultValue": false, "operations": ["set"],
                        "allowedValues": [
                            { "self": format!("{}/rest/api/3/resolution/10000", st.base), "id": "10000", "name": "Terminé" },
                            { "self": format!("{}/rest/api/3/resolution/10001", st.base), "id": "10001", "name": "Abandonné" }
                        ]
                    }})
                } else {
                    json!({})
                };
            }
            t
        })
        .collect();
    ok(json!({ "expand": "transitions", "transitions": list }))
}

async fn do_transition(
    AxState(app): AxState<App>,
    Path(key): Path<String>,
    body: Bytes,
) -> ApiResult {
    let body = parse_body(&body)?;
    let mut st = app.lock();
    let i = st.issue_index(&key).ok_or_else(|| issue_not_found(&key))?;
    let tid = body
        .get("transition")
        .and_then(|t| t.get("id"))
        .and_then(as_id)
        .map(|i| i.to_string())
        .ok_or_else(|| ApiError::bad("Le champ transition.id est requis."))?;
    let current = st.issues[i].status_id;
    let target = STATUSES
        .iter()
        .find(|s| s.id != current && transition_id(s.id) == tid)
        .ok_or_else(|| {
            ApiError::bad(format!(
                "Transition id '{tid}' is not valid for this issue."
            ))
        })?;
    if needs_resolution(current, target.id) {
        let has_resolution = body
            .get("fields")
            .and_then(|f| f.get("resolution"))
            .is_some_and(|r| !r.is_null());
        if !has_resolution {
            return Err(ApiError {
                status: StatusCode::BAD_REQUEST,
                messages: vec![
                    "Résolution requise pour passer à « Terminé » depuis « À faire ».".into(),
                ],
                errors: {
                    let mut m = Map::new();
                    m.insert("resolution".into(), json!("La résolution est requise."));
                    m
                },
            });
        }
    }
    st.issues[i].status_id = target.id;
    no_content()
}

async fn createmeta_types(
    AxState(app): AxState<App>,
    Path(project): Path<String>,
    Query(q): Params,
) -> ApiResult {
    let (start, max) = offset(&q)?;
    let st = app.lock();
    if st.project(&project).is_none() {
        return Err(ApiError::not_found(format!(
            "No project could be found with key or id '{project}'."
        )));
    }
    let all: Vec<Value> = ISSUE_TYPES
        .iter()
        .map(|t| st.issue_type_json(t.0))
        .collect();
    let (values, _) = slice(&all, start, max);
    ok(json!({ "maxResults": max, "startAt": start, "total": all.len(), "issueTypes": values }))
}

async fn createmeta_fields(
    AxState(app): AxState<App>,
    Path((project, type_id)): Path<(String, String)>,
    Query(q): Params,
) -> ApiResult {
    let (start, max) = offset(&q)?;
    let st = app.lock();
    let p = st.project(&project).ok_or_else(|| {
        ApiError::not_found(format!(
            "No project could be found with key or id '{project}'."
        ))
    })?;
    if !ISSUE_TYPES.iter().any(|t| t.0 == type_id) {
        return Err(ApiError::not_found(format!(
            "Issue type {type_id} introuvable pour ce projet."
        )));
    }
    let b = &st.base;
    let f = |id: &str, name: &str, required: bool, ops: &[&str], schema: Value| {
        json!({ "fieldId": id, "key": id, "name": name, "required": required,
                "hasDefaultValue": false, "operations": ops, "schema": schema })
    };
    let mut issuetype = f(
        "issuetype",
        "Issue Type",
        true,
        &[],
        json!({"type": "issuetype", "system": "issuetype"}),
    );
    let mut it = st.issue_type_json(&type_id);
    it.as_object_mut().map(|o| o.remove("hierarchyLevel"));
    issuetype["allowedValues"] = json!([it]);
    let mut priority = f(
        "priority",
        "Priority",
        false,
        &["set"],
        json!({"type": "priority", "system": "priority"}),
    );
    priority["hasDefaultValue"] = json!(true);
    priority["defaultValue"] =
        json!({ "self": format!("{b}/rest/api/3/priority/3"), "id": "3", "name": "Medium" });
    priority["allowedValues"] = PRIORITIES
        .iter()
        .map(|(id, name)| json!({ "self": format!("{b}/rest/api/3/priority/{id}"), "id": id, "name": name }))
        .collect();
    let mut assignee = f(
        "assignee",
        "Assignee",
        false,
        &["set"],
        json!({"type": "user", "system": "assignee"}),
    );
    assignee["autoCompleteUrl"] = json!(format!(
        "{b}/rest/api/3/user/assignable/search?project={}&query=",
        p.key
    ));
    let mut fix = f(
        "fixVersions",
        "Fix versions",
        false,
        &["set", "add", "remove"],
        json!({"type": "array", "items": "version", "system": "fixVersions"}),
    );
    fix["allowedValues"] = st
        .versions
        .iter()
        .filter(|v| v.project_id == p.id && !v.archived)
        .map(|v: &Version| {
            json!({ "self": format!("{b}/rest/api/3/version/{}", v.id), "id": v.id.to_string(),
                    "name": v.name, "archived": v.archived, "released": v.released, "projectId": v.project_id })
        })
        .collect();
    let all = vec![
        f(
            "summary",
            "Summary",
            true,
            &["set"],
            json!({"type": "string", "system": "summary"}),
        ),
        issuetype,
        f(
            "description",
            "Description",
            false,
            &["set"],
            json!({"type": "string", "system": "description"}),
        ),
        priority,
        assignee,
        fix,
        f(
            F_POINTS,
            "Story point estimate",
            false,
            &["set"],
            json!({"type": "number", "custom": "com.pyxis.greenhopper.jira:jsw-story-points", "customId": 10016}),
        ),
        f(
            F_SPRINT,
            "Sprint",
            false,
            &["set"],
            json!({"type": "array", "items": "json", "custom": "com.pyxis.greenhopper.jira:gh-sprint", "customId": 10020}),
        ),
    ];
    let (values, _) = slice(&all, start, max);
    ok(json!({ "maxResults": max, "startAt": start, "total": all.len(), "fields": values }))
}

// --- Versions -----------------------------------------------------------------------------------

fn version_not_found(id: &str) -> ApiError {
    ApiError::not_found(format!("Could not find version for id '{id}'"))
}

fn version_index(st: &State, raw: &str) -> Result<usize, ApiError> {
    let id = path_id(raw, "Version").map_err(|_| version_not_found(raw))?;
    st.versions
        .iter()
        .position(|v| v.id == id)
        .ok_or_else(|| version_not_found(raw))
}

async fn project_versions(
    AxState(app): AxState<App>,
    Path(key): Path<String>,
    Query(q): Params,
) -> ApiResult {
    let (start, max) = offset(&q)?;
    let st = app.lock();
    let p = st.project(&key).ok_or_else(|| {
        ApiError::not_found(format!("No project could be found with key '{key}'."))
    })?;
    let statuses: Option<Vec<String>> = q
        .get("status")
        .map(|s| s.split(',').map(|x| x.trim().to_lowercase()).collect());
    let needle = q.get("query").map(|s| s.to_lowercase());
    let mut list: Vec<&Version> = st
        .versions
        .iter()
        .filter(|v| v.project_id == p.id)
        .filter(|v| {
            statuses.as_ref().is_none_or(|s| {
                (s.iter().any(|x| x == "released") && v.released && !v.archived)
                    || (s.iter().any(|x| x == "unreleased") && !v.released && !v.archived)
                    || (s.iter().any(|x| x == "archived") && v.archived)
            })
        })
        .filter(|v| {
            needle.as_ref().is_none_or(|n| {
                v.name.to_lowercase().contains(n) || v.description.to_lowercase().contains(n)
            })
        })
        .collect();
    if let Some(order) = q.get("orderBy") {
        let (desc, field) = match order.trim() {
            o if o.starts_with('-') => (true, &o[1..]),
            o => (false, o.trim_start_matches('+')),
        };
        let key_of = |v: &Version| -> Option<String> {
            match field {
                "name" => Some(v.name.clone()),
                "description" => Some(v.description.clone()),
                "releaseDate" => v.release_date.clone(),
                "startDate" => v.start_date.clone(),
                _ => None,
            }
        };
        if field != "sequence" {
            if !["name", "description", "releaseDate", "startDate"].contains(&field) {
                return Err(ApiError::bad(format!("orderBy invalide : « {order} »")));
            }
            // Versions sans valeur toujours en dernier.
            list.sort_by(|a, b| match (key_of(a), key_of(b)) {
                (Some(x), Some(y)) if desc => y.cmp(&x),
                (Some(x), Some(y)) => x.cmp(&y),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            });
        } else if desc {
            list.reverse();
        }
    }
    let all: Vec<Value> = list.into_iter().map(|v| st.version_json(v)).collect();
    let (values, is_last) = slice(&all, start, max);
    let path = format!("/rest/api/3/project/{}/version", p.key);
    let mut out = json!({
        "self": page_url(&st.base, &path, start, max, &q),
        "maxResults": max, "startAt": start, "total": all.len(), "isLast": is_last, "values": values,
    });
    if !is_last {
        out["nextPage"] = json!(page_url(&st.base, &path, start + max, max, &q));
    }
    ok(out)
}

async fn get_version(AxState(app): AxState<App>, Path(id): Path<String>) -> ApiResult {
    let st = app.lock();
    let i = version_index(&st, &id)?;
    ok(st.version_json(&st.versions[i]))
}

/// Champs de date d'une version : `YYYY-MM-DD` ou `null` (effacement).
fn version_date(body: &Map<String, Value>, key: &str) -> Result<Option<Option<String>>, ApiError> {
    match body.get(key) {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(None)),
        Some(Value::String(s)) if s.is_empty() => Ok(Some(None)),
        Some(Value::String(s)) if is_date(s) => Ok(Some(Some(s.clone()))),
        Some(v) => Err(ApiError::field(
            key,
            format!("Date invalide : {v} (attendu AAAA-MM-JJ)."),
        )),
    }
}

fn opt_bool(body: &Map<String, Value>, key: &str) -> Result<Option<bool>, ApiError> {
    match body.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(b)) => Ok(Some(*b)),
        Some(_) => Err(ApiError::field(key, "Booléen attendu.")),
    }
}

fn check_version_dates(v: &Version) -> Result<(), ApiError> {
    if let (Some(s), Some(r)) = (&v.start_date, &v.release_date)
        && r < s
    {
        return Err(ApiError::field(
            "releaseDate",
            "La date de publication doit être postérieure à la date de début.",
        ));
    }
    Ok(())
}

fn check_version_name(st: &State, v: &Version) -> Result<(), ApiError> {
    if v.name.trim().is_empty() {
        return Err(ApiError::field(
            "name",
            "You must specify a valid version name",
        ));
    }
    if v.name.chars().count() > 255 {
        return Err(ApiError::field(
            "name",
            "Version name must be less than 255 characters.",
        ));
    }
    if st.versions.iter().any(|x| {
        x.id != v.id && x.project_id == v.project_id && x.name.eq_ignore_ascii_case(&v.name)
    }) {
        return Err(ApiError::field(
            "name",
            "A version with this name already exists in this project.",
        ));
    }
    Ok(())
}

async fn create_version(AxState(app): AxState<App>, body: Bytes) -> ApiResult {
    let body = parse_body(&body)?;
    let mut st = app.lock();
    let project = match (body.get("projectId"), body.get("project")) {
        (Some(id), _) if !id.is_null() => {
            as_id(id).and_then(|id| st.project(&id.to_string()).cloned())
        }
        (_, Some(Value::String(k))) => st.project(k).cloned(),
        _ => {
            return Err(ApiError::field(
                "projectId",
                "Project must be specified to create a version.",
            ));
        }
    }
    .ok_or_else(|| {
        ApiError::not_found("Project not found or you do not have permission to administer it.")
    })?;
    let mut v = Version {
        id: st.next_version_id,
        project_id: project.id,
        name: opt_str(&body, "name")?
            .unwrap_or_default()
            .trim()
            .to_owned(),
        description: opt_str(&body, "description")?
            .unwrap_or_default()
            .to_owned(),
        released: opt_bool(&body, "released")?.unwrap_or(false),
        archived: opt_bool(&body, "archived")?.unwrap_or(false),
        start_date: None,
        release_date: None,
    };
    if let Some(d) = version_date(&body, "startDate")? {
        v.start_date = d;
    }
    if let Some(d) = version_date(&body, "releaseDate")? {
        v.release_date = d;
    }
    check_version_name(&st, &v)?;
    check_version_dates(&v)?;
    st.next_version_id += 1;
    let out = st.version_json(&v);
    st.versions.push(v);
    created(out)
}

async fn update_version(
    AxState(app): AxState<App>,
    Path(id): Path<String>,
    body: Bytes,
) -> ApiResult {
    let body = parse_body(&body)?;
    let mut st = app.lock();
    let idx = version_index(&st, &id)?;
    let mut v = st.versions[idx].clone();
    if let Some(n) = opt_str(&body, "name")? {
        v.name = n.trim().to_owned();
    }
    if body.contains_key("description") {
        v.description = opt_str(&body, "description")?
            .unwrap_or_default()
            .to_owned();
    }
    if let Some(d) = version_date(&body, "startDate")? {
        v.start_date = d;
    }
    if let Some(d) = version_date(&body, "releaseDate")? {
        v.release_date = d;
    }
    if let Some(b) = opt_bool(&body, "released")? {
        v.released = b;
    }
    if let Some(b) = opt_bool(&body, "archived")? {
        v.archived = b;
    }
    check_version_name(&st, &v)?;
    check_version_dates(&v)?;
    // moveUnfixedIssuesTo : URI `self` d'une autre version du projet.
    let target = match opt_str(&body, "moveUnfixedIssuesTo")? {
        None => None,
        Some(uri) => {
            let tid = uri
                .trim_end_matches('/')
                .rsplit('/')
                .next()
                .and_then(|s| s.parse::<u64>().ok())
                .filter(|t| {
                    *t != v.id && st.version(*t).is_some_and(|x| x.project_id == v.project_id)
                })
                .ok_or_else(|| {
                    ApiError::field(
                        "moveUnfixedIssuesTo",
                        format!("Version cible invalide : « {uri} »."),
                    )
                })?;
            Some(tid)
        }
    };
    if let Some(t) = target {
        for issue in st.issues.iter_mut() {
            let done = status(issue.status_id).is_some_and(|s| s.category == Category::Done);
            if !done && issue.fix_versions.contains(&v.id) {
                issue.fix_versions.retain(|x| *x != v.id);
                if !issue.fix_versions.contains(&t) {
                    issue.fix_versions.push(t);
                }
            }
        }
    }
    let out = st.version_json(&v);
    st.versions[idx] = v;
    ok(out)
}

async fn remove_and_swap(
    AxState(app): AxState<App>,
    Path(id): Path<String>,
    body: Bytes,
) -> ApiResult {
    let body = if body.iter().all(u8::is_ascii_whitespace) {
        Map::new()
    } else {
        parse_body(&body)?
    };
    let mut st = app.lock();
    let idx = version_index(&st, &id)?;
    let vid = st.versions[idx].id;
    let pid = st.versions[idx].project_id;
    let target = match body.get("moveFixIssuesTo") {
        None | Some(Value::Null) => None,
        Some(t) => Some(
            as_id(t)
                .filter(|t| *t != vid && st.version(*t).is_some_and(|x| x.project_id == pid))
                .ok_or_else(|| {
                    ApiError::field("moveFixIssuesTo", format!("Version cible invalide : {t}."))
                })?,
        ),
    };
    for issue in st.issues.iter_mut() {
        if issue.fix_versions.contains(&vid) {
            issue.fix_versions.retain(|x| *x != vid);
            if let Some(t) = target
                && !issue.fix_versions.contains(&t)
            {
                issue.fix_versions.push(t);
            }
        }
    }
    st.versions.remove(idx);
    no_content()
}
