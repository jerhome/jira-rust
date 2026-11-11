//! Implémentation en mémoire de [`JiraApi`] avec des données fictives (projets DEMO et OPS).
//! Sert au développement de la TUI (`jira-rust --mock`) et aux tests. Aucun accès réseau.

use std::sync::{Mutex, MutexGuard};

use async_trait::async_trait;

use crate::model::*;
use crate::{Error, JiraApi, Result};

const ACTIVE: SprintState = SprintState::Active;
const FUTURE: SprintState = SprintState::Future;

#[derive(Debug, Clone)]
struct MockIssue {
    project_key: String,
    issue: Issue,
    /// Sprints clôturés dont l'issue faisait partie (historique, comme le champ Sprint de Jira).
    closed_sprints: Vec<u64>,
}

#[derive(Debug)]
struct State {
    projects: Vec<Project>,
    boards: Vec<Board>,
    sprints: Vec<Sprint>,
    /// L'ordre du vecteur est le rang.
    issues: Vec<MockIssue>,
    versions: Vec<Version>,
    next_issue_number: u32,
    next_version_id: u32,
    next_sprint_id: u64,
}

/// Faux Jira en mémoire. Thread-safe, clonable via `Arc`.
#[derive(Debug)]
pub struct MockJira {
    state: Mutex<State>,
}

impl Default for MockJira {
    fn default() -> Self {
        Self::new()
    }
}

fn status(id: &str, name: &str, category: StatusCategory) -> Status {
    Status {
        id: id.into(),
        name: name.into(),
        category,
    }
}

fn statuses() -> Vec<Status> {
    vec![
        status("1", "À faire", StatusCategory::ToDo),
        status("3", "En cours", StatusCategory::InProgress),
        status("4", "En revue", StatusCategory::InProgress),
        status("5", "Terminé", StatusCategory::Done),
    ]
}

fn issue_types_list() -> Vec<IssueType> {
    vec![
        IssueType {
            id: "10001".into(),
            name: "Story".into(),
            subtask: false,
        },
        IssueType {
            id: "10002".into(),
            name: "Bug".into(),
            subtask: false,
        },
        IssueType {
            id: "10003".into(),
            name: "Tâche".into(),
            subtask: false,
        },
    ]
}

fn user(id: &str, name: &str) -> User {
    User {
        account_id: id.into(),
        display_name: name.into(),
        email: None,
    }
}

impl MockJira {
    pub fn new() -> Self {
        let st = statuses();
        let types = issue_types_list();
        let alice = user("mock-alice", "Alice Martin");
        let bob = user("mock-bob", "Bob Durand");
        let v110 = VersionRef {
            id: "101".into(),
            name: "1.1.0".into(),
        };
        let v200 = VersionRef {
            id: "102".into(),
            name: "2.0.0".into(),
        };

        // (résumé, type, statut, assignee, points, version, sprint)
        #[allow(clippy::type_complexity)]
        let demo: Vec<(
            &str,
            usize,
            usize,
            Option<&User>,
            Option<f64>,
            Option<&VersionRef>,
            Option<u64>,
        )> = vec![
            (
                "Connexion SSO",
                0,
                3,
                Some(&alice),
                Some(5.0),
                Some(&v110),
                Some(10),
            ),
            (
                "Page de profil utilisateur",
                0,
                2,
                Some(&bob),
                Some(3.0),
                Some(&v110),
                Some(10),
            ),
            (
                "Erreur 500 à l'export CSV",
                1,
                1,
                Some(&alice),
                Some(2.0),
                Some(&v110),
                Some(10),
            ),
            (
                "Mise à jour des dépendances",
                2,
                0,
                None,
                Some(1.0),
                Some(&v110),
                Some(10),
            ),
            (
                "Notifications par e-mail",
                0,
                0,
                Some(&bob),
                Some(8.0),
                Some(&v110),
                Some(10),
            ),
            (
                "Filtre avancé de recherche",
                0,
                0,
                None,
                Some(5.0),
                Some(&v200),
                Some(11),
            ),
            (
                "Tableau de bord d'équipe",
                0,
                0,
                None,
                Some(13.0),
                Some(&v200),
                Some(11),
            ),
            ("Traduction anglaise", 2, 0, None, None, Some(&v200), None),
            (
                "Crash au redimensionnement",
                1,
                0,
                None,
                Some(2.0),
                None,
                None,
            ),
            ("Export PDF des rapports", 0, 0, None, None, None, None),
            (
                "Documentation API publique",
                2,
                0,
                None,
                Some(3.0),
                Some(&v200),
                None,
            ),
            ("Mode sombre", 0, 0, None, None, None, None),
        ];

        let mut issues: Vec<MockIssue> = demo
            .into_iter()
            .enumerate()
            .map(|(i, (summary, t, s, assignee, pts, ver, sprint))| {
                let n = i + 1;
                MockIssue {
                    project_key: "DEMO".into(),
                    issue: Issue {
                        id: format!("{}", 20000 + n),
                        key: format!("DEMO-{n}"),
                        summary: summary.into(),
                        description: Some(format!("Description fictive de DEMO-{n}.")),
                        description_is_lossless: true,
                        status: st[s].clone(),
                        issue_type: types[t].clone(),
                        priority: Some(if t == 1 { "High" } else { "Medium" }.into()),
                        assignee: assignee.cloned(),
                        story_points: pts,
                        fix_versions: ver.cloned().into_iter().collect(),
                        sprint_id: sprint,
                    },
                    closed_sprints: vec![],
                }
            })
            .collect();

        issues.push(MockIssue {
            project_key: "OPS".into(),
            issue: Issue {
                id: "30001".into(),
                key: "OPS-1".into(),
                summary: "Renouveler les certificats".into(),
                description: None,
                description_is_lossless: true,
                status: st[0].clone(),
                issue_type: types[2].clone(),
                priority: Some("High".into()),
                assignee: None,
                story_points: Some(2.0),
                fix_versions: vec![],
                sprint_id: None,
            },
            closed_sprints: vec![],
        });

        let version = |id: &str, pid: &str, name: &str, released: bool, date: &str| Version {
            id: id.into(),
            project_id: pid.into(),
            name: name.into(),
            description: None,
            released,
            archived: false,
            start_date: None,
            release_date: Some(date.into()),
        };

        let state = State {
            projects: vec![
                Project {
                    id: "10000".into(),
                    key: "DEMO".into(),
                    name: "Démo".into(),
                },
                Project {
                    id: "10010".into(),
                    key: "OPS".into(),
                    name: "Opérations".into(),
                },
            ],
            boards: vec![
                Board {
                    id: 1,
                    name: "DEMO board".into(),
                    project_key: Some("DEMO".into()),
                    board_type: Some("scrum".into()),
                },
                Board {
                    id: 2,
                    name: "OPS board".into(),
                    project_key: Some("OPS".into()),
                    board_type: Some("scrum".into()),
                },
            ],
            sprints: vec![
                Sprint {
                    id: 9,
                    board_id: Some(1),
                    name: "DEMO Sprint 3".into(),
                    state: SprintState::Closed,
                    goal: None,
                    start_date: Some("2026-10-14T09:00:00.000Z".into()),
                    end_date: Some("2026-10-28T17:00:00.000Z".into()),
                },
                Sprint {
                    id: 10,
                    board_id: Some(1),
                    name: "DEMO Sprint 4".into(),
                    state: ACTIVE,
                    goal: Some("Livrer l'authentification".into()),
                    start_date: Some("2026-11-04T09:00:00.000Z".into()),
                    end_date: Some("2026-11-18T17:00:00.000Z".into()),
                },
                Sprint {
                    id: 11,
                    board_id: Some(1),
                    name: "DEMO Sprint 5".into(),
                    state: FUTURE,
                    goal: None,
                    start_date: None,
                    end_date: None,
                },
            ],
            issues,
            versions: vec![
                version("100", "10000", "1.0.0", true, "2026-09-30"),
                version("101", "10000", "1.1.0", false, "2026-11-30"),
                version("102", "10000", "2.0.0", false, "2027-02-28"),
            ],
            next_issue_number: 100,
            next_version_id: 200,
            next_sprint_id: 100,
        };

        Self {
            state: Mutex::new(state),
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        // Un panic dans un test ne doit pas empoisonner les suivants.
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl State {
    fn issue_mut(&mut self, key: &str) -> Result<&mut MockIssue> {
        self.issues
            .iter_mut()
            .find(|i| i.issue.key == key)
            .ok_or_else(|| Error::NotFound(format!("issue {key}")))
    }

    fn sprint_mut(&mut self, id: u64) -> Result<&mut Sprint> {
        self.sprints
            .iter_mut()
            .find(|s| s.id == id)
            .ok_or_else(|| Error::NotFound(format!("sprint {id}")))
    }

    fn board_project(&self, board_id: u64) -> Result<String> {
        self.boards
            .iter()
            .find(|b| b.id == board_id)
            .and_then(|b| b.project_key.clone())
            .ok_or_else(|| Error::NotFound(format!("board {board_id}")))
    }

    /// Vérifie que toutes les clés existent (avant toute modification).
    fn check_keys<'a>(&self, keys: impl IntoIterator<Item = &'a String>) -> Result<()> {
        for key in keys {
            if !self.issues.iter().any(|i| &i.issue.key == key) {
                return Err(Error::NotFound(format!("issue {key}")));
            }
        }
        Ok(())
    }

    fn project_id(&self, key: &str) -> Option<&str> {
        self.projects
            .iter()
            .find(|p| p.key == key)
            .map(|p| p.id.as_str())
    }

    /// Références de versions, toutes du projet donné (400 sinon, comme Jira).
    fn fix_version_refs(&self, project_key: &str, ids: &[String]) -> Result<Vec<VersionRef>> {
        let pid = self.project_id(project_key);
        ids.iter()
            .map(|id| {
                let v = self.versions.iter().find(|v| &v.id == id);
                match v {
                    Some(v) if Some(v.project_id.as_str()) == pid => Ok(VersionRef {
                        id: v.id.clone(),
                        name: v.name.clone(),
                    }),
                    Some(_) => Err(Error::Api {
                        status: 400,
                        message: format!(
                            "fixVersions : la version {id} n'appartient pas au projet {project_key}"
                        ),
                    }),
                    None => Err(Error::NotFound(format!("version {id}"))),
                }
            })
            .collect()
    }

    fn version_mut(&mut self, id: &str) -> Result<&mut Version> {
        self.versions
            .iter_mut()
            .find(|v| v.id == id)
            .ok_or_else(|| Error::NotFound(format!("version {id}")))
    }
}

#[async_trait]
impl JiraApi for MockJira {
    async fn myself(&self) -> Result<User> {
        Ok(User {
            account_id: "mock-me".into(),
            display_name: "Utilisateur Mock".into(),
            email: Some("moi@example.com".into()),
        })
    }

    async fn projects(&self) -> Result<Vec<Project>> {
        Ok(self.lock().projects.clone())
    }

    async fn scrum_boards(&self, project_key: &str) -> Result<Vec<Board>> {
        Ok(self
            .lock()
            .boards
            .iter()
            .filter(|b| b.project_key.as_deref() == Some(project_key))
            .cloned()
            .collect())
    }

    async fn sprints(&self, board_id: u64, states: &[SprintState]) -> Result<Vec<Sprint>> {
        Ok(self
            .lock()
            .sprints
            .iter()
            .filter(|s| s.board_id == Some(board_id))
            .filter(|s| states.is_empty() || states.contains(&s.state))
            .cloned()
            .collect())
    }

    async fn backlog_issues(&self, board_id: u64) -> Result<Vec<Issue>> {
        let st = self.lock();
        let project = st.board_project(board_id)?;
        Ok(st
            .issues
            .iter()
            .filter(|i| {
                i.project_key == project
                    && i.issue.sprint_id.is_none()
                    && i.issue.status.category != StatusCategory::Done
                    && !i.issue.issue_type.subtask
            })
            .map(|i| i.issue.clone())
            .collect())
    }

    async fn sprint_issues(&self, sprint_id: u64) -> Result<Vec<Issue>> {
        let st = self.lock();
        if !st.sprints.iter().any(|s| s.id == sprint_id) {
            return Err(Error::NotFound(format!("sprint {sprint_id}")));
        }
        Ok(st
            .issues
            .iter()
            .filter(|i| {
                i.issue.sprint_id == Some(sprint_id) || i.closed_sprints.contains(&sprint_id)
            })
            .map(|i| i.issue.clone())
            .collect())
    }

    async fn move_to_sprint(&self, sprint_id: u64, issue_keys: &[String]) -> Result<()> {
        let mut st = self.lock();
        let state = st.sprint_mut(sprint_id)?.state;
        if state == SprintState::Closed {
            return Err(Error::Invalid(format!("le sprint {sprint_id} est clôturé")));
        }
        st.check_keys(issue_keys)?;
        for key in issue_keys {
            st.issue_mut(key)?.issue.sprint_id = Some(sprint_id);
        }
        Ok(())
    }

    async fn move_to_backlog(&self, issue_keys: &[String]) -> Result<()> {
        let mut st = self.lock();
        st.check_keys(issue_keys)?;
        for key in issue_keys {
            st.issue_mut(key)?.issue.sprint_id = None;
        }
        Ok(())
    }

    async fn rank(&self, issue_keys: &[String], position: &RankPosition) -> Result<()> {
        let mut st = self.lock();
        let anchor = match position {
            RankPosition::Before(k) | RankPosition::After(k) => k.clone(),
        };
        if issue_keys.contains(&anchor) {
            return Err(Error::Invalid(
                "l'issue de référence fait partie des issues à déplacer".into(),
            ));
        }
        st.check_keys(issue_keys.iter().chain([&anchor]))?;
        let mut moved = Vec::with_capacity(issue_keys.len());
        for key in issue_keys {
            let idx = st
                .issues
                .iter()
                .position(|i| &i.issue.key == key)
                .ok_or_else(|| Error::NotFound(format!("issue {key}")))?;
            moved.push(st.issues.remove(idx));
        }
        let anchor_idx = st
            .issues
            .iter()
            .position(|i| i.issue.key == anchor)
            .ok_or_else(|| Error::NotFound(format!("issue {anchor}")))?;
        let insert_at = match position {
            RankPosition::Before(_) => anchor_idx,
            RankPosition::After(_) => anchor_idx + 1,
        };
        st.issues.splice(insert_at..insert_at, moved);
        Ok(())
    }

    async fn start_sprint(&self, sprint_id: u64, params: &StartSprint) -> Result<Sprint> {
        check_sprint_dates(&params.start_date, &params.end_date)?;
        let mut st = self.lock();
        let board = st.sprint_mut(sprint_id)?.board_id;
        if st
            .sprints
            .iter()
            .any(|s| s.board_id == board && s.state == SprintState::Active)
        {
            return Err(Error::Invalid(
                "un sprint est déjà actif sur ce board".into(),
            ));
        }
        let sprint = st.sprint_mut(sprint_id)?;
        if sprint.state != SprintState::Future {
            return Err(Error::Invalid(format!(
                "le sprint {sprint_id} n'est pas futur"
            )));
        }
        sprint.state = SprintState::Active;
        sprint.start_date = Some(params.start_date.clone());
        sprint.end_date = Some(params.end_date.clone());
        if params.goal.is_some() {
            sprint.goal = params.goal.clone();
        }
        Ok(sprint.clone())
    }

    async fn close_sprint(&self, sprint_id: u64) -> Result<Sprint> {
        let mut st = self.lock();
        let sprint = st.sprint_mut(sprint_id)?;
        if sprint.state != SprintState::Active {
            return Err(Error::Invalid(format!(
                "le sprint {sprint_id} n'est pas actif"
            )));
        }
        sprint.state = SprintState::Closed;
        let closed = sprint.clone();
        // Aucun déplacement : le sprint n'étant plus ouvert, il passe dans l'historique des issues
        // (`sprint_id` ne désigne qu'un sprint actif/futur). Les issues non terminées sans autre sprint
        // ouvert réapparaissent donc dans le backlog, comme avec la JQL du client HTTP.
        for i in st.issues.iter_mut() {
            if i.issue.sprint_id == Some(sprint_id) {
                i.issue.sprint_id = None;
                i.closed_sprints.push(sprint_id);
            }
        }
        Ok(closed)
    }

    async fn create_sprint(
        &self,
        board_id: u64,
        name: &str,
        goal: Option<String>,
    ) -> Result<Sprint> {
        let mut st = self.lock();
        st.board_project(board_id)?;
        if name.trim().is_empty() {
            return Err(Error::Api {
                status: 400,
                message: "name : le nom du sprint est obligatoire".into(),
            });
        }
        let id = st.next_sprint_id;
        st.next_sprint_id += 1;
        let sprint = Sprint {
            id,
            board_id: Some(board_id),
            name: name.to_owned(),
            state: FUTURE,
            goal: goal.filter(|g| !g.trim().is_empty()),
            start_date: None,
            end_date: None,
        };
        st.sprints.push(sprint.clone());
        Ok(sprint)
    }

    async fn update_sprint(&self, sprint_id: u64, update: &SprintUpdate) -> Result<Sprint> {
        let mut st = self.lock();
        let sprint = st.sprint_mut(sprint_id)?;
        if let Some(n) = &update.name
            && n.trim().is_empty()
        {
            return Err(Error::Api {
                status: 400,
                message: "name : le nom du sprint est obligatoire".into(),
            });
        }
        let dates_changed = update.start_date.is_some() || update.end_date.is_some();
        if dates_changed && sprint.state != SprintState::Closed {
            let start = update.start_date.as_ref().or(sprint.start_date.as_ref());
            let end = update.end_date.as_ref().or(sprint.end_date.as_ref());
            match (start, end) {
                (Some(start), Some(end)) => check_sprint_dates(start, end)?,
                // Une seule date connue : seul le format est vérifié.
                (Some(d), None) => {
                    parse_sprint_date("startDate", d)?;
                }
                (None, Some(d)) => {
                    parse_sprint_date("endDate", d)?;
                }
                (None, None) => {}
            }
        }
        if let Some(n) = &update.name {
            sprint.name = n.clone();
        }
        if let Some(g) = &update.goal {
            sprint.goal = g.clone().filter(|g| !g.trim().is_empty());
        }
        // Sprint clôturé : seuls nom et objectif sont modifiables (le reste est ignoré, comme Jira) ;
        // des dates différentes demandées sont signalées comme par le client HTTP.
        if sprint.state != SprintState::Closed {
            if let Some(d) = &update.start_date {
                sprint.start_date = Some(d.clone());
            }
            if let Some(d) = &update.end_date {
                sprint.end_date = Some(d.clone());
            }
        }
        let sprint = sprint.clone();
        crate::timeutil::check_closed_sprint_dates(update, &sprint)?;
        Ok(sprint)
    }

    async fn issue_types(&self, project_key: &str) -> Result<Vec<IssueType>> {
        if !self.lock().projects.iter().any(|p| p.key == project_key) {
            return Err(Error::NotFound(format!("projet {project_key}")));
        }
        Ok(issue_types_list())
    }

    async fn create_issue(&self, new: &NewIssue) -> Result<String> {
        let mut st = self.lock();
        if !st.projects.iter().any(|p| p.key == new.project_key) {
            return Err(Error::NotFound(format!("projet {}", new.project_key)));
        }
        if new.summary.trim().is_empty() {
            return Err(Error::Invalid("le résumé est obligatoire".into()));
        }
        let issue_type = issue_types_list()
            .into_iter()
            .find(|t| t.id == new.issue_type_id)
            .ok_or_else(|| Error::Invalid(format!("type d'issue {} inconnu", new.issue_type_id)))?;
        let fix_versions = st.fix_version_refs(&new.project_key, &new.fix_version_ids)?;
        let n = st.next_issue_number;
        st.next_issue_number += 1;
        let key = format!("{}-{n}", new.project_key);
        st.issues.push(MockIssue {
            project_key: new.project_key.clone(),
            issue: Issue {
                id: format!("{}", 90000 + n),
                key: key.clone(),
                summary: new.summary.clone(),
                description: new.description.clone().filter(|d| !d.trim().is_empty()),
                description_is_lossless: true,
                status: statuses()[0].clone(),
                issue_type,
                priority: Some("Medium".into()),
                assignee: None,
                story_points: new.story_points,
                fix_versions,
                sprint_id: None,
            },
            closed_sprints: vec![],
        });
        Ok(key)
    }

    async fn update_issue(&self, key: &str, update: &IssueUpdate) -> Result<()> {
        let mut st = self.lock();
        let project = st.issue_mut(key)?.project_key.clone();
        let fix_versions = match &update.fix_version_ids {
            Some(ids) => Some(st.fix_version_refs(&project, ids)?),
            None => None,
        };
        let issue = &mut st.issue_mut(key)?.issue;
        if update.summary.as_ref().is_some_and(|s| s.trim().is_empty()) {
            return Err(Error::Invalid("le résumé est obligatoire".into()));
        }
        if let Some(s) = &update.summary {
            issue.summary = s.clone();
        }
        if let Some(d) = &update.description {
            issue.description = d.clone().filter(|d| !d.trim().is_empty());
            issue.description_is_lossless = true;
        }
        if let Some(p) = update.story_points {
            issue.story_points = p;
        }
        if let Some(v) = fix_versions {
            issue.fix_versions = v;
        }
        Ok(())
    }

    async fn transitions(&self, key: &str) -> Result<Vec<Transition>> {
        let mut st = self.lock();
        let current = st.issue_mut(key)?.issue.status.id.clone();
        Ok(statuses()
            .into_iter()
            .filter(|s| s.id != current)
            .map(|s| Transition {
                id: format!("{}1", s.id),
                name: format!("Passer à « {} »", s.name),
                has_screen: requires_resolution_screen(&current, &s.id),
                to: s,
            })
            .collect())
    }

    async fn transition_issue(&self, key: &str, transition_id: &str) -> Result<()> {
        let target = statuses()
            .into_iter()
            .find(|s| format!("{}1", s.id) == transition_id)
            .ok_or_else(|| Error::Invalid(format!("transition {transition_id} inconnue")))?;
        let mut st = self.lock();
        let issue = &mut st.issue_mut(key)?.issue;
        if issue.status.id == target.id {
            return Err(Error::Invalid(format!(
                "transition {transition_id} non disponible"
            )));
        }
        if requires_resolution_screen(&issue.status.id, &target.id) {
            return Err(Error::Api {
                status: 400,
                message: "resolution : résolution requise (écran de transition)".into(),
            });
        }
        issue.status = target;
        Ok(())
    }

    async fn versions(&self, project_key: &str) -> Result<Vec<Version>> {
        let st = self.lock();
        let project = st
            .projects
            .iter()
            .find(|p| p.key == project_key)
            .ok_or_else(|| Error::NotFound(format!("projet {project_key}")))?;
        Ok(st
            .versions
            .iter()
            .filter(|v| v.project_id == project.id)
            .cloned()
            .collect())
    }

    async fn version_issues(&self, project_key: &str, version_id: &str) -> Result<Vec<Issue>> {
        let st = self.lock();
        Ok(st
            .issues
            .iter()
            .filter(|i| i.project_key == project_key)
            .filter(|i| i.issue.fix_versions.iter().any(|v| v.id == version_id))
            .map(|i| i.issue.clone())
            .collect())
    }

    async fn create_version(&self, new: &NewVersion) -> Result<Version> {
        let mut st = self.lock();
        if !st.projects.iter().any(|p| p.id == new.project_id) {
            return Err(Error::NotFound(format!("projet id {}", new.project_id)));
        }
        if st
            .versions
            .iter()
            .any(|v| v.project_id == new.project_id && v.name == new.name)
        {
            return Err(Error::Api {
                status: 400,
                message: format!("name : une version nommée « {} » existe déjà", new.name),
            });
        }
        let id = st.next_version_id.to_string();
        st.next_version_id += 1;
        let version = Version {
            id,
            project_id: new.project_id.clone(),
            name: new.name.clone(),
            description: new.description.clone(),
            released: false,
            archived: false,
            start_date: new.start_date.clone(),
            release_date: new.release_date.clone(),
        };
        st.versions.push(version.clone());
        Ok(version)
    }

    async fn update_version(&self, version_id: &str, update: &VersionUpdate) -> Result<Version> {
        let mut st = self.lock();
        let project_id = st.version_mut(version_id)?.project_id.clone();
        if update.name.as_ref().is_some_and(|n| n.trim().is_empty()) {
            return Err(Error::Api {
                status: 400,
                message: "name : le nom de la version est obligatoire".into(),
            });
        }
        // Version cible des issues non résolues : validée avant toute modification.
        let target = match &update.move_unfixed_issues_to {
            Some(t) if t == version_id => {
                return Err(Error::Invalid(
                    "la version cible doit différer de la version modifiée".into(),
                ));
            }
            Some(t) => {
                let tv = st
                    .versions
                    .iter()
                    .find(|v| &v.id == t)
                    .ok_or_else(|| Error::NotFound(format!("version {t}")))?;
                if tv.project_id != project_id {
                    return Err(Error::Api {
                        status: 400,
                        message: format!("moveUnfixedIssuesTo : version {t} d'un autre projet"),
                    });
                }
                Some(VersionRef {
                    id: tv.id.clone(),
                    name: tv.name.clone(),
                })
            }
            None => None,
        };
        let v = st.version_mut(version_id)?;
        if let Some(n) = &update.name {
            v.name = n.clone();
        }
        if let Some(d) = &update.description {
            v.description = d.clone();
        }
        if let Some(r) = &update.start_date {
            v.start_date = r.clone();
        }
        if let Some(r) = &update.release_date {
            v.release_date = r.clone();
        }
        if let Some(r) = update.released {
            v.released = r;
        }
        if let Some(a) = update.archived {
            v.archived = a;
        }
        let updated = v.clone();
        if let Some(target) = target {
            for i in st.issues.iter_mut() {
                if i.issue.status.category == StatusCategory::Done {
                    continue;
                }
                let fv = &mut i.issue.fix_versions;
                if let Some(pos) = fv.iter().position(|r| r.id == updated.id) {
                    fv.remove(pos);
                    if !fv.iter().any(|r| r.id == target.id) {
                        fv.push(target.clone());
                    }
                }
            }
        }
        // Propager le nouveau nom dans les références des issues.
        for i in st.issues.iter_mut() {
            for fv in i.issue.fix_versions.iter_mut() {
                if fv.id == updated.id {
                    fv.name = updated.name.clone();
                }
            }
        }
        Ok(updated)
    }

    async fn delete_version(
        &self,
        version_id: &str,
        move_fix_issues_to: Option<String>,
    ) -> Result<()> {
        let mut st = self.lock();
        let project_id = st.version_mut(version_id)?.project_id.clone();
        let target = match &move_fix_issues_to {
            Some(t) if t == version_id => {
                return Err(Error::Invalid(
                    "la version cible doit différer de la version supprimée".into(),
                ));
            }
            Some(t) => {
                let tv = st
                    .versions
                    .iter()
                    .find(|v| &v.id == t)
                    .ok_or_else(|| Error::NotFound(format!("version {t}")))?;
                if tv.project_id != project_id {
                    return Err(Error::Api {
                        status: 400,
                        message: format!("moveFixIssuesTo : version {t} d'un autre projet"),
                    });
                }
                Some(VersionRef {
                    id: tv.id.clone(),
                    name: tv.name.clone(),
                })
            }
            None => None,
        };
        st.versions.retain(|v| v.id != version_id);
        for i in st.issues.iter_mut() {
            let fv = &mut i.issue.fix_versions;
            if let Some(pos) = fv.iter().position(|r| r.id == version_id) {
                fv.remove(pos);
                if let Some(t) = &target
                    && !fv.iter().any(|r| r.id == t.id)
                {
                    fv.push(t.clone());
                }
            }
        }
        Ok(())
    }
}

/// Workflow fictif : passer directement de « À faire » (1) à « Terminé » (5) exige un écran de
/// résolution que l'API ne remplit pas -> la transition échoue (400), comme sur un vrai Jira.
fn requires_resolution_screen(from_status: &str, to_status: &str) -> bool {
    from_status == "1" && to_status == "5"
}

/// Dates de sprint : ISO 8601 valides et fin strictement postérieure au début (sinon 400).
fn check_sprint_dates(start: &str, end: &str) -> Result<()> {
    let s = parse_sprint_date("startDate", start)?;
    let e = parse_sprint_date("endDate", end)?;
    if e <= s {
        return Err(Error::Api {
            status: 400,
            message: "endDate : la date de fin doit être postérieure à la date de début".into(),
        });
    }
    Ok(())
}

fn parse_sprint_date(field: &str, v: &str) -> Result<i64> {
    crate::timeutil::parse_iso8601_millis(v).ok_or_else(|| Error::Api {
        status: 400,
        message: format!("{field} : date invalide « {v} » (ISO 8601 attendu)"),
    })
}
