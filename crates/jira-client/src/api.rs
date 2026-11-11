use async_trait::async_trait;

use crate::Result;
use crate::model::*;

/// Contrat entre la TUI et Jira. Les listes renvoyées sont complètes (pagination gérée par l'implémentation)
/// et ordonnées comme dans Jira (rang pour les issues).
#[async_trait]
pub trait JiraApi: Send + Sync {
    // --- Compte / navigation ---
    async fn myself(&self) -> Result<User>;
    async fn projects(&self) -> Result<Vec<Project>>;
    async fn scrum_boards(&self, project_key: &str) -> Result<Vec<Board>>;

    // --- Backlog & sprints ---
    /// Sprints du board filtrés par état ; `states` vide = tous les états.
    async fn sprints(&self, board_id: u64, states: &[SprintState]) -> Result<Vec<Sprint>>;
    /// Issues du backlog du board : issues du filtre du board hors sprint actif/futur, non terminées
    /// (catégorie de statut ≠ Done), sous-tâches exclues, triées par rang.
    async fn backlog_issues(&self, board_id: u64) -> Result<Vec<Issue>>;
    /// Issues du sprint (y compris, pour un sprint clôturé, celles qui en sont sorties ensuite), par rang.
    async fn sprint_issues(&self, sprint_id: u64) -> Result<Vec<Issue>>;
    /// Déplace les issues dans un sprint actif ou futur. Envoyé par lots de 50 : non atomique au-delà.
    async fn move_to_sprint(&self, sprint_id: u64, issue_keys: &[String]) -> Result<()>;
    /// Retire les issues de leurs sprints actifs/futurs. Lots de 50, non atomique au-delà.
    async fn move_to_backlog(&self, issue_keys: &[String]) -> Result<()>;
    /// Place les issues (dans l'ordre donné) avant/après l'issue de référence, qui ne doit pas en faire
    /// partie. Un succès partiel de Jira (207) est renvoyé en erreur listant les clés refusées.
    async fn rank(&self, issue_keys: &[String], position: &RankPosition) -> Result<()>;
    async fn start_sprint(&self, sprint_id: u64, params: &StartSprint) -> Result<Sprint>;
    /// Clôture le sprint actif, sans rien faire d'autre. Le sort des issues non terminées n'est pas
    /// documenté par l'API : **la TUI doit déplacer explicitement les issues non terminées (vers le
    /// sprint suivant ou le backlog) avant de clôturer.**
    async fn close_sprint(&self, sprint_id: u64) -> Result<Sprint>;
    /// Crée un sprint futur sur le board.
    async fn create_sprint(
        &self,
        board_id: u64,
        name: &str,
        goal: Option<String>,
    ) -> Result<Sprint>;
    /// Mise à jour partielle (nom, objectif, dates) ; ne change jamais l'état du sprint.
    /// Sprint clôturé : Jira ignore les dates sans erreur ; si des dates demandées diffèrent de la
    /// réponse, renvoie `Error::Invalid` (nom/objectif déjà appliqués). Voir [`SprintUpdate`].
    async fn update_sprint(&self, sprint_id: u64, update: &SprintUpdate) -> Result<Sprint>;

    // --- Issues ---
    async fn issue_types(&self, project_key: &str) -> Result<Vec<IssueType>>;
    /// Crée une issue et renvoie sa clé (ex. `DEMO-42`).
    async fn create_issue(&self, issue: &NewIssue) -> Result<String>;
    async fn update_issue(&self, key: &str, update: &IssueUpdate) -> Result<()>;
    /// Transitions disponibles dans l'état courant (à relire avant chaque transition).
    async fn transitions(&self, key: &str) -> Result<Vec<Transition>>;
    /// Un droit manquant est signalé par Jira en 400 (pas 403) : voir
    /// [`crate::Error::maybe_permission`].
    async fn transition_issue(&self, key: &str, transition_id: &str) -> Result<()>;

    // --- Versions ---
    async fn versions(&self, project_key: &str) -> Result<Vec<Version>>;
    async fn version_issues(&self, project_key: &str, version_id: &str) -> Result<Vec<Issue>>;
    async fn create_version(&self, version: &NewVersion) -> Result<Version>;
    /// Mise à jour partielle. Lors d'une release avec `move_unfixed_issues_to`, Jira déplace les
    /// issues sans résolution (non documenté précisément). Avec `move_unfixed_issues_to`, relit d'abord la version cible pour
    /// obtenir son lien `self` (voir [`VersionUpdate::move_unfixed_issues_to`]). Un droit manquant
    /// est signalé par Jira en 400 (pas 403) : voir [`crate::Error::maybe_permission`].
    async fn update_version(&self, version_id: &str, update: &VersionUpdate) -> Result<Version>;
    /// Supprime la version (`POST /rest/api/3/version/{id}/removeAndSwap`).
    ///
    /// Si `move_fix_issues_to` est fourni (id d'une autre version du projet), les issues dont c'était
    /// la fixVersion y sont rattachées. **Dans tous les cas**, les références dans les *Affects
    /// versions* et dans les champs custom de type version (version picker) sont **retirées** sans
    /// remplacement ; sans cible, la fixVersion est elle aussi simplement retirée.
    /// Un droit manquant est signalé par Jira en 404 (pas 403) : voir
    /// [`crate::Error::maybe_permission`].
    async fn delete_version(
        &self,
        version_id: &str,
        move_fix_issues_to: Option<String>,
    ) -> Result<()>;
}
