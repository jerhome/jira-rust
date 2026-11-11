//! État de l'application, indépendant du terminal : données chargées, onglets, curseurs, popups.

use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant};

use jiff::civil::Date;
use jiff::tz::TimeZone;
use jira_client::model::*;

use super::action::WriteOp;
use super::form::Form;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Backlog,
    Sprint,
    Versions,
}

impl Tab {
    pub const ALL: [Tab; 3] = [Tab::Backlog, Tab::Sprint, Tab::Versions];

    pub fn title(self) -> &'static str {
        match self {
            Tab::Backlog => "Backlog",
            Tab::Sprint => "Sprint actif",
            Tab::Versions => "Versions",
        }
    }

    pub fn index(self) -> usize {
        Self::ALL.iter().position(|t| *t == self).unwrap_or(0)
    }

    pub fn cycle(self, forward: bool) -> Tab {
        let i = self.index();
        let n = Self::ALL.len();
        Self::ALL[if forward {
            (i + 1) % n
        } else {
            (i + n - 1) % n
        }]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Picker,
    Main,
}

/// Sélection projet → board.
#[derive(Debug, Default)]
pub struct Picker {
    pub projects: Vec<Project>,
    /// Projet choisi : on affiche alors la liste de ses boards.
    pub project: Option<Project>,
    pub boards: Vec<Board>,
    pub cursor: usize,
    pub loading: bool,
    /// Au démarrage seulement : sauter l'étape projet s'il n'y en a qu'un.
    pub auto: bool,
}

impl Picker {
    pub fn len(&self) -> usize {
        if self.project.is_some() {
            self.boards.len()
        } else {
            self.projects.len()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Context {
    pub project: Project,
    pub board: Board,
}

/// Données d'un board, chargées d'un bloc et remplacées à chaque rafraîchissement.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BoardData {
    /// Sprint actif du board courant (le premier si plusieurs, voir `parallel_active`).
    pub active: Option<Sprint>,
    /// Autres sprints actifs du même board (option « sprints parallèles ») : non affichés ;
    /// leur présence désactive la clôture (on ne choisit pas un sprint au hasard).
    pub parallel_active: Vec<Sprint>,
    /// Sprints listés par le board mais dont le board d'origine est différent (sprints partagés
    /// entre boards aux filtres recoupés) : ignorés, la TUI n'agit jamais dessus.
    pub foreign_sprints: usize,
    pub future: Vec<Sprint>,
    pub sprint_issues: HashMap<u64, Vec<Issue>>,
    pub backlog: Vec<Issue>,
    pub versions: Vec<Version>,
    pub issue_types: Vec<IssueType>,
}

impl BoardData {
    /// Issues d'un conteneur (`None` = backlog), dans l'ordre de rang.
    pub fn issues_in(&self, loc: Option<u64>) -> &[Issue] {
        match loc {
            None => &self.backlog,
            Some(id) => self.sprint_issues.get(&id).map_or(&[], Vec::as_slice),
        }
    }

    pub fn sprint_name(&self, id: u64) -> Option<&str> {
        self.active
            .iter()
            .chain(self.future.iter())
            .find(|s| s.id == id)
            .map(|s| s.name.as_str())
    }

    pub fn find_issue(&self, key: &str) -> Option<&Issue> {
        self.backlog
            .iter()
            .chain(self.sprint_issues.values().flatten())
            .find(|i| i.key == key)
    }

    /// Versions affichées : non livrées puis livrées (archivées exclues), ordre Jira conservé.
    pub fn versions_ordered(&self) -> Vec<&Version> {
        let live = self.versions.iter().filter(|v| !v.archived);
        live.clone()
            .filter(|v| !v.released)
            .chain(live.filter(|v| v.released))
            .collect()
    }
}

/// Clés des issues non terminées à déplacer à la clôture d'un sprint : hors catégorie « Terminé »
/// et hors sous-tâches (elles suivent leur parent ; Jira refuse de les déplacer seules).
pub fn unfinished_keys(issues: &[Issue]) -> Vec<String> {
    issues
        .iter()
        .filter(|i| i.status.category != StatusCategory::Done && !i.issue_type.subtask)
        .map(|i| i.key.clone())
        .collect()
}

pub fn points_sum<'a>(issues: impl IntoIterator<Item = &'a Issue>) -> f64 {
    issues.into_iter().filter_map(|i| i.story_points).sum()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BacklogRow {
    SprintHeader(u64),
    BacklogHeader,
    Issue { loc: Option<u64>, index: usize },
}

#[derive(Debug, Default)]
pub struct BacklogView {
    pub cursor: usize,
    pub collapsed: HashSet<u64>,
}

#[derive(Debug, Default)]
pub struct SprintView {
    pub column: usize,
    pub rows: [usize; 3],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VersionsFocus {
    #[default]
    List,
    Issues,
}

#[derive(Debug, Default)]
pub struct VersionsView {
    pub cursor: usize,
    pub focus: VersionsFocus,
    pub issue_cursor: usize,
    pub issues: Vec<Issue>,
    /// Version dont `issues` est le contenu.
    pub issues_for: Option<String>,
    /// Version en cours de chargement (les réponses d'autres versions sont ignorées).
    pub loading_for: Option<String>,
    /// Le dernier chargement de `issues_for` a échoué (`issues` vide mais non fiable).
    pub issues_failed: bool,
}

pub const COLUMNS: [(StatusCategory, &str); 3] = [
    (StatusCategory::ToDo, "À faire"),
    (StatusCategory::InProgress, "En cours"),
    (StatusCategory::Done, "Terminé"),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MoveDest {
    Sprint { id: u64, name: String },
    Backlog,
}

impl MoveDest {
    pub fn label(&self) -> &str {
        match self {
            MoveDest::Sprint { name, .. } => name,
            MoveDest::Backlog => "Backlog",
        }
    }
}

/// Destination des issues non terminées d'une version livrée : `id == None` = les laisser.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionDest {
    pub id: Option<String>,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Popup {
    Help,
    Detail(Box<Issue>),
    /// `items == None` : transitions en cours de chargement.
    Transitions {
        key: String,
        items: Option<Vec<Transition>>,
        cursor: usize,
    },
    Move {
        keys: Vec<String>,
        dests: Vec<MoveDest>,
        cursor: usize,
    },
    PickSprint {
        sprints: Vec<Sprint>,
        cursor: usize,
    },
    CloseSprint {
        sprint: Box<Sprint>,
        unfinished: Vec<String>,
        done: usize,
        dests: Vec<MoveDest>,
        cursor: usize,
    },
    /// Livraison d'une version contenant des issues non terminées : choix de leur destination.
    ReleaseVersion {
        version: Box<Version>,
        unfinished: Vec<String>,
        dests: Vec<VersionDest>,
        cursor: usize,
    },
    Form(Box<Form>),
    Confirm {
        op: WriteOp,
        /// Popup à rouvrir en cas d'annulation (formulaire, liste…).
        back: Option<Box<Popup>>,
        /// Instant du premier affichage effectif (posé après `terminal.draw`, voir
        /// [`App::mark_drawn`]), repoussé par toute frappe reçue trop tôt. `None` = jamais
        /// dessinée : aucune validation possible.
        shown_at: Option<Instant>,
    },
    /// Ctrl+C alors qu'un formulaire contient des modifications : confirmer la sortie.
    QuitConfirm {
        back: Box<Popup>,
    },
}

/// Délai minimal entre l'affichage d'une confirmation (ou la dernière frappe reçue pendant
/// qu'elle est ouverte) et sa validation : une validation plus rapide n'a pas pu être lue.
pub const CONFIRM_DELAY: Duration = Duration::from_millis(400);

impl Popup {
    pub fn confirm(op: WriteOp, back: Option<Popup>) -> Self {
        Popup::Confirm {
            op,
            back: back.map(Box::new),
            shown_at: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StatusLine {
    pub text: String,
    pub error: bool,
}

pub struct App {
    pub screen: Screen,
    pub picker: Picker,
    pub ctx: Option<Context>,
    pub tab: Tab,
    pub data: BoardData,
    pub loaded: bool,
    pub backlog: BacklogView,
    pub sprint: SprintView,
    pub versions: VersionsView,
    pub popup: Option<Popup>,
    pub status: StatusLine,
    /// Nombre d'effets dont on attend le résultat (indicateur de chargement).
    pub pending: usize,
    pub spinner: usize,
    pub should_quit: bool,
    pub fatal: Option<String>,
    pub today: Date,
    pub tz: TimeZone,
    /// Numéro de la dernière requête de board émise : toute autre réponse est périmée
    /// (ancien board, ou rafraîchissement dépassé par un plus récent).
    pub board_seq: u64,
    /// Numéro de la dernière requête d'issues de version émise.
    pub version_seq: u64,
    /// Une écriture est en cours : les suivantes attendent dans `write_queue` (ordre conservé).
    pub write_in_flight: bool,
    pub write_queue: VecDeque<WriteOp>,
    /// Sortie demandée pendant une écriture : effective quand la file est vide.
    pub quit_after_writes: bool,
    /// Issue à re-sélectionner au prochain chargement.
    pub focus_key: Option<String>,
    /// Version à re-sélectionner au prochain chargement.
    pub focus_version: Option<String>,
    /// Lecture seule : aucune écriture n'est lancée (le client HTTP la refuse aussi).
    pub read_only: bool,
}

impl App {
    pub fn new(today: Date, tz: TimeZone) -> Self {
        Self {
            screen: Screen::Picker,
            picker: Picker {
                auto: true,
                ..Picker::default()
            },
            ctx: None,
            tab: Tab::Backlog,
            data: BoardData::default(),
            loaded: false,
            backlog: BacklogView::default(),
            sprint: SprintView::default(),
            versions: VersionsView::default(),
            popup: None,
            status: StatusLine::default(),
            pending: 0,
            spinner: 0,
            should_quit: false,
            fatal: None,
            today,
            tz,
            board_seq: 0,
            version_seq: 0,
            write_in_flight: false,
            write_queue: VecDeque::new(),
            quit_after_writes: false,
            focus_key: None,
            focus_version: None,
            read_only: false,
        }
    }

    /// Appelé après chaque `terminal.draw` : une confirmation affichée devient validable
    /// (après [`CONFIRM_DELAY`]).
    pub fn mark_drawn(&mut self, now: Instant) {
        if let Some(Popup::Confirm { shown_at, .. }) = &mut self.popup
            && shown_at.is_none()
        {
            *shown_at = Some(now);
        }
    }

    /// Écritures en cours ou en attente.
    pub fn writes_pending(&self) -> usize {
        usize::from(self.write_in_flight) + self.write_queue.len()
    }

    pub fn info(&mut self, text: impl Into<String>) {
        self.status = StatusLine {
            text: text.into(),
            error: false,
        };
    }

    pub fn error(&mut self, text: impl Into<String>) {
        let text = text.into();
        tracing::warn!(error = %text, "erreur affichée");
        self.status = StatusLine { text, error: true };
    }

    // --- Backlog ---

    pub fn backlog_rows(&self) -> Vec<BacklogRow> {
        let mut rows = Vec::new();
        for s in &self.data.future {
            rows.push(BacklogRow::SprintHeader(s.id));
            if !self.backlog.collapsed.contains(&s.id) {
                let n = self.data.issues_in(Some(s.id)).len();
                rows.extend((0..n).map(|index| BacklogRow::Issue {
                    loc: Some(s.id),
                    index,
                }));
            }
        }
        rows.push(BacklogRow::BacklogHeader);
        rows.extend(
            (0..self.data.backlog.len()).map(|index| BacklogRow::Issue { loc: None, index }),
        );
        rows
    }

    pub fn backlog_row(&self) -> Option<BacklogRow> {
        self.backlog_rows().get(self.backlog.cursor).copied()
    }

    /// Sprint futur « courant » dans le backlog (en-tête ou issue sélectionnée).
    pub fn backlog_current_sprint(&self) -> Option<u64> {
        match self.backlog_row()? {
            BacklogRow::SprintHeader(id) => Some(id),
            BacklogRow::Issue { loc, .. } => loc,
            BacklogRow::BacklogHeader => None,
        }
    }

    // --- Sprint actif ---

    pub fn sprint_columns(&self) -> [Vec<&Issue>; 3] {
        let issues = self
            .data
            .active
            .as_ref()
            .map_or(&[][..], |s| self.data.issues_in(Some(s.id)));
        COLUMNS.map(|(cat, _)| issues.iter().filter(|i| i.status.category == cat).collect())
    }

    // --- Versions ---

    pub fn selected_version(&self) -> Option<&Version> {
        self.data
            .versions_ordered()
            .get(self.versions.cursor)
            .copied()
    }

    // --- Sélection ---

    /// Issue sélectionnée dans l'onglet courant.
    pub fn selected_issue(&self) -> Option<&Issue> {
        match self.tab {
            Tab::Backlog => match self.backlog_row()? {
                BacklogRow::Issue { loc, index } => self.data.issues_in(loc).get(index),
                _ => None,
            },
            Tab::Sprint => {
                let col = self.sprint.column;
                self.sprint_columns()[col]
                    .get(self.sprint.rows[col])
                    .copied()
            }
            Tab::Versions => match self.versions.focus {
                VersionsFocus::Issues => self.versions.issues.get(self.versions.issue_cursor),
                VersionsFocus::List => None,
            },
        }
    }

    /// Voisins (précédent, suivant) de l'issue sélectionnée dans sa liste ordonnée.
    pub fn selected_neighbors(&self) -> Option<(Option<&Issue>, Option<&Issue>)> {
        let (list, idx): (Vec<&Issue>, usize) = match self.tab {
            Tab::Backlog => match self.backlog_row()? {
                BacklogRow::Issue { loc, index } => {
                    (self.data.issues_in(loc).iter().collect(), index)
                }
                _ => return None,
            },
            Tab::Sprint => {
                let col = self.sprint.column;
                let mut cols = self.sprint_columns();
                (std::mem::take(&mut cols[col]), self.sprint.rows[col])
            }
            Tab::Versions => return None,
        };
        list.get(idx)?;
        let prev = idx.checked_sub(1).and_then(|i| list.get(i).copied());
        Some((prev, list.get(idx + 1).copied()))
    }

    /// Ramène les curseurs dans les bornes et re-sélectionne `focus_key` / `focus_version` si demandé.
    pub fn reconcile_cursors(&mut self) {
        if let Some(key) = self.focus_key.take() {
            if let Some(issue) = self.data.find_issue(&key)
                && let Some(sid) = issue.sprint_id
            {
                self.backlog.collapsed.remove(&sid);
            }
            let rows = self.backlog_rows();
            if let Some(pos) = rows.iter().position(|r| match r {
                BacklogRow::Issue { loc, index } => self
                    .data
                    .issues_in(*loc)
                    .get(*index)
                    .is_some_and(|i| i.key == key),
                _ => false,
            }) {
                self.backlog.cursor = pos;
            }
            let found = self
                .sprint_columns()
                .iter()
                .enumerate()
                .find_map(|(c, col)| col.iter().position(|i| i.key == key).map(|r| (c, r)));
            if let Some((c, r)) = found {
                self.sprint.column = c;
                self.sprint.rows[c] = r;
            }
        }
        if let Some(id) = self.focus_version.take()
            && let Some(pos) = self.data.versions_ordered().iter().position(|v| v.id == id)
        {
            self.versions.cursor = pos;
        }

        let n = self.backlog_rows().len();
        self.backlog.cursor = self.backlog.cursor.min(n.saturating_sub(1));
        let lens = self.sprint_columns().map(|c| c.len());
        for (row, len) in self.sprint.rows.iter_mut().zip(lens) {
            *row = (*row).min(len.saturating_sub(1));
        }
        let nv = self.data.versions_ordered().len();
        self.versions.cursor = self.versions.cursor.min(nv.saturating_sub(1));
        self.versions.issue_cursor = self
            .versions
            .issue_cursor
            .min(self.versions.issues.len().saturating_sub(1));
    }
}
