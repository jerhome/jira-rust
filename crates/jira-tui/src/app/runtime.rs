//! Exécution des [`Effect`] contre `JiraApi`. Seul module qui appelle l'API.

use jira_client::model::*;
use jira_client::{Error, JiraApi};

use super::action::{Effect, Msg, WriteOp};
use super::state::{BoardData, unfinished_keys};

/// Message d'erreur affichable. Les refus de droits reçoivent une explication : certaine pour
/// un 403, hypothétique pour un 400/404 dont le message évoque une permission (Jira ne renvoie
/// pas toujours 403, voir [`Error::maybe_permission`]).
pub fn error_text(e: &Error) -> String {
    match e {
        Error::Forbidden(_) => format!("{e} — droits Jira insuffisants pour cette action"),
        Error::Unauthorized => format!("{e} — relancer `jira-rust doctor`"),
        _ if e.maybe_permission() => format!("{e} — droits Jira insuffisants ?"),
        _ => e.to_string(),
    }
}

fn text<T>(r: jira_client::Result<T>) -> Result<T, String> {
    r.map_err(|e| error_text(&e))
}

pub async fn execute(api: &dyn JiraApi, effect: Effect) -> Msg {
    match effect {
        Effect::LoadProjects => Msg::Projects(text(api.projects().await)),
        Effect::LoadBoards { project_key } => {
            let result = text(api.scrum_boards(&project_key).await);
            Msg::Boards {
                project_key,
                result,
            }
        }
        Effect::LoadBoard {
            seq,
            board_id,
            project_key,
        } => {
            let result = text(load_board(api, board_id, &project_key).await).map(Box::new);
            Msg::Board { seq, result }
        }
        Effect::LoadTransitions { key } => {
            let result = text(api.transitions(&key).await);
            Msg::Transitions { key, result }
        }
        Effect::LoadVersionIssues {
            seq,
            project_key,
            version_id,
        } => {
            let result = text(api.version_issues(&project_key, &version_id).await);
            Msg::VersionIssues {
                seq,
                version_id,
                result,
            }
        }
        Effect::Write(op) => {
            let result = write(api, &op).await;
            if let Err(e) = &result {
                tracing::warn!(op = %op.log_label(), error = %e, "écriture échouée");
            }
            Msg::Written { op, result }
        }
    }
}

async fn load_board(
    api: &dyn JiraApi,
    board_id: u64,
    project_key: &str,
) -> jira_client::Result<BoardData> {
    let (sprints, backlog, versions, issue_types) = tokio::try_join!(
        api.sprints(board_id, &[SprintState::Active, SprintState::Future]),
        api.backlog_issues(board_id),
        api.versions(project_key),
        api.issue_types(project_key),
    )?;
    // Piège Jira (docs/api/sprints.md) : un board liste aussi les sprints d'autres boards dont
    // les filtres se recoupent. On n'agit que sur les sprints d'origine = ce board ; un
    // `board_id` absent est traité comme étranger (prudence).
    let (own, foreign): (Vec<Sprint>, Vec<Sprint>) = sprints
        .into_iter()
        .partition(|s| s.board_id == Some(board_id));
    if !foreign.is_empty() {
        tracing::info!(
            board = board_id,
            ignored = ?foreign.iter().map(|s| s.id).collect::<Vec<_>>(),
            "sprints d'autres boards ignorés"
        );
    }
    let mut actives = own
        .iter()
        .filter(|s| s.state == SprintState::Active)
        .cloned();
    let active = actives.next();
    let parallel_active: Vec<Sprint> = actives.collect();
    let future: Vec<Sprint> = own
        .into_iter()
        .filter(|s| s.state == SprintState::Future)
        .collect();
    let mut sprint_issues = std::collections::HashMap::new();
    for s in active.iter().chain(future.iter()) {
        sprint_issues.insert(s.id, api.sprint_issues(s.id).await?);
    }
    Ok(BoardData {
        active,
        parallel_active,
        foreign_sprints: foreign.len(),
        future,
        sprint_issues,
        backlog,
        versions,
        issue_types,
    })
}

async fn write(api: &dyn JiraApi, op: &WriteOp) -> Result<Option<String>, String> {
    if let WriteOp::CloseSprint {
        sprint_id,
        sprint_name,
        unfinished,
        ..
    } = op
    {
        check_close_unchanged(api, *sprint_id, sprint_name, unfinished).await?;
    }
    text(write_unchecked(api, op).await)
}

/// Relit le sprint juste avant la clôture : si l'ensemble des issues non terminées n'est plus
/// celui que l'utilisateur a confirmé (issue ajoutée, terminée, déplacée entre-temps, par lui
/// ou par un collègue), on abandonne sans rien écrire.
async fn check_close_unchanged(
    api: &dyn JiraApi,
    sprint_id: u64,
    sprint_name: &str,
    confirmed: &[String],
) -> Result<(), String> {
    use std::collections::BTreeSet;
    let current = text(api.sprint_issues(sprint_id).await)?;
    let now: BTreeSet<String> = unfinished_keys(&current).into_iter().collect();
    let expected: BTreeSet<String> = confirmed.iter().cloned().collect();
    if now == expected {
        return Ok(());
    }
    let list = |s: Vec<&String>| {
        if s.is_empty() {
            "aucune".to_string()
        } else {
            s.into_iter().cloned().collect::<Vec<_>>().join(", ")
        }
    };
    let added = list(now.difference(&expected).collect());
    let removed = list(expected.difference(&now).collect());
    tracing::warn!(sprint = sprint_id, %added, %removed, "clôture abandonnée : sprint modifié");
    Err(format!(
        "clôture de {sprint_name} abandonnée, rien n'a été modifié : les issues non terminées \
         ont changé depuis la confirmation (en plus : {added} ; en moins : {removed}). \
         Vérifiez puis recommencez (x)"
    ))
}

async fn write_unchecked(api: &dyn JiraApi, op: &WriteOp) -> jira_client::Result<Option<String>> {
    match op {
        WriteOp::Transition {
            key, transition_id, ..
        } => {
            api.transition_issue(key, transition_id).await?;
        }
        WriteOp::MoveToSprint {
            keys, sprint_id, ..
        } => api.move_to_sprint(*sprint_id, keys).await?,
        WriteOp::MoveToBacklog { keys } => api.move_to_backlog(keys).await?,
        WriteOp::Rank { key, position } => api.rank(std::slice::from_ref(key), position).await?,
        WriteOp::StartSprint {
            sprint_id, params, ..
        } => {
            api.start_sprint(*sprint_id, params).await?;
        }
        WriteOp::CloseSprint {
            sprint_id,
            unfinished,
            destination,
            ..
        } => {
            // close_sprint ne déplace pas les issues : on le fait explicitement avant.
            if !unfinished.is_empty() {
                match destination {
                    Some((dest, _)) => api.move_to_sprint(*dest, unfinished).await?,
                    None => api.move_to_backlog(unfinished).await?,
                }
            }
            api.close_sprint(*sprint_id).await?;
        }
        WriteOp::CreateIssue(new) => return api.create_issue(new).await.map(Some),
        WriteOp::UpdateIssue { key, update, .. } => api.update_issue(key, update).await?,
        WriteOp::CreateVersion(new) => return api.create_version(new).await.map(|v| Some(v.id)),
        WriteOp::UpdateVersion {
            version_id, update, ..
        } => {
            api.update_version(version_id, update).await?;
        }
        WriteOp::ReleaseVersion {
            version_id,
            release_date,
            move_to,
            ..
        } => {
            let update = VersionUpdate {
                released: Some(true),
                release_date: release_date.clone().map(Some),
                move_unfixed_issues_to: move_to.as_ref().map(|(id, _)| id.clone()),
                ..VersionUpdate::default()
            };
            api.update_version(version_id, &update).await?;
        }
    }
    Ok(None)
}
