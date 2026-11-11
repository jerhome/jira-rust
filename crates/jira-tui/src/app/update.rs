//! Logique pure : `update(app, event) -> effets`. Aucune E/S ici.

use std::time::Instant;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use jira_client::model::*;

use super::action::{AppEvent, Effect, Msg, WriteOp};
use super::form::{Form, FormOutcome};
use super::state::*;

/// Point d'entrée unique de la logique. Tient à jour le compteur d'effets en attente.
pub fn update(app: &mut App, event: AppEvent) -> Vec<Effect> {
    let effects = match event {
        AppEvent::Key(key, at) => app.on_key(key, at),
        AppEvent::Paste(text, at) => {
            app.on_paste(&text, at);
            Vec::new()
        }
        AppEvent::Msg(msg) => {
            app.pending = app.pending.saturating_sub(1);
            app.on_msg(msg)
        }
        AppEvent::Tick(today) => {
            app.spinner = app.spinner.wrapping_add(1);
            if today != app.today {
                tracing::info!(%today, "changement de date");
                app.today = today;
            }
            Vec::new()
        }
        AppEvent::Resize => Vec::new(),
        AppEvent::InputError(e) => {
            tracing::error!(error = %e, "lecture du clavier impossible");
            app.fatal = Some(format!("lecture du clavier impossible : {e}"));
            app.should_quit = true;
            Vec::new()
        }
    };
    app.pending += effects.len();
    effects
}

/// Effets initiaux (chargement des projets).
pub fn init(app: &mut App) -> Vec<Effect> {
    app.picker.loading = true;
    app.pending += 1;
    vec![Effect::LoadProjects]
}

fn step(cursor: &mut usize, len: usize, delta: isize) {
    if len == 0 {
        *cursor = 0;
        return;
    }
    let next = (*cursor as isize + delta).clamp(0, len as isize - 1);
    *cursor = next as usize;
}

/// Navigation dans une liste de popup : renvoie `Some(true)` pour Entrée, `Some(false)` pour Échap.
fn list_nav(key: KeyEvent, cursor: &mut usize, len: usize) -> Option<bool> {
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => step(cursor, len, 1),
        KeyCode::Char('k') | KeyCode::Up => step(cursor, len, -1),
        KeyCode::Char('g') | KeyCode::Home => *cursor = 0,
        KeyCode::Char('G') | KeyCode::End => *cursor = len.saturating_sub(1),
        KeyCode::Enter => return Some(true),
        KeyCode::Esc | KeyCode::Char('q') => return Some(false),
        _ => {}
    }
    None
}

impl App {
    fn on_key(&mut self, key: KeyEvent, at: Instant) -> Vec<Effect> {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            match self.popup.take() {
                Some(p) if has_unsaved_form(&p) => {
                    self.popup = Some(Popup::QuitConfirm { back: Box::new(p) })
                }
                Some(Popup::QuitConfirm { .. }) | None => self.request_quit(),
                Some(p) => {
                    self.popup = Some(p);
                    self.request_quit();
                }
            }
            return Vec::new();
        }
        if let Some(popup) = self.popup.take() {
            return self.on_popup_key(popup, key, at);
        }
        match self.screen {
            Screen::Picker => self.on_picker_key(key),
            Screen::Main => self.on_main_key(key),
        }
    }

    /// Sortie demandée : immédiate, sauf si une écriture est en cours ou en file (elle serait
    /// interrompue sans qu'on sache si Jira l'a appliquée) → sortie différée.
    fn request_quit(&mut self) {
        let n = self.writes_pending();
        if n == 0 {
            self.should_quit = true;
            return;
        }
        self.quit_after_writes = true;
        tracing::info!(writes = n, "sortie différée : écriture en cours");
        self.info(format!(
            "Écriture en cours ({n}) : patienter, sortie automatique à la fin"
        ));
    }

    /// Texte collé : inséré dans le champ actif d'un formulaire, ignoré ailleurs (jamais
    /// rejoué comme des frappes : un saut de ligne collé ne valide rien).
    fn on_paste(&mut self, text: &str, at: Instant) {
        match &mut self.popup {
            Some(Popup::Form(form)) => form.paste(text),
            Some(Popup::Confirm { shown_at, .. }) => rearm(shown_at, at),
            _ => {}
        }
    }

    // ------------------------------------------------------------------ Sélecteur

    fn on_picker_key(&mut self, key: KeyEvent) -> Vec<Effect> {
        let len = self.picker.len();
        match key.code {
            KeyCode::Char('q') => self.request_quit(),
            KeyCode::Char('?') => self.popup = Some(Popup::Help),
            KeyCode::Char('j') | KeyCode::Down => step(&mut self.picker.cursor, len, 1),
            KeyCode::Char('k') | KeyCode::Up => step(&mut self.picker.cursor, len, -1),
            KeyCode::Enter if !self.picker.loading => {
                if self.picker.project.is_some() {
                    return self.choose_board(self.picker.cursor);
                }
                return self.choose_project(self.picker.cursor);
            }
            KeyCode::Char('R') => {
                self.picker = Picker {
                    loading: true,
                    ..Picker::default()
                };
                return vec![Effect::LoadProjects];
            }
            KeyCode::Esc => {
                if self.picker.project.take().is_some() {
                    self.picker.cursor = 0;
                } else if self.ctx.is_some() {
                    self.screen = Screen::Main;
                }
            }
            _ => {}
        }
        Vec::new()
    }

    fn open_picker(&mut self) -> Vec<Effect> {
        self.screen = Screen::Picker;
        self.picker.project = None;
        self.picker.auto = false;
        self.picker.cursor = self
            .ctx
            .as_ref()
            .and_then(|c| {
                self.picker
                    .projects
                    .iter()
                    .position(|p| p.key == c.project.key)
            })
            .unwrap_or(0);
        if self.picker.projects.is_empty() {
            self.picker.loading = true;
            return vec![Effect::LoadProjects];
        }
        Vec::new()
    }

    fn choose_project(&mut self, index: usize) -> Vec<Effect> {
        let Some(project) = self.picker.projects.get(index).cloned() else {
            return Vec::new();
        };
        let project_key = project.key.clone();
        self.picker.project = Some(project);
        self.picker.boards.clear();
        self.picker.cursor = 0;
        self.picker.loading = true;
        vec![Effect::LoadBoards { project_key }]
    }

    fn choose_board(&mut self, index: usize) -> Vec<Effect> {
        let (Some(project), Some(board)) = (
            self.picker.project.clone(),
            self.picker.boards.get(index).cloned(),
        ) else {
            return Vec::new();
        };
        tracing::info!(project = %project.key, board = board.id, "board sélectionné");
        self.ctx = Some(Context { project, board });
        self.screen = Screen::Main;
        self.picker.auto = false;
        self.data = BoardData::default();
        self.loaded = false;
        self.backlog = BacklogView::default();
        self.sprint = SprintView::default();
        self.versions = VersionsView::default();
        self.info("");
        self.load_board().into_iter().collect()
    }

    /// Nouvelle requête de board : toute réponse à une requête antérieure sera ignorée.
    fn load_board(&mut self) -> Option<Effect> {
        let ctx = self.ctx.as_ref()?;
        let (board_id, project_key) = (ctx.board.id, ctx.project.key.clone());
        self.board_seq += 1;
        Some(Effect::LoadBoard {
            seq: self.board_seq,
            board_id,
            project_key,
        })
    }

    // ------------------------------------------------------------------ Écran principal

    fn on_main_key(&mut self, key: KeyEvent) -> Vec<Effect> {
        match key.code {
            KeyCode::Char('q') => self.request_quit(),
            KeyCode::Char('?') => self.popup = Some(Popup::Help),
            KeyCode::Esc => self.info(""),
            KeyCode::Char('1') => return self.set_tab(Tab::Backlog),
            KeyCode::Char('2') => return self.set_tab(Tab::Sprint),
            KeyCode::Char('3') => return self.set_tab(Tab::Versions),
            KeyCode::Tab => return self.set_tab(self.tab.cycle(true)),
            KeyCode::BackTab => return self.set_tab(self.tab.cycle(false)),
            KeyCode::Char('p') => return self.open_picker(),
            KeyCode::Char('R') => return self.refresh(),
            KeyCode::Char('j') | KeyCode::Down => return self.move_cursor(1),
            KeyCode::Char('k') | KeyCode::Up => return self.move_cursor(-1),
            KeyCode::Char('g') | KeyCode::Home => return self.move_cursor(isize::MIN / 2),
            KeyCode::Char('G') | KeyCode::End => return self.move_cursor(isize::MAX / 2),
            KeyCode::Char('h') | KeyCode::Left => self.move_horizontal(-1),
            KeyCode::Char('l') | KeyCode::Right => self.move_horizontal(1),
            KeyCode::Enter => self.on_enter(),
            KeyCode::Char(' ') => self.toggle_collapse(),
            _ if !self.loaded => self.info("Chargement en cours…"),
            KeyCode::Char('t') => {
                if let Some(issue) = self.selected_issue().cloned() {
                    return self.open_transitions(&issue);
                }
                self.info("Sélectionnez une issue");
            }
            KeyCode::Char('m') => match self.selected_issue().cloned() {
                Some(issue) => self.open_move(&issue),
                None => self.info("Sélectionnez une issue"),
            },
            KeyCode::Char('K') => self.open_rank(true),
            KeyCode::Char('J') => self.open_rank(false),
            KeyCode::Char('c') => match self.tab {
                Tab::Versions => self.open_create_version(),
                _ => self.open_create_issue(),
            },
            KeyCode::Char('e') => match self.tab {
                Tab::Versions if self.versions.focus == VersionsFocus::List => {
                    self.open_edit_version()
                }
                _ => match self.selected_issue().cloned() {
                    Some(issue) => self.open_edit_issue(&issue),
                    None => self.info("Sélectionnez une issue"),
                },
            },
            KeyCode::Char('r') if self.tab == Tab::Versions => self.open_release_version(),
            KeyCode::Char('s') => self.open_start_sprint(),
            KeyCode::Char('x') => self.open_close_sprint(),
            _ => {}
        }
        Vec::new()
    }

    fn set_tab(&mut self, tab: Tab) -> Vec<Effect> {
        self.tab = tab;
        self.ensure_version_issues()
    }

    fn refresh(&mut self) -> Vec<Effect> {
        self.versions.issues_for = None;
        self.versions.loading_for = None;
        self.info("Rafraîchissement…");
        self.load_board().into_iter().collect()
    }

    fn move_cursor(&mut self, delta: isize) -> Vec<Effect> {
        match self.tab {
            Tab::Backlog => {
                let n = self.backlog_rows().len();
                step(&mut self.backlog.cursor, n, delta);
            }
            Tab::Sprint => {
                let col = self.sprint.column;
                let n = self.sprint_columns()[col].len();
                step(&mut self.sprint.rows[col], n, delta);
            }
            Tab::Versions => match self.versions.focus {
                VersionsFocus::List => {
                    let n = self.data.versions_ordered().len();
                    step(&mut self.versions.cursor, n, delta);
                    self.versions.issue_cursor = 0;
                    return self.ensure_version_issues();
                }
                VersionsFocus::Issues => {
                    let n = self.versions.issues.len();
                    step(&mut self.versions.issue_cursor, n, delta);
                }
            },
        }
        Vec::new()
    }

    fn move_horizontal(&mut self, delta: isize) {
        match self.tab {
            Tab::Sprint => step(&mut self.sprint.column, COLUMNS.len(), delta),
            Tab::Versions => {
                self.versions.focus = if delta > 0 && !self.versions.issues.is_empty() {
                    VersionsFocus::Issues
                } else {
                    VersionsFocus::List
                };
            }
            Tab::Backlog => {}
        }
    }

    fn on_enter(&mut self) {
        if self.tab == Tab::Backlog
            && matches!(self.backlog_row(), Some(BacklogRow::SprintHeader(_)))
        {
            self.toggle_collapse();
            return;
        }
        if self.tab == Tab::Versions && self.versions.focus == VersionsFocus::List {
            if !self.versions.issues.is_empty() {
                self.versions.focus = VersionsFocus::Issues;
            }
            return;
        }
        if let Some(issue) = self.selected_issue().cloned() {
            self.popup = Some(Popup::Detail(Box::new(issue)));
        }
    }

    fn toggle_collapse(&mut self) {
        if self.tab != Tab::Backlog {
            return;
        }
        let Some(id) = self.backlog_current_sprint() else {
            return;
        };
        if !self.backlog.collapsed.remove(&id) {
            self.backlog.collapsed.insert(id);
        }
        // Garder le curseur sur l'en-tête du sprint replié/déplié.
        if let Some(pos) = self
            .backlog_rows()
            .iter()
            .position(|r| *r == BacklogRow::SprintHeader(id))
        {
            self.backlog.cursor = pos;
        }
    }

    // ------------------------------------------------------------------ Ouverture des popups d'écriture

    fn open_transitions(&mut self, issue: &Issue) -> Vec<Effect> {
        self.popup = Some(Popup::Transitions {
            key: issue.key.clone(),
            items: None,
            cursor: 0,
        });
        vec![Effect::LoadTransitions {
            key: issue.key.clone(),
        }]
    }

    fn open_move(&mut self, issue: &Issue) {
        let mut dests = Vec::new();
        if let Some(a) = &self.data.active
            && issue.sprint_id != Some(a.id)
        {
            dests.push(MoveDest::Sprint {
                id: a.id,
                name: format!("{} (actif)", a.name),
            });
        }
        for s in &self.data.future {
            if issue.sprint_id != Some(s.id) {
                dests.push(MoveDest::Sprint {
                    id: s.id,
                    name: s.name.clone(),
                });
            }
        }
        if issue.sprint_id.is_some() {
            dests.push(MoveDest::Backlog);
        }
        if dests.is_empty() {
            self.info("Aucune destination disponible (pas de sprint actif ni futur)");
            return;
        }
        self.popup = Some(Popup::Move {
            keys: vec![issue.key.clone()],
            dests,
            cursor: 0,
        });
    }

    fn open_rank(&mut self, up: bool) {
        if self.tab == Tab::Versions {
            self.info("Le rang se modifie dans les onglets Backlog et Sprint");
            return;
        }
        let Some(issue) = self.selected_issue().map(|i| i.key.clone()) else {
            self.info("Sélectionnez une issue");
            return;
        };
        let Some((prev, next)) = self.selected_neighbors() else {
            return;
        };
        let position = match (up, prev, next) {
            (true, Some(p), _) => RankPosition::Before(p.key.clone()),
            (false, _, Some(n)) => RankPosition::After(n.key.clone()),
            (true, None, _) => return self.info(format!("{issue} est déjà en tête")),
            (false, _, None) => return self.info(format!("{issue} est déjà en dernier")),
        };
        self.popup = Some(Popup::confirm(
            WriteOp::Rank {
                key: issue,
                position,
            },
            None,
        ));
    }

    fn open_create_issue(&mut self) {
        let Some(ctx) = &self.ctx else { return };
        let form = Form::create_issue(
            &ctx.project.key,
            &self.data.issue_types,
            &self.data.versions,
        );
        self.popup = Some(Popup::Form(Box::new(form)));
    }

    fn open_edit_issue(&mut self, issue: &Issue) {
        let form = Form::edit_issue(issue, &self.data.versions);
        self.popup = Some(Popup::Form(Box::new(form)));
    }

    fn open_create_version(&mut self) {
        let Some(ctx) = &self.ctx else { return };
        self.popup = Some(Popup::Form(Box::new(Form::create_version(&ctx.project.id))));
    }

    fn open_edit_version(&mut self) {
        match self.selected_version() {
            Some(v) => self.popup = Some(Popup::Form(Box::new(Form::edit_version(v)))),
            None => self.info("Aucune version sélectionnée"),
        }
    }

    fn open_release_version(&mut self) {
        let Some(v) = self.selected_version().cloned() else {
            return self.info("Aucune version sélectionnée");
        };
        if v.released {
            return self.info(format!("La version {} est déjà livrée", v.name));
        }
        // Les issues non terminées se lisent dans la liste de la version : elle doit être à jour.
        if self.versions.issues_for.as_deref() != Some(v.id.as_str()) {
            return self.info(format!("Issues de {} en cours de chargement…", v.name));
        }
        if self.versions.issues_failed {
            return self.error(format!(
                "Issues de {} non chargées : R pour réessayer avant de livrer",
                v.name
            ));
        }
        let unfinished: Vec<String> = self
            .versions
            .issues
            .iter()
            .filter(|i| i.status.category != StatusCategory::Done)
            .map(|i| i.key.clone())
            .collect();
        if unfinished.is_empty() {
            let op = release_op(&v, self.today, Vec::new(), None);
            self.popup = Some(Popup::confirm(op, None));
            return;
        }
        let mut dests = vec![VersionDest {
            id: None,
            label: format!("Aucune (restent dans {})", v.name),
        }];
        dests.extend(
            self.data
                .versions_ordered()
                .into_iter()
                .filter(|o| !o.released && o.id != v.id)
                .map(|o| VersionDest {
                    id: Some(o.id.clone()),
                    label: o.name.clone(),
                }),
        );
        self.popup = Some(Popup::ReleaseVersion {
            version: Box::new(v),
            unfinished,
            dests,
            cursor: 0,
        });
    }

    fn open_start_sprint(&mut self) {
        if let Some(a) = &self.data.active {
            let msg = format!("{} est déjà actif : clôturez-le d'abord (x)", a.name);
            return self.error(msg);
        }
        if self.data.future.is_empty() {
            return self.info("Aucun sprint futur à démarrer");
        }
        let chosen = if self.tab == Tab::Backlog {
            self.backlog_current_sprint()
                .and_then(|id| self.data.future.iter().find(|s| s.id == id))
        } else {
            None
        };
        match (chosen, self.data.future.as_slice()) {
            (Some(s), _) | (None, [s]) => {
                let form = Form::start_sprint(s, self.today);
                self.popup = Some(Popup::Form(Box::new(form)));
            }
            (None, sprints) => {
                self.popup = Some(Popup::PickSprint {
                    sprints: sprints.to_vec(),
                    cursor: 0,
                });
            }
        }
    }

    fn open_close_sprint(&mut self) {
        let Some(sprint) = self.data.active.clone() else {
            return self.info("Aucun sprint actif");
        };
        if !self.data.parallel_active.is_empty() {
            let names: Vec<&str> = std::iter::once(sprint.name.as_str())
                .chain(self.data.parallel_active.iter().map(|s| s.name.as_str()))
                .collect();
            return self.error(format!(
                "Sprints actifs en parallèle ({}) : clôture désactivée, à faire dans Jira",
                names.join(", ")
            ));
        }
        let issues = self.data.issues_in(Some(sprint.id));
        let unfinished = unfinished_keys(issues);
        let done = issues
            .iter()
            .filter(|i| i.status.category == StatusCategory::Done && !i.issue_type.subtask)
            .count();
        if unfinished.is_empty() {
            let op = close_op(&sprint, unfinished, &MoveDest::Backlog);
            self.popup = Some(Popup::confirm(op, None));
            return;
        }
        let mut dests = vec![MoveDest::Backlog];
        dests.extend(self.data.future.iter().map(|s| MoveDest::Sprint {
            id: s.id,
            name: s.name.clone(),
        }));
        self.popup = Some(Popup::CloseSprint {
            sprint: Box::new(sprint),
            unfinished,
            done,
            dests,
            cursor: 0,
        });
    }

    // ------------------------------------------------------------------ Popups

    fn on_popup_key(&mut self, popup: Popup, key: KeyEvent, at: Instant) -> Vec<Effect> {
        match popup {
            Popup::Help => {
                if !matches!(
                    key.code,
                    KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q') | KeyCode::Enter
                ) {
                    self.popup = Some(Popup::Help);
                }
            }
            Popup::Detail(issue) => match key.code {
                KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => {}
                KeyCode::Char('t') => return self.open_transitions(&issue),
                KeyCode::Char('m') => self.open_move(&issue),
                KeyCode::Char('e') => self.open_edit_issue(&issue),
                _ => self.popup = Some(Popup::Detail(issue)),
            },
            Popup::Transitions {
                key: issue_key,
                items,
                mut cursor,
            } => {
                let len = items.as_ref().map_or(0, Vec::len);
                match list_nav(key, &mut cursor, len) {
                    Some(false) => {}
                    Some(true) => {
                        if let Some(t) = items.as_ref().and_then(|v| v.get(cursor)) {
                            let op = WriteOp::Transition {
                                key: issue_key.clone(),
                                transition_id: t.id.clone(),
                                transition_name: t.name.clone(),
                                to_status: t.to.name.clone(),
                            };
                            let back = Popup::Transitions {
                                key: issue_key,
                                items,
                                cursor,
                            };
                            self.popup = Some(Popup::confirm(op, Some(back)));
                        } else {
                            self.popup = Some(Popup::Transitions {
                                key: issue_key,
                                items,
                                cursor,
                            });
                        }
                    }
                    None => {
                        self.popup = Some(Popup::Transitions {
                            key: issue_key,
                            items,
                            cursor,
                        })
                    }
                }
            }
            Popup::Move {
                keys,
                dests,
                mut cursor,
            } => match list_nav(key, &mut cursor, dests.len()) {
                Some(false) => {}
                Some(true) => {
                    let op = match &dests[cursor] {
                        MoveDest::Sprint { id, name } => WriteOp::MoveToSprint {
                            keys: keys.clone(),
                            sprint_id: *id,
                            sprint_name: name.clone(),
                        },
                        MoveDest::Backlog => WriteOp::MoveToBacklog { keys: keys.clone() },
                    };
                    let back = Popup::Move {
                        keys,
                        dests,
                        cursor,
                    };
                    self.popup = Some(Popup::confirm(op, Some(back)));
                }
                None => {
                    self.popup = Some(Popup::Move {
                        keys,
                        dests,
                        cursor,
                    })
                }
            },
            Popup::PickSprint {
                sprints,
                mut cursor,
            } => match list_nav(key, &mut cursor, sprints.len()) {
                Some(false) => {}
                Some(true) => {
                    let form = Form::start_sprint(&sprints[cursor], self.today);
                    self.popup = Some(Popup::Form(Box::new(form)));
                }
                None => self.popup = Some(Popup::PickSprint { sprints, cursor }),
            },
            Popup::CloseSprint {
                sprint,
                unfinished,
                done,
                dests,
                mut cursor,
            } => match list_nav(key, &mut cursor, dests.len()) {
                Some(false) => {}
                Some(true) => {
                    let op = close_op(&sprint, unfinished.clone(), &dests[cursor]);
                    let back = Popup::CloseSprint {
                        sprint,
                        unfinished,
                        done,
                        dests,
                        cursor,
                    };
                    self.popup = Some(Popup::confirm(op, Some(back)));
                }
                None => {
                    self.popup = Some(Popup::CloseSprint {
                        sprint,
                        unfinished,
                        done,
                        dests,
                        cursor,
                    });
                }
            },
            Popup::ReleaseVersion {
                version,
                unfinished,
                dests,
                mut cursor,
            } => match list_nav(key, &mut cursor, dests.len()) {
                Some(false) => {}
                Some(true) => {
                    let dest = &dests[cursor];
                    let move_to = dest.id.clone().map(|id| (id, dest.label.clone()));
                    let op = release_op(&version, self.today, unfinished.clone(), move_to);
                    let back = Popup::ReleaseVersion {
                        version,
                        unfinished,
                        dests,
                        cursor,
                    };
                    self.popup = Some(Popup::confirm(op, Some(back)));
                }
                None => {
                    self.popup = Some(Popup::ReleaseVersion {
                        version,
                        unfinished,
                        dests,
                        cursor,
                    });
                }
            },
            Popup::Form(mut form) => match form.handle_key(key) {
                FormOutcome::Cancel => {}
                FormOutcome::Continue => self.popup = Some(Popup::Form(form)),
                FormOutcome::Submit => match form.build_op(&self.tz) {
                    Ok(op) => self.popup = Some(Popup::confirm(op, Some(Popup::Form(form)))),
                    Err(e) => {
                        form.error = Some(e);
                        self.popup = Some(Popup::Form(form));
                    }
                },
            },
            Popup::Confirm {
                op,
                back,
                mut shown_at,
            } => match key.code {
                KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('q') => {
                    self.popup = back.map(|b| *b);
                }
                KeyCode::Enter | KeyCode::Char('y') | KeyCode::Char('o')
                    if !confirm_armed(shown_at, at) =>
                {
                    // Popup pas encore dessinée, ou frappe arrivée dans la foulée (collage,
                    // Entrée répétée) : la confirmation n'a pas pu être lue. Ignorée.
                    tracing::info!(op = %op.log_label(), "validation trop rapide ignorée");
                    rearm(&mut shown_at, at);
                    self.info("Validation ignorée (trop rapide) : relisez puis confirmez");
                    self.popup = Some(Popup::Confirm { op, back, shown_at });
                }
                KeyCode::Enter | KeyCode::Char('y') | KeyCode::Char('o') if self.read_only => {
                    tracing::info!(op = %op.log_label(), "écriture refusée (lecture seule)");
                    self.error(format!(
                        "Mode lecture seule : « {} » non envoyé à Jira",
                        op.describe().0
                    ));
                }
                KeyCode::Enter | KeyCode::Char('y') | KeyCode::Char('o') => {
                    tracing::info!(op = %op.log_label(), "écriture confirmée");
                    return self.submit_write(op);
                }
                _ => {
                    rearm(&mut shown_at, at);
                    self.popup = Some(Popup::Confirm { op, back, shown_at });
                }
            },
            Popup::QuitConfirm { back } => match key.code {
                KeyCode::Enter | KeyCode::Char('y') | KeyCode::Char('o') => self.request_quit(),
                KeyCode::Esc | KeyCode::Char('n') => {
                    // Une confirmation rouverte doit être relue avant d'être validée.
                    self.popup = Some(match *back {
                        Popup::Confirm { op, back, .. } => Popup::Confirm {
                            op,
                            back,
                            shown_at: None,
                        },
                        other => other,
                    });
                }
                _ => self.popup = Some(Popup::QuitConfirm { back }),
            },
        }
        Vec::new()
    }

    /// Les écritures sont sérialisées : une seule en vol, les suivantes en file (ordre conservé).
    fn submit_write(&mut self, op: WriteOp) -> Vec<Effect> {
        if self.write_in_flight {
            self.info(format!(
                "{} : en file d'attente (une écriture est en cours)",
                op.describe().0
            ));
            self.write_queue.push_back(op);
            return Vec::new();
        }
        self.write_in_flight = true;
        self.info(format!("{}…", op.describe().0));
        vec![Effect::Write(op)]
    }

    // ------------------------------------------------------------------ Versions

    /// Demande le chargement des issues de la version sélectionnée si nécessaire.
    fn ensure_version_issues(&mut self) -> Vec<Effect> {
        if self.tab != Tab::Versions || !self.loaded {
            return Vec::new();
        }
        let (Some(ctx), Some(v)) = (&self.ctx, self.selected_version()) else {
            self.versions.issues.clear();
            self.versions.issues_for = None;
            return Vec::new();
        };
        let id = v.id.clone();
        if self.versions.issues_for.as_ref() == Some(&id)
            || self.versions.loading_for.as_ref() == Some(&id)
        {
            return Vec::new();
        }
        let project_key = ctx.project.key.clone();
        self.versions.loading_for = Some(id.clone());
        self.version_seq += 1;
        vec![Effect::LoadVersionIssues {
            seq: self.version_seq,
            project_key,
            version_id: id,
        }]
    }

    // ------------------------------------------------------------------ Résultats asynchrones

    fn on_msg(&mut self, msg: Msg) -> Vec<Effect> {
        match msg {
            Msg::Projects(result) => {
                self.picker.loading = false;
                match result {
                    Ok(projects) => {
                        self.picker.projects = projects;
                        self.picker.cursor = 0;
                        if self.picker.projects.is_empty() {
                            self.error("Aucun projet accessible");
                        } else if self.picker.auto && self.picker.projects.len() == 1 {
                            return self.choose_project(0);
                        }
                    }
                    Err(e) => self.error(format!("Projets : {e}")),
                }
            }
            Msg::Boards {
                project_key,
                result,
            } => {
                if self.picker.project.as_ref().map(|p| &p.key) != Some(&project_key) {
                    return Vec::new();
                }
                self.picker.loading = false;
                match result {
                    Ok(boards) if boards.is_empty() => {
                        self.error(format!("Aucun board Scrum pour {project_key}"));
                        self.picker.project = None;
                    }
                    Ok(boards) => {
                        self.picker.boards = boards;
                        if self.picker.boards.len() == 1 {
                            return self.choose_board(0);
                        }
                    }
                    Err(e) => {
                        self.error(format!("Boards de {project_key} : {e}"));
                        self.picker.project = None;
                    }
                }
            }
            Msg::Board { seq, result } => {
                if seq != self.board_seq {
                    tracing::debug!(seq, latest = self.board_seq, "réponse de board périmée");
                    return Vec::new();
                }
                match result {
                    Ok(data) => {
                        self.data = *data;
                        self.loaded = true;
                        if self.status.text == "Rafraîchissement…" {
                            self.info("");
                        }
                        self.reconcile_cursors();
                        return self.ensure_version_issues();
                    }
                    Err(e) => self.error(format!("Chargement du board : {e}")),
                }
            }
            Msg::Transitions { key, result } => {
                let Some(Popup::Transitions { key: k, items, .. }) = &mut self.popup else {
                    return Vec::new();
                };
                if *k != key {
                    return Vec::new();
                }
                match result {
                    Ok(list) if list.is_empty() => {
                        self.popup = None;
                        self.info(format!("Aucune transition disponible pour {key}"));
                    }
                    Ok(list) => *items = Some(list),
                    Err(e) => {
                        self.popup = None;
                        self.error(format!("Transitions de {key} : {e}"));
                    }
                }
            }
            Msg::VersionIssues {
                seq,
                version_id,
                result,
            } => {
                if seq != self.version_seq
                    || self.versions.loading_for.as_ref() != Some(&version_id)
                {
                    tracing::debug!(seq, latest = self.version_seq, "issues de version périmées");
                    return Vec::new();
                }
                self.versions.loading_for = None;
                self.versions.issues_for = Some(version_id);
                self.versions.issues_failed = result.is_err();
                match result {
                    Ok(issues) => self.versions.issues = issues,
                    Err(e) => {
                        self.versions.issues.clear();
                        self.error(format!("Issues de la version : {e}"));
                    }
                }
                if self.versions.issues.is_empty() {
                    self.versions.focus = VersionsFocus::List;
                }
                self.reconcile_cursors();
            }
            Msg::Written { op, result } => {
                self.write_in_flight = false;
                match &result {
                    Ok(created) => {
                        tracing::info!(op = %op.log_label(), "écriture réussie");
                        self.info(op.success_message(created.as_deref()));
                        self.focus_key = match &op {
                            WriteOp::CreateIssue(_) => created.clone(),
                            _ => op.focus_key().map(str::to_string),
                        };
                        if let WriteOp::CreateVersion(_) = op {
                            self.focus_version = created.clone();
                        }
                    }
                    Err(e) => self.error(format!("Échec — {} : {e}", op.describe().0)),
                }
                // Rafraîchir dans tous les cas (une écriture composée peut être partielle).
                self.versions.issues_for = None;
                self.versions.loading_for = None;
                let mut effects: Vec<Effect> = self.load_board().into_iter().collect();
                if let Some(next) = self.write_queue.pop_front() {
                    effects.extend(self.submit_write(next));
                } else if self.quit_after_writes {
                    if result.is_ok() {
                        self.should_quit = true;
                        return Vec::new();
                    }
                    // Échec : rester pour que l'utilisateur voie l'erreur.
                    self.quit_after_writes = false;
                }
                return effects;
            }
            Msg::TaskFailed(e) => {
                tracing::error!(error = %e, "tâche asynchrone interrompue");
                self.fatal = Some(e);
                self.should_quit = true;
            }
        }
        Vec::new()
    }
}

/// Formulaire contenant une saisie non enregistrée (directement ou derrière une confirmation).
fn has_unsaved_form(p: &Popup) -> bool {
    match p {
        Popup::Form(f) => f.is_modified(),
        Popup::Confirm { back: Some(b), .. } => has_unsaved_form(b),
        _ => false,
    }
}

/// La confirmation a été dessinée et laissée affichée au moins [`CONFIRM_DELAY`] sans frappe.
fn confirm_armed(shown_at: Option<Instant>, at: Instant) -> bool {
    shown_at.is_some_and(|t| at.saturating_duration_since(t) >= CONFIRM_DELAY)
}

/// Toute frappe prématurée repousse l'échéance (une rafale ne finit jamais par valider).
fn rearm(shown_at: &mut Option<Instant>, at: Instant) {
    if let Some(t) = shown_at {
        *t = (*t).max(at);
    }
}

fn close_op(sprint: &Sprint, unfinished: Vec<String>, dest: &MoveDest) -> WriteOp {
    WriteOp::CloseSprint {
        sprint_id: sprint.id,
        sprint_name: sprint.name.clone(),
        unfinished,
        destination: match dest {
            MoveDest::Sprint { id, name } => Some((*id, name.clone())),
            MoveDest::Backlog => None,
        },
    }
}

fn release_op(
    v: &Version,
    today: jiff::civil::Date,
    unfinished: Vec<String>,
    move_to: Option<(String, String)>,
) -> WriteOp {
    WriteOp::ReleaseVersion {
        version_id: v.id.clone(),
        version_name: v.name.clone(),
        // Date prévue conservée ; à défaut, livrée aujourd'hui.
        release_date: v.release_date.is_none().then(|| today.to_string()),
        unfinished,
        move_to,
    }
}
