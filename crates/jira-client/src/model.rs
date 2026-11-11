//! Types de domaine partagés avec la TUI (indépendants du JSON Jira).
//!
//! Conventions de dates : versions en `YYYY-MM-DD`, sprints en ISO 8601 (`2026-11-11T09:00:00.000Z`).

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct User {
    pub account_id: String,
    pub display_name: String,
    pub email: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
    pub id: String,
    pub key: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Board {
    pub id: u64,
    pub name: String,
    pub project_key: Option<String>,
    /// Type Jira du board : `scrum` (projet company-managed) ou `simple` (projet team-managed,
    /// sprints éventuellement désactivés : liste de sprints vide). `None` si non renvoyé.
    pub board_type: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SprintState {
    Active,
    Future,
    Closed,
}

impl SprintState {
    /// Valeur attendue par l'API Agile (`state=active,future`).
    pub fn as_api_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Future => "future",
            Self::Closed => "closed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sprint {
    pub id: u64,
    pub board_id: Option<u64>,
    pub name: String,
    pub state: SprintState,
    pub goal: Option<String>,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
}

/// Catégorie de statut Jira (`statusCategory.key` : `new` / `indeterminate` / `done`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StatusCategory {
    ToDo,
    InProgress,
    Done,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    pub id: String,
    pub name: String,
    pub category: StatusCategory,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueType {
    pub id: String,
    pub name: String,
    pub subtask: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionRef {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Issue {
    pub id: String,
    pub key: String,
    pub summary: String,
    /// Description convertie de l'ADF en texte brut (voir [`crate::adf`]).
    pub description: Option<String>,
    /// `true` si la description ADF ne contient que des paragraphes de texte simple : la réécrire depuis
    /// le texte brut ne perd rien. `false` (listes, titres, mise en forme, mentions…) : la TUI doit
    /// avertir avant d'écraser la description.
    pub description_is_lossless: bool,
    pub status: Status,
    pub issue_type: IssueType,
    pub priority: Option<String>,
    pub assignee: Option<User>,
    pub story_points: Option<f64>,
    pub fix_versions: Vec<VersionRef>,
    /// Sprint actif ou futur auquel l'issue appartient (None = backlog).
    pub sprint_id: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transition {
    pub id: String,
    pub name: String,
    pub to: Status,
    /// La transition ouvre un écran (ou exige des champs) : l'appliquer sans les renseigner peut
    /// échouer (400, ex. « résolution requise »). La TUI ne sait pas remplir ces écrans.
    pub has_screen: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub description: Option<String>,
    pub released: bool,
    pub archived: bool,
    pub start_date: Option<String>,
    pub release_date: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct NewIssue {
    pub project_key: String,
    pub issue_type_id: String,
    pub summary: String,
    pub description: Option<String>,
    pub story_points: Option<f64>,
    pub fix_version_ids: Vec<String>,
}

/// Modification partielle d'une issue : `None` = champ inchangé ; `Some(None)` = vider le champ.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct IssueUpdate {
    pub summary: Option<String>,
    pub description: Option<Option<String>>,
    pub story_points: Option<Option<f64>>,
    pub fix_version_ids: Option<Vec<String>>,
}

/// Position de rang : avant ou après une issue de référence (clé).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RankPosition {
    Before(String),
    After(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StartSprint {
    pub start_date: String,
    pub end_date: String,
    pub goal: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NewVersion {
    pub project_id: String,
    pub name: String,
    pub description: Option<String>,
    pub start_date: Option<String>,
    pub release_date: Option<String>,
}

/// Modification partielle d'une version (mêmes conventions que [`IssueUpdate`]).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct VersionUpdate {
    pub name: Option<String>,
    pub description: Option<Option<String>>,
    pub start_date: Option<Option<String>>,
    pub release_date: Option<Option<String>>,
    pub released: Option<bool>,
    pub archived: Option<bool>,
    /// Id d'une autre version du projet vers laquelle déplacer les issues non résolues.
    ///
    /// **N'a d'effet qu'avec `released: Some(true)` dans la même mise à jour, sur une version pas
    /// encore publiée** : Jira n'applique le déplacement qu'au moment de la publication (une version
    /// déjà publiée ignore une nouvelle demande de publication, et donc le déplacement).
    ///
    /// Jira déplace alors les issues sans résolution (non documenté précisément).
    /// Le client relit la version cible (`GET /rest/api/3/version/{id}`, une requête de plus) et
    /// envoie son lien `self` tel que renvoyé par Jira (champ `moveUnfixedIssuesTo`) ; l'URL n'est
    /// jamais reconstruite depuis l'URL de base, mais elle doit désigner le même hôte que celle-ci
    /// (sinon `Error::Decode`, rien n'est modifié). Cible inconnue : `Error::NotFound` (rien n'est
    /// modifié). Cible identique à la version modifiée : `Error::Invalid`. L'appartenance au même
    /// projet n'est pas vérifiée côté client.
    pub move_unfixed_issues_to: Option<String>,
}

/// Modification partielle d'un sprint (`POST /rest/agile/1.0/sprint/{id}`) : `None` = inchangé.
///
/// Sur un sprint clôturé, Jira n'applique que `name` et `goal` et **ignore silencieusement** les
/// dates (réponse 200). Le client compare alors les dates demandées à celles de la réponse et renvoie
/// `Error::Invalid` (« dates ignorées par Jira : sprint fermé ») si elles diffèrent ; le nom et
/// l'objectif ont néanmoins été enregistrés. La TUI devrait désactiver l'édition des dates d'un
/// sprint clôturé.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SprintUpdate {
    pub name: Option<String>,
    /// `Some(None)` = vider l'objectif.
    pub goal: Option<Option<String>>,
    /// ISO 8601 (`2026-11-16T08:00:00.000Z`).
    pub start_date: Option<String>,
    pub end_date: Option<String>,
}
