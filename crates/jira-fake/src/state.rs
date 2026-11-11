//! État en mémoire du faux Jira et rendu JSON au format HTTP de Jira Cloud.
//!
//! Le jeu de données reproduit celui de `MockJira` (projets DEMO et OPS, board scrum, sprints,
//! versions, issues) mais sans dépendre de la crate `jira-client` : seul le format HTTP compte.

use serde_json::{Map, Value, json};

use crate::time::{today, user_date};

/// Ids des champs custom (fictifs, comme dans `docs/api`).
pub const F_SPRINT: &str = "customfield_10020";
pub const F_POINTS: &str = "customfield_10016";
pub const F_POINTS_LEGACY: &str = "customfield_10028";
pub const F_RANK: &str = "customfield_10019";
pub const RANK_FIELD_ID: u64 = 10019;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    New,
    Indeterminate,
    Done,
}

#[derive(Debug, Clone)]
pub struct Status {
    pub id: &'static str,
    pub name: &'static str,
    pub category: Category,
}

pub const STATUSES: [Status; 4] = [
    Status {
        id: "1",
        name: "À faire",
        category: Category::New,
    },
    Status {
        id: "3",
        name: "En cours",
        category: Category::Indeterminate,
    },
    Status {
        id: "4",
        name: "En revue",
        category: Category::Indeterminate,
    },
    Status {
        id: "5",
        name: "Terminé",
        category: Category::Done,
    },
];

pub fn status(id: &str) -> Option<&'static Status> {
    STATUSES.iter().find(|s| s.id == id)
}

/// (id, nom, description)
pub const ISSUE_TYPES: [(&str, &str, &str); 3] = [
    (
        "10001",
        "Story",
        "Functionality or a feature expressed as a user goal.",
    ),
    ("10002", "Bug", "A problem or error."),
    ("10003", "Tâche", "A small, distinct piece of work."),
];

pub const PRIORITIES: [(&str, &str); 5] = [
    ("1", "Highest"),
    ("2", "High"),
    ("3", "Medium"),
    ("4", "Low"),
    ("5", "Lowest"),
];

#[derive(Debug, Clone)]
pub struct User {
    pub account_id: &'static str,
    pub display_name: &'static str,
    pub email: Option<&'static str>,
    pub slug: &'static str,
}

pub const USERS: [User; 2] = [
    User {
        account_id: "712020:00000000-0000-0000-0000-00000000a11c",
        display_name: "Alice Martin",
        email: Some("alice@example.com"),
        slug: "alice",
    },
    User {
        account_id: "712020:00000000-0000-0000-0000-000000000b0b",
        display_name: "Bob Durand",
        email: None,
        slug: "bob",
    },
];

pub fn user(account_id: &str) -> Option<&'static User> {
    USERS.iter().find(|u| u.account_id == account_id)
}

#[derive(Debug, Clone)]
pub struct Project {
    pub id: u64,
    pub key: String,
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct Board {
    pub id: u64,
    pub name: String,
    pub project_key: String,
    pub filter_id: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SprintState {
    Future,
    Active,
    Closed,
}

impl SprintState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Future => "future",
            Self::Active => "active",
            Self::Closed => "closed",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "future" => Some(Self::Future),
            "active" => Some(Self::Active),
            "closed" => Some(Self::Closed),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Sprint {
    pub id: u64,
    pub name: String,
    pub state: SprintState,
    pub goal: String,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    pub complete_date: Option<String>,
    pub created_date: String,
    pub origin_board_id: u64,
}

#[derive(Debug, Clone)]
pub struct Version {
    pub id: u64,
    pub project_id: u64,
    pub name: String,
    pub description: String,
    pub released: bool,
    pub archived: bool,
    pub start_date: Option<String>,
    pub release_date: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Issue {
    pub id: u64,
    pub key: String,
    pub project_key: String,
    pub summary: String,
    /// Document ADF ou `None`.
    pub description: Option<Value>,
    pub status_id: &'static str,
    pub type_id: String,
    pub priority_id: Option<String>,
    pub assignee: Option<String>,
    pub points: Option<f64>,
    pub fix_versions: Vec<u64>,
    /// Historique des sprints (fermés + au plus un ouvert), comme le champ Sprint de Jira.
    pub sprints: Vec<u64>,
}

#[derive(Debug)]
pub struct State {
    pub base: String,
    pub projects: Vec<Project>,
    pub boards: Vec<Board>,
    pub sprints: Vec<Sprint>,
    /// L'ordre du vecteur est le rang (global, comme le champ Rank de Jira).
    pub issues: Vec<Issue>,
    pub versions: Vec<Version>,
    pub next_issue_id: u64,
    pub next_sprint_id: u64,
    pub next_version_id: u64,
}

/// Document ADF d'un paragraphe.
pub fn adf(text: &str) -> Value {
    json!({ "type": "doc", "version": 1, "content": [
        { "type": "paragraph", "content": [ { "type": "text", "text": text } ] }
    ]})
}

impl State {
    pub fn seed(base: impl Into<String>) -> Self {
        let alice = USERS[0].account_id;
        let bob = USERS[1].account_id;
        // (résumé, type, statut, assignee, points, version, sprints)
        #[allow(clippy::type_complexity)]
        let demo: [(
            &str,
            &str,
            &str,
            Option<&str>,
            Option<f64>,
            Option<u64>,
            &[u64],
        ); 12] = [
            (
                "Connexion SSO",
                "10001",
                "5",
                Some(alice),
                Some(5.0),
                Some(101),
                &[10],
            ),
            (
                "Page de profil utilisateur",
                "10001",
                "4",
                Some(bob),
                Some(3.0),
                Some(101),
                &[10],
            ),
            (
                "Erreur 500 à l'export CSV",
                "10002",
                "3",
                Some(alice),
                Some(2.0),
                Some(101),
                &[9, 10],
            ),
            (
                "Mise à jour des dépendances",
                "10003",
                "1",
                None,
                Some(1.0),
                Some(101),
                &[10],
            ),
            (
                "Notifications par e-mail",
                "10001",
                "1",
                Some(bob),
                Some(8.0),
                Some(101),
                &[10],
            ),
            (
                "Filtre avancé de recherche",
                "10001",
                "1",
                None,
                Some(5.0),
                Some(102),
                &[11],
            ),
            (
                "Tableau de bord d'équipe",
                "10001",
                "1",
                None,
                Some(13.0),
                Some(102),
                &[11],
            ),
            (
                "Traduction anglaise",
                "10003",
                "1",
                None,
                None,
                Some(102),
                &[],
            ),
            (
                "Crash au redimensionnement",
                "10002",
                "1",
                None,
                Some(2.0),
                None,
                &[],
            ),
            // Passée par le sprint 9 (clôturé) sans être terminée : revenue au backlog.
            (
                "Export PDF des rapports",
                "10001",
                "1",
                None,
                None,
                None,
                &[9],
            ),
            (
                "Documentation API publique",
                "10003",
                "1",
                None,
                Some(3.0),
                Some(102),
                &[],
            ),
            ("Mode sombre", "10001", "1", None, None, None, &[]),
        ];
        let mut issues: Vec<Issue> = demo
            .iter()
            .enumerate()
            .map(|(i, (summary, t, s, who, pts, ver, sprints))| {
                let n = i as u64 + 1;
                Issue {
                    id: 20000 + n,
                    key: format!("DEMO-{n}"),
                    project_key: "DEMO".into(),
                    summary: (*summary).into(),
                    description: Some(adf(&format!("Description fictive de DEMO-{n}."))),
                    status_id: status(s).map_or("1", |s| s.id),
                    type_id: (*t).into(),
                    priority_id: Some(if *t == "10002" { "2" } else { "3" }.into()),
                    assignee: who.map(str::to_owned),
                    points: *pts,
                    fix_versions: ver.iter().copied().collect(),
                    sprints: sprints.to_vec(),
                }
            })
            .collect();
        issues.push(Issue {
            id: 30001,
            key: "OPS-1".into(),
            project_key: "OPS".into(),
            summary: "Renouveler les certificats".into(),
            description: None,
            status_id: "1",
            type_id: "10003".into(),
            priority_id: Some("2".into()),
            assignee: None,
            points: Some(2.0),
            fix_versions: vec![],
            sprints: vec![],
        });

        let sprint =
            |id, name: &str, state, goal: &str, start: Option<&str>, end: Option<&str>| Sprint {
                id,
                name: name.into(),
                state,
                goal: goal.into(),
                start_date: start.map(Into::into),
                end_date: end.map(Into::into),
                complete_date: (state == SprintState::Closed)
                    .then(|| "2026-10-28T16:45:12.345Z".into()),
                created_date: "2026-10-01T10:00:00.000Z".into(),
                origin_board_id: 1,
            };
        let version =
            |id, name: &str, desc: &str, released, start: Option<&str>, date: &str| Version {
                id,
                project_id: 10000,
                name: name.into(),
                description: desc.into(),
                released,
                archived: false,
                start_date: start.map(Into::into),
                release_date: Some(date.into()),
            };

        Self {
            base: base.into(),
            projects: vec![
                Project {
                    id: 10000,
                    key: "DEMO".into(),
                    name: "Démo".into(),
                },
                Project {
                    id: 10010,
                    key: "OPS".into(),
                    name: "Opérations".into(),
                },
            ],
            boards: vec![
                Board {
                    id: 1,
                    name: "DEMO board".into(),
                    project_key: "DEMO".into(),
                    filter_id: 10001,
                },
                Board {
                    id: 2,
                    name: "OPS board".into(),
                    project_key: "OPS".into(),
                    filter_id: 10002,
                },
            ],
            sprints: vec![
                sprint(
                    9,
                    "DEMO Sprint 3",
                    SprintState::Closed,
                    "",
                    Some("2026-10-14T09:00:00.000Z"),
                    Some("2026-10-28T17:00:00.000Z"),
                ),
                sprint(
                    10,
                    "DEMO Sprint 4",
                    SprintState::Active,
                    "Livrer l'authentification",
                    Some("2026-11-04T09:00:00.000Z"),
                    Some("2026-11-18T17:00:00.000Z"),
                ),
                sprint(11, "DEMO Sprint 5", SprintState::Future, "", None, None),
            ],
            issues,
            versions: vec![
                version(100, "1.0.0", "Première version", true, None, "2026-09-30"),
                version(
                    101,
                    "1.1.0",
                    "Authentification",
                    false,
                    Some("2026-10-01"),
                    "2026-11-30",
                ),
                version(102, "2.0.0", "", false, None, "2027-02-28"),
            ],
            next_issue_id: 20100,
            next_sprint_id: 100,
            next_version_id: 200,
        }
    }

    // --- Recherches ---------------------------------------------------------------------------

    pub fn project(&self, key_or_id: &str) -> Option<&Project> {
        let k = key_or_id.trim();
        self.projects
            .iter()
            .find(|p| p.key.eq_ignore_ascii_case(k) || p.id.to_string() == k)
    }

    pub fn board(&self, id: u64) -> Option<&Board> {
        self.boards.iter().find(|b| b.id == id)
    }

    pub fn sprint(&self, id: u64) -> Option<&Sprint> {
        self.sprints.iter().find(|s| s.id == id)
    }

    pub fn sprint_is_open(&self, id: u64) -> bool {
        self.sprint(id)
            .is_some_and(|s| s.state != SprintState::Closed)
    }

    /// JQL `openSprints()` sur Jira Cloud : sprints actifs seulement.
    pub fn sprint_is_active(&self, id: u64) -> bool {
        self.sprint(id)
            .is_some_and(|s| s.state == SprintState::Active)
    }

    /// JQL `futureSprints()`.
    pub fn sprint_is_future(&self, id: u64) -> bool {
        self.sprint(id)
            .is_some_and(|s| s.state == SprintState::Future)
    }

    pub fn version(&self, id: u64) -> Option<&Version> {
        self.versions.iter().find(|v| v.id == id)
    }

    /// Recherche par clé ou id.
    pub fn issue_index(&self, key_or_id: &str) -> Option<usize> {
        let k = key_or_id.trim();
        self.issues
            .iter()
            .position(|i| i.key.eq_ignore_ascii_case(k) || i.id.to_string() == k)
    }

    // --- Rendu JSON ---------------------------------------------------------------------------

    pub fn status_json(&self, s: &Status) -> Value {
        let (cid, ckey, color, cname) = match s.category {
            Category::New => (2, "new", "blue-gray", "À faire"),
            Category::Indeterminate => (4, "indeterminate", "yellow", "En cours"),
            Category::Done => (3, "done", "green", "Terminé"),
        };
        let b = &self.base;
        json!({
            "self": format!("{b}/rest/api/3/status/{}", s.id),
            "description": "",
            "iconUrl": format!("{b}/images/icons/statuses/generic.png"),
            "name": s.name,
            "id": s.id,
            "statusCategory": {
                "self": format!("{b}/rest/api/3/statuscategory/{cid}"),
                "id": cid, "key": ckey, "colorName": color, "name": cname
            }
        })
    }

    pub fn user_json(&self, u: &User) -> Value {
        let mut v = json!({
            "self": format!("{}/rest/api/3/user?accountId={}", self.base, u.account_id),
            "accountId": u.account_id,
            "accountType": "atlassian",
            "avatarUrls": {
                "48x48": format!("https://avatar.example.test/{}/48.png", u.slug),
                "24x24": format!("https://avatar.example.test/{}/24.png", u.slug),
                "16x16": format!("https://avatar.example.test/{}/16.png", u.slug),
                "32x32": format!("https://avatar.example.test/{}/32.png", u.slug),
            },
            "displayName": u.display_name,
            "active": true,
            "timeZone": "Europe/Paris",
        });
        if let Some(e) = u.email {
            v["emailAddress"] = json!(e);
        }
        v
    }

    pub fn issue_type_json(&self, id: &str) -> Value {
        let (id, name, desc) = ISSUE_TYPES
            .iter()
            .find(|t| t.0 == id)
            .copied()
            .unwrap_or(ISSUE_TYPES[0]);
        json!({
            "self": format!("{}/rest/api/3/issuetype/{id}", self.base),
            "id": id,
            "description": desc,
            "iconUrl": format!("{}/images/icons/issuetypes/{}.svg", self.base, name.to_lowercase()),
            "name": name,
            "untranslatedName": name,
            "subtask": false,
            "hierarchyLevel": 0
        })
    }

    pub fn priority_json(&self, id: &str) -> Value {
        let name = PRIORITIES
            .iter()
            .find(|p| p.0 == id)
            .map_or("Medium", |p| p.1);
        json!({
            "self": format!("{}/rest/api/3/priority/{id}", self.base),
            "iconUrl": format!("{}/images/icons/priorities/{}.svg", self.base, name.to_lowercase()),
            "name": name,
            "id": id
        })
    }

    pub fn version_json(&self, v: &Version) -> Value {
        let mut o = json!({
            "self": format!("{}/rest/api/3/version/{}", self.base, v.id),
            "id": v.id.to_string(),
            "description": v.description,
            "name": v.name,
            "archived": v.archived,
            "released": v.released,
            "projectId": v.project_id,
        });
        if let Some(d) = &v.start_date {
            o["startDate"] = json!(d);
            o["userStartDate"] = json!(user_date(d));
        }
        if let Some(d) = &v.release_date {
            o["releaseDate"] = json!(d);
            o["userReleaseDate"] = json!(user_date(d));
            if !v.released {
                o["overdue"] = json!(d.as_str() < today().as_str());
            }
        }
        o
    }

    fn fix_version_json(&self, v: &Version) -> Value {
        let mut o = json!({
            "self": format!("{}/rest/api/3/version/{}", self.base, v.id),
            "id": v.id.to_string(),
            "description": v.description,
            "name": v.name,
            "archived": v.archived,
            "released": v.released,
        });
        if let Some(d) = &v.release_date {
            o["releaseDate"] = json!(d);
        }
        o
    }

    pub fn sprint_json(&self, s: &Sprint) -> Value {
        let mut o = json!({
            "id": s.id,
            "self": format!("{}/rest/agile/1.0/sprint/{}", self.base, s.id),
            "state": s.state.as_str(),
            "name": s.name,
            "createdDate": s.created_date,
            "originBoardId": s.origin_board_id,
            "goal": s.goal,
        });
        for (k, v) in [
            ("startDate", &s.start_date),
            ("endDate", &s.end_date),
            ("completeDate", &s.complete_date),
        ] {
            if let Some(v) = v {
                o[k] = json!(v);
            }
        }
        o
    }

    /// Forme du champ custom Sprint dans une issue (`boardId`, pas de `self`).
    fn sprint_field_json(&self, s: &Sprint) -> Value {
        let mut o = json!({
            "id": s.id,
            "name": s.name,
            "state": s.state.as_str(),
            "boardId": s.origin_board_id,
            "goal": s.goal,
        });
        for (k, v) in [
            ("startDate", &s.start_date),
            ("endDate", &s.end_date),
            ("completeDate", &s.complete_date),
        ] {
            if let Some(v) = v {
                o[k] = json!(v);
            }
        }
        o
    }

    pub fn board_json(&self, b: &Board) -> Value {
        let p = self.project(&b.project_key);
        let pname = p.map_or("", |p| p.name.as_str());
        let display = format!("{pname} ({})", b.project_key);
        json!({
            "id": b.id,
            "self": format!("{}/rest/agile/1.0/board/{}", self.base, b.id),
            "name": b.name,
            "type": "scrum",
            "location": {
                "projectId": p.map_or(0, |p| p.id),
                "displayName": display,
                "projectName": pname,
                "projectKey": b.project_key,
                "projectTypeKey": "software",
                "avatarURI": format!("{}/rest/api/2/universal_avatar/view/type/project/avatar/10400?size=small", self.base),
                "name": display,
            }
        })
    }

    pub fn project_json(&self, p: &Project) -> Value {
        let avatar = format!(
            "{}/rest/api/3/universal_avatar/view/type/project/avatar/10400",
            self.base
        );
        json!({
            "expand": "description,lead,issueTypes,url,projectKeys,permissions,insight",
            "self": format!("{}/rest/api/3/project/{}", self.base, p.id),
            "id": p.id.to_string(),
            "key": p.key,
            "name": p.name,
            "avatarUrls": {
                "48x48": avatar,
                "24x24": format!("{avatar}?size=small"),
                "16x16": format!("{avatar}?size=xsmall"),
                "32x32": format!("{avatar}?size=medium"),
            },
            "projectTypeKey": "software",
            "simplified": false,
            "style": "classic",
            "isPrivate": false,
            "properties": {}
        })
    }

    /// Rang LexoRank fictif dérivé de la position (ordre lexicographique = ordre du vecteur).
    fn rank_of(&self, index: usize) -> String {
        format!("0|i{:05}:", (index + 1) * 8)
    }

    /// Tous les champs d'une issue au format v3.
    pub fn issue_fields(&self, index: usize) -> Map<String, Value> {
        let i = &self.issues[index];
        let mut f = Map::new();
        f.insert("summary".into(), json!(i.summary));
        f.insert(
            "description".into(),
            i.description.clone().unwrap_or(Value::Null),
        );
        let st = status(i.status_id).unwrap_or(&STATUSES[0]);
        f.insert("status".into(), self.status_json(st));
        f.insert("issuetype".into(), self.issue_type_json(&i.type_id));
        f.insert(
            "priority".into(),
            i.priority_id
                .as_deref()
                .map_or(Value::Null, |p| self.priority_json(p)),
        );
        f.insert(
            "assignee".into(),
            i.assignee
                .as_deref()
                .and_then(user)
                .map_or(Value::Null, |u| self.user_json(u)),
        );
        if let Some(p) = self.project(&i.project_key) {
            f.insert(
                "project".into(),
                json!({
                    "self": format!("{}/rest/api/3/project/{}", self.base, p.id),
                    "id": p.id.to_string(), "key": p.key, "name": p.name,
                    "projectTypeKey": "software", "simplified": false
                }),
            );
        }
        let fv: Vec<Value> = i
            .fix_versions
            .iter()
            .filter_map(|id| self.version(*id))
            .map(|v| self.fix_version_json(v))
            .collect();
        f.insert("fixVersions".into(), Value::Array(fv));
        let sprints: Vec<Value> = i
            .sprints
            .iter()
            .filter_map(|id| self.sprint(*id))
            .map(|s| self.sprint_field_json(s))
            .collect();
        f.insert(
            F_SPRINT.into(),
            if sprints.is_empty() {
                Value::Null
            } else {
                Value::Array(sprints)
            },
        );
        f.insert(F_POINTS.into(), i.points.map_or(Value::Null, |p| json!(p)));
        f.insert(F_POINTS_LEGACY.into(), Value::Null);
        f.insert(F_RANK.into(), json!(self.rank_of(index)));
        f
    }

    /// Issue v3 restreinte aux champs demandés (`fields` de Jira : défaut `id` seulement).
    pub fn issue_json(&self, index: usize, fields: &FieldSelection) -> Value {
        let i = &self.issues[index];
        let all = self.issue_fields(index);
        let selected: Map<String, Value> = all
            .into_iter()
            .filter(|(k, _)| fields.includes(k))
            .collect();
        json!({
            "expand": "renderedFields,names,schema,operations,editmeta,changelog,versionedRepresentations",
            "id": i.id.to_string(),
            "self": format!("{}/rest/api/3/issue/{}", self.base, i.id),
            "key": i.key,
            "fields": selected,
        })
    }
}

/// Paramètre `fields` de Jira : liste explicite, `*all`, `*navigable`, exclusions `-champ`.
#[derive(Debug, Default)]
pub struct FieldSelection {
    all: bool,
    include: Vec<String>,
    exclude: Vec<String>,
}

impl FieldSelection {
    pub fn parse<'a>(items: impl IntoIterator<Item = &'a str>) -> Self {
        let mut sel = Self::default();
        for raw in items {
            let f = raw.trim();
            match f {
                "" | "id" | "key" => {}
                "*all" | "*navigable" => sel.all = true,
                _ => match f.strip_prefix('-') {
                    Some(x) => sel.exclude.push(x.to_owned()),
                    None => sel.include.push(f.to_owned()),
                },
            }
        }
        sel
    }

    pub fn all() -> Self {
        Self {
            all: true,
            ..Self::default()
        }
    }

    fn includes(&self, name: &str) -> bool {
        if self.exclude.iter().any(|e| e == name) {
            return false;
        }
        self.all || self.include.iter().any(|i| i == name)
    }
}
