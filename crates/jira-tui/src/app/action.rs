//! Messages entre l'état (`App`) et le monde extérieur :
//! - [`Effect`] : travail asynchrone demandé par `update` (appel API), exécuté par le runtime ;
//! - [`Msg`] : résultat d'un effet, renvoyé dans la boucle par le canal mpsc ;
//! - [`WriteOp`] : opération d'écriture, toujours confirmée par l'utilisateur avant d'être émise.

use std::time::Instant;

use crossterm::event::KeyEvent;
use jiff::civil::Date;
use jira_client::model::*;

use super::state::BoardData;

/// Événement entrant dans la boucle principale.
#[derive(Debug)]
pub enum AppEvent {
    /// Touche et instant de sa lecture (horodatée par le thread clavier).
    Key(KeyEvent, Instant),
    /// Texte collé (bracketed paste) : jamais interprété comme des touches.
    Paste(String, Instant),
    Msg(Msg),
    /// Attente vide du clavier ; porte la date du jour (rafraîchie si minuit est passé).
    Tick(Date),
    Resize,
    /// Lecture du clavier impossible : on quitte avec une erreur.
    InputError(String),
}

/// Appel API à exécuter hors de la boucle UI. Chaque effet produit exactement un [`Msg`].
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    LoadProjects,
    LoadBoards {
        project_key: String,
    },
    /// `seq` : numéro de la requête ; seule la réponse à la dernière émise est acceptée.
    LoadBoard {
        seq: u64,
        board_id: u64,
        project_key: String,
    },
    LoadTransitions {
        key: String,
    },
    LoadVersionIssues {
        seq: u64,
        project_key: String,
        version_id: String,
    },
    Write(WriteOp),
}

/// Résultat d'un [`Effect`]. Les erreurs sont déjà converties en texte affichable.
#[derive(Debug)]
pub enum Msg {
    Projects(Result<Vec<Project>, String>),
    Boards {
        project_key: String,
        result: Result<Vec<Board>, String>,
    },
    Board {
        seq: u64,
        result: Result<Box<BoardData>, String>,
    },
    Transitions {
        key: String,
        result: Result<Vec<Transition>, String>,
    },
    VersionIssues {
        seq: u64,
        version_id: String,
        result: Result<Vec<Issue>, String>,
    },
    /// Écriture terminée ; `Ok(Some(id))` = identifiant créé (clé d'issue ou id de version).
    Written {
        op: WriteOp,
        result: Result<Option<String>, String>,
    },
    /// La tâche asynchrone a paniqué : état du terminal incertain, on quitte.
    TaskFailed(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum WriteOp {
    Transition {
        key: String,
        transition_id: String,
        transition_name: String,
        to_status: String,
    },
    MoveToSprint {
        keys: Vec<String>,
        sprint_id: u64,
        sprint_name: String,
    },
    MoveToBacklog {
        keys: Vec<String>,
    },
    Rank {
        key: String,
        position: RankPosition,
    },
    StartSprint {
        sprint_id: u64,
        sprint_name: String,
        params: StartSprint,
    },
    /// Clôture : les issues non terminées (hors sous-tâches, qui suivent leur parent) sont
    /// d'abord déplacées explicitement vers `destination` (`None` = backlog), puis le sprint est
    /// clôturé. Juste avant d'agir, le runtime relit le sprint et abandonne si l'ensemble des
    /// non terminées diffère de `unfinished` (celui que l'utilisateur a confirmé).
    CloseSprint {
        sprint_id: u64,
        sprint_name: String,
        unfinished: Vec<String>,
        destination: Option<(u64, String)>,
    },
    CreateIssue(NewIssue),
    UpdateIssue {
        key: String,
        update: IssueUpdate,
        /// Noms des versions envoyées (affichage), présent ssi `update.fix_version_ids` l'est.
        fix_version_names: Option<Vec<String>>,
    },
    CreateVersion(NewVersion),
    UpdateVersion {
        version_id: String,
        version_name: String,
        update: VersionUpdate,
    },
    /// Livraison : `released = true`, date du jour si aucune date prévue, et déplacement
    /// optionnel des issues non terminées vers `move_to` (id, nom) — fait par Jira lui-même
    /// (`moveUnfixedIssuesTo`).
    ReleaseVersion {
        version_id: String,
        version_name: String,
        release_date: Option<String>,
        unfinished: Vec<String>,
        move_to: Option<(String, String)>,
    },
}

fn opt(v: &Option<String>) -> String {
    v.as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or("(vide)")
        .to_string()
}

pub fn fmt_points(p: Option<f64>) -> String {
    match p {
        None => "-".into(),
        Some(p) if p.fract() == 0.0 => format!("{p:.0}"),
        Some(p) => format!("{p}"),
    }
}

impl WriteOp {
    /// Titre et lignes de la popup de confirmation.
    pub fn describe(&self) -> (String, Vec<String>) {
        match self {
            Self::Transition {
                key,
                transition_name,
                to_status,
                ..
            } => (
                "Transition".into(),
                vec![
                    format!("{key} : {transition_name}"),
                    format!("Nouveau statut : {to_status}"),
                ],
            ),
            Self::MoveToSprint {
                keys, sprint_name, ..
            } => (
                "Déplacer vers un sprint".into(),
                vec![format!("{} → {sprint_name}", keys.join(", "))],
            ),
            Self::MoveToBacklog { keys } => (
                "Renvoyer au backlog".into(),
                vec![format!("{} → Backlog", keys.join(", "))],
            ),
            Self::Rank { key, position } => {
                let line = match position {
                    RankPosition::Before(k) => format!("Placer {key} avant {k}"),
                    RankPosition::After(k) => format!("Placer {key} après {k}"),
                };
                ("Changer le rang".into(), vec![line])
            }
            Self::StartSprint {
                sprint_name,
                params,
                ..
            } => (
                "Démarrer le sprint".into(),
                vec![
                    sprint_name.clone(),
                    format!("Début : {}", params.start_date),
                    format!("Fin   : {}", params.end_date),
                    format!("Objectif : {}", opt(&params.goal)),
                ],
            ),
            Self::CloseSprint {
                sprint_name,
                unfinished,
                destination,
                ..
            } => {
                let dest = destination.as_ref().map_or("Backlog", |(_, n)| n.as_str());
                let mut lines = vec![format!("Clôturer {sprint_name}")];
                if unfinished.is_empty() {
                    lines.push("Toutes les issues sont terminées.".into());
                } else {
                    lines.push(format!(
                        "{} issue(s) non terminée(s) → {dest}",
                        unfinished.len()
                    ));
                    lines.push(unfinished.join(", "));
                }
                ("Clôturer le sprint".into(), lines)
            }
            Self::CreateIssue(n) => (
                "Créer une issue".into(),
                vec![
                    format!("Projet : {}", n.project_key),
                    format!("Résumé : {}", n.summary),
                    format!("Points : {}", fmt_points(n.story_points)),
                    format!(
                        "Description : {}",
                        opt(&n.description).lines().next().unwrap_or("")
                    ),
                ],
            ),
            Self::UpdateIssue {
                key,
                update,
                fix_version_names,
            } => {
                let mut lines = vec![format!("Modifier {key} :")];
                if let Some(s) = &update.summary {
                    lines.push(format!("  résumé → {s}"));
                }
                if let Some(p) = update.story_points {
                    lines.push(format!("  points → {}", fmt_points(p)));
                }
                if let Some(ids) = &update.fix_version_ids {
                    // Noms affichés ; à défaut (ne devrait pas arriver), les ids.
                    let names = fix_version_names.as_ref().unwrap_or(ids);
                    lines.push(format!(
                        "  version → {}",
                        if names.is_empty() {
                            "(aucune)".into()
                        } else {
                            names.join(", ")
                        }
                    ));
                }
                if update.description.is_some() {
                    lines.push("  description modifiée".into());
                }
                ("Modifier l'issue".into(), lines)
            }
            Self::CreateVersion(v) => (
                "Créer une version".into(),
                vec![
                    format!("Nom : {}", v.name),
                    format!("Début : {}", opt(&v.start_date)),
                    format!("Livraison prévue : {}", opt(&v.release_date)),
                ],
            ),
            Self::ReleaseVersion {
                version_name,
                release_date,
                unfinished,
                move_to,
                ..
            } => {
                let mut lines = vec![format!("Livrer la version {version_name}")];
                if let Some(d) = release_date {
                    lines.push(format!("Date de livraison : {d}"));
                }
                // Jira choisit lui-même les issues déplacées (non résolues) : le décompte local,
                // fondé sur la catégorie de statut, n'est qu'une estimation.
                if !unfinished.is_empty() {
                    let n = unfinished.len();
                    lines.push(match move_to {
                        Some((_, name)) => format!(
                            "Jira déplacera les issues non résolues (≈ {n} d'après leur statut) vers {name}"
                        ),
                        None => format!(
                            "Les issues non résolues (≈ {n} d'après leur statut) restent dans {version_name}"
                        ),
                    });
                }
                ("Livrer la version".into(), lines)
            }
            Self::UpdateVersion {
                version_name,
                update,
                ..
            } => {
                let mut lines = vec![format!("Modifier la version {version_name} :")];
                if let Some(n) = &update.name {
                    lines.push(format!("  nom → {n}"));
                }
                if let Some(d) = &update.release_date {
                    lines.push(format!("  livraison → {}", opt(d)));
                }
                if update.description.is_some() {
                    lines.push("  description modifiée".into());
                }
                ("Modifier la version".into(), lines)
            }
        }
    }

    /// Message de statut après succès.
    pub fn success_message(&self, created: Option<&str>) -> String {
        match self {
            Self::Transition { key, to_status, .. } => format!("{key} → {to_status}"),
            Self::MoveToSprint {
                keys, sprint_name, ..
            } => format!("{} déplacée(s) vers {sprint_name}", keys.join(", ")),
            Self::MoveToBacklog { keys } => format!("{} renvoyée(s) au backlog", keys.join(", ")),
            Self::Rank { key, .. } => format!("Rang de {key} mis à jour"),
            Self::StartSprint { sprint_name, .. } => format!("{sprint_name} démarré"),
            Self::CloseSprint { sprint_name, .. } => format!("{sprint_name} clôturé"),
            Self::CreateIssue(_) => format!("Issue {} créée", created.unwrap_or("?")),
            Self::UpdateIssue { key, .. } => format!("{key} modifiée"),
            Self::CreateVersion(v) => format!("Version {} créée", v.name),
            Self::UpdateVersion { version_name, .. } => format!("Version {version_name} modifiée"),
            Self::ReleaseVersion {
                version_name,
                move_to,
                unfinished,
                ..
            } => match move_to {
                Some((_, name)) if !unfinished.is_empty() => {
                    format!("Version {version_name} livrée (issues non résolues → {name})")
                }
                _ => format!("Version {version_name} livrée"),
            },
        }
    }

    /// Issue à garder sélectionnée après rafraîchissement.
    pub fn focus_key(&self) -> Option<&str> {
        match self {
            Self::Transition { key, .. }
            | Self::Rank { key, .. }
            | Self::UpdateIssue { key, .. } => Some(key),
            Self::MoveToSprint { keys, .. } | Self::MoveToBacklog { keys } => {
                keys.first().map(String::as_str)
            }
            _ => None,
        }
    }

    /// Libellé court pour les logs (sans contenu utilisateur volumineux).
    pub fn log_label(&self) -> String {
        match self {
            Self::Transition {
                key, transition_id, ..
            } => format!("transition {key} ({transition_id})"),
            Self::MoveToSprint {
                keys, sprint_id, ..
            } => format!("move {keys:?} -> sprint {sprint_id}"),
            Self::MoveToBacklog { keys } => format!("move {keys:?} -> backlog"),
            Self::Rank { key, position } => format!("rank {key} {position:?}"),
            Self::StartSprint { sprint_id, .. } => format!("start sprint {sprint_id}"),
            Self::CloseSprint {
                sprint_id,
                destination,
                ..
            } => format!(
                "close sprint {sprint_id} (dest {:?})",
                destination.as_ref().map(|d| d.0)
            ),
            Self::CreateIssue(n) => format!("create issue in {}", n.project_key),
            Self::UpdateIssue { key, .. } => format!("update issue {key}"),
            Self::CreateVersion(v) => format!("create version {}", v.name),
            Self::UpdateVersion { version_id, .. } => format!("update version {version_id}"),
            Self::ReleaseVersion {
                version_id,
                move_to,
                ..
            } => format!(
                "release version {version_id} (unfixed -> {:?})",
                move_to.as_ref().map(|m| &m.0)
            ),
        }
    }
}
