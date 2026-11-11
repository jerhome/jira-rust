//! Scénarios de bout en bout sur `MockJira` (aucun accès réseau) et rendus `TestBackend`.
//!
//! Le harnais exécute la logique exactement comme la boucle réelle (`update` → effets →
//! `runtime::execute` → `Msg`), mais de façon déterministe et sans terminal.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use jiff::civil::date;
use jiff::tz::TimeZone;
use jira_client::mock::MockJira;
use jira_client::model::*;
use jira_client::{Error, JiraApi};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use tokio::sync::mpsc::unbounded_channel;

use super::action::{AppEvent, Effect, Msg, WriteOp};
use super::state::*;
use super::{dispatch, runtime, ui, update};

// ------------------------------------------------------------------ API de test

/// `MockJira` + journal des écritures + injection d'erreurs (statut HTTP) et de paniques.
#[derive(Default)]
pub(super) struct TestApi {
    inner: MockJira,
    writes: Mutex<Vec<String>>,
    fail: Mutex<HashMap<&'static str, (u16, String)>>,
    panic_on: Mutex<Option<&'static str>>,
    /// Sprints ajoutés à la réponse de `sprints` (autres boards, sprints parallèles).
    extra_sprints: Mutex<Vec<Sprint>>,
    /// Issues ajoutées à la réponse de `sprint_issues` (sous-tâches…).
    extra_sprint_issues: Mutex<HashMap<u64, Vec<Issue>>>,
}

impl TestApi {
    fn writes(&self) -> Vec<String> {
        self.writes.lock().unwrap().clone()
    }

    fn fail_on(&self, method: &'static str, status: u16) {
        self.fail_on_msg(method, status, "erreur simulée");
    }

    fn fail_on_msg(&self, method: &'static str, status: u16, message: &str) {
        self.fail
            .lock()
            .unwrap()
            .insert(method, (status, message.to_string()));
    }

    fn check(&self, method: &'static str) -> jira_client::Result<()> {
        if *self.panic_on.lock().unwrap() == Some(method) {
            panic!("panique simulée dans {method}");
        }
        match self.fail.lock().unwrap().get(method).cloned() {
            None => Ok(()),
            Some((403, _)) => Err(Error::Forbidden("vous n'avez pas la permission".into())),
            Some((404, message)) => Err(Error::NotFound(message)),
            Some((status, message)) => Err(Error::Api { status, message }),
        }
    }

    fn write(&self, method: &'static str, detail: String) -> jira_client::Result<()> {
        self.check(method)?;
        self.writes
            .lock()
            .unwrap()
            .push(format!("{method} {detail}"));
        Ok(())
    }
}

#[async_trait]
impl JiraApi for TestApi {
    async fn myself(&self) -> jira_client::Result<User> {
        self.check("myself")?;
        self.inner.myself().await
    }
    async fn projects(&self) -> jira_client::Result<Vec<Project>> {
        self.check("projects")?;
        self.inner.projects().await
    }
    async fn scrum_boards(&self, project_key: &str) -> jira_client::Result<Vec<Board>> {
        self.check("scrum_boards")?;
        self.inner.scrum_boards(project_key).await
    }
    async fn sprints(
        &self,
        board_id: u64,
        states: &[SprintState],
    ) -> jira_client::Result<Vec<Sprint>> {
        self.check("sprints")?;
        let mut all = self.inner.sprints(board_id, states).await?;
        all.extend(
            self.extra_sprints
                .lock()
                .unwrap()
                .iter()
                .filter(|s| states.is_empty() || states.contains(&s.state))
                .cloned(),
        );
        Ok(all)
    }
    async fn backlog_issues(&self, board_id: u64) -> jira_client::Result<Vec<Issue>> {
        self.check("backlog_issues")?;
        self.inner.backlog_issues(board_id).await
    }
    async fn sprint_issues(&self, sprint_id: u64) -> jira_client::Result<Vec<Issue>> {
        self.check("sprint_issues")?;
        let mut all = self.inner.sprint_issues(sprint_id).await?;
        if let Some(extra) = self.extra_sprint_issues.lock().unwrap().get(&sprint_id) {
            all.extend(extra.iter().cloned());
        }
        Ok(all)
    }
    async fn move_to_sprint(&self, sprint_id: u64, keys: &[String]) -> jira_client::Result<()> {
        self.write("move_to_sprint", format!("{sprint_id} {keys:?}"))?;
        self.inner.move_to_sprint(sprint_id, keys).await
    }
    async fn move_to_backlog(&self, keys: &[String]) -> jira_client::Result<()> {
        self.write("move_to_backlog", format!("{keys:?}"))?;
        self.inner.move_to_backlog(keys).await
    }
    async fn rank(&self, keys: &[String], position: &RankPosition) -> jira_client::Result<()> {
        self.write("rank", format!("{keys:?} {position:?}"))?;
        self.inner.rank(keys, position).await
    }
    async fn start_sprint(
        &self,
        sprint_id: u64,
        params: &StartSprint,
    ) -> jira_client::Result<Sprint> {
        self.write("start_sprint", sprint_id.to_string())?;
        self.inner.start_sprint(sprint_id, params).await
    }
    async fn close_sprint(&self, sprint_id: u64) -> jira_client::Result<Sprint> {
        self.write("close_sprint", sprint_id.to_string())?;
        self.inner.close_sprint(sprint_id).await
    }
    async fn create_sprint(
        &self,
        board_id: u64,
        name: &str,
        goal: Option<String>,
    ) -> jira_client::Result<Sprint> {
        self.write("create_sprint", format!("{board_id} {name}"))?;
        self.inner.create_sprint(board_id, name, goal).await
    }
    async fn update_sprint(
        &self,
        sprint_id: u64,
        update: &SprintUpdate,
    ) -> jira_client::Result<Sprint> {
        self.write("update_sprint", sprint_id.to_string())?;
        self.inner.update_sprint(sprint_id, update).await
    }
    async fn issue_types(&self, project_key: &str) -> jira_client::Result<Vec<IssueType>> {
        self.check("issue_types")?;
        self.inner.issue_types(project_key).await
    }
    async fn create_issue(&self, issue: &NewIssue) -> jira_client::Result<String> {
        self.write("create_issue", issue.summary.clone())?;
        self.inner.create_issue(issue).await
    }
    async fn update_issue(&self, key: &str, update: &IssueUpdate) -> jira_client::Result<()> {
        self.write("update_issue", key.to_string())?;
        self.inner.update_issue(key, update).await
    }
    async fn transitions(&self, key: &str) -> jira_client::Result<Vec<Transition>> {
        self.check("transitions")?;
        self.inner.transitions(key).await
    }
    async fn transition_issue(&self, key: &str, transition_id: &str) -> jira_client::Result<()> {
        self.write("transition_issue", format!("{key} {transition_id}"))?;
        self.inner.transition_issue(key, transition_id).await
    }
    async fn versions(&self, project_key: &str) -> jira_client::Result<Vec<Version>> {
        self.check("versions")?;
        self.inner.versions(project_key).await
    }
    async fn version_issues(
        &self,
        project_key: &str,
        version_id: &str,
    ) -> jira_client::Result<Vec<Issue>> {
        self.check("version_issues")?;
        self.inner.version_issues(project_key, version_id).await
    }
    async fn create_version(&self, version: &NewVersion) -> jira_client::Result<Version> {
        self.write("create_version", version.name.clone())?;
        self.inner.create_version(version).await
    }
    async fn update_version(
        &self,
        version_id: &str,
        update: &VersionUpdate,
    ) -> jira_client::Result<Version> {
        self.write("update_version", version_id.to_string())?;
        self.inner.update_version(version_id, update).await
    }
    async fn delete_version(
        &self,
        version_id: &str,
        move_fix_issues_to: Option<String>,
    ) -> jira_client::Result<()> {
        self.write("delete_version", version_id.to_string())?;
        self.inner
            .delete_version(version_id, move_fix_issues_to)
            .await
    }
}

/// Issue fictive tirée de `MockJira` (évite de dépendre de la liste exacte des champs du modèle).
pub(super) fn mock_issue(key: &str) -> Issue {
    let rt = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    rt.block_on(async {
        let api = MockJira::new();
        let mut all = api.backlog_issues(1).await.unwrap();
        for s in [10, 11] {
            all.extend(api.sprint_issues(s).await.unwrap());
        }
        all.into_iter()
            .find(|i| i.key == key)
            .expect("issue fictive")
    })
}

// ------------------------------------------------------------------ Harnais

pub(super) fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

/// Harnais générique : `TestApi` (MockJira instrumenté) ici, `JiraHttp` → jira-fake dans `e2e_tests`.
pub(super) struct H<A: JiraApi + 'static = TestApi> {
    pub(super) app: App,
    pub(super) api: Arc<A>,
    /// Effets émis mais volontairement non exécutés (réponses « tardives »).
    deferred: usize,
    /// Horloge virtuelle des frappes : avance d'une seconde entre deux touches « humaines ».
    pub(super) clock: Instant,
}

impl H<TestApi> {
    fn new() -> Self {
        Self::with_api(Arc::new(TestApi::default()))
    }

    async fn started() -> Self {
        Self::started_with(Arc::new(TestApi::default())).await
    }
}

impl<A: JiraApi + 'static> H<A> {
    pub(super) fn with_api(api: Arc<A>) -> Self {
        Self {
            app: App::new(date(2026, 11, 11), TimeZone::UTC),
            api,
            deferred: 0,
            clock: Instant::now(),
        }
    }

    /// Démarrage complet : projets → DEMO → board unique choisi automatiquement → données.
    pub(super) async fn started_with(api: Arc<A>) -> Self {
        let mut h = Self::with_api(api);
        let effects = update::init(&mut h.app);
        h.settle(effects).await;
        assert_eq!(h.app.screen, Screen::Picker, "deux projets : choix manuel");
        h.press(KeyCode::Enter).await;
        assert_eq!(h.app.screen, Screen::Main);
        assert!(h.app.loaded);
        h
    }

    /// Exécute les effets (et ceux qu'ils engendrent) jusqu'à épuisement.
    pub(super) async fn settle(&mut self, effects: Vec<Effect>) {
        let mut queue: VecDeque<Effect> = effects.into();
        while let Some(effect) = queue.pop_front() {
            let msg = runtime::execute(self.api.as_ref(), effect).await;
            queue.extend(update::update(&mut self.app, AppEvent::Msg(msg)));
        }
        assert_eq!(
            self.app.pending, self.deferred,
            "compteur de chargement cohérent"
        );
    }

    /// Dessine réellement l'écran (`TestBackend`) puis le signale à l'app, comme la boucle.
    pub(super) fn draw(&mut self) {
        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
        terminal.draw(|f| ui::draw(f, &self.app)).unwrap();
        self.app.mark_drawn(self.clock);
    }

    /// Frappe « humaine » : écran dessiné, puis touche une seconde plus tard.
    fn human_key(&mut self, k: KeyEvent) -> Vec<Effect> {
        self.draw();
        self.clock += Duration::from_secs(1);
        update::update(&mut self.app, AppEvent::Key(k, self.clock))
    }

    pub(super) async fn send(&mut self, k: KeyEvent) {
        let effects = self.human_key(k);
        self.settle(effects).await;
    }

    /// Rafale de touches traitées dans le même lot, sans redessin entre elles (collage sans
    /// bracketed paste, frappe en avance), comme la boucle réelle.
    pub(super) async fn burst(&mut self, keys: &[KeyEvent]) {
        self.draw();
        self.clock += Duration::from_secs(1);
        let mut effects = Vec::new();
        for k in keys {
            self.clock += Duration::from_millis(1);
            effects.extend(update::update(&mut self.app, AppEvent::Key(*k, self.clock)));
        }
        self.settle(effects).await;
    }

    /// Texte collé (bracketed paste).
    pub(super) async fn paste(&mut self, text: &str) {
        self.draw();
        self.clock += Duration::from_secs(1);
        let effects = update::update(&mut self.app, AppEvent::Paste(text.to_string(), self.clock));
        self.settle(effects).await;
    }

    pub(super) async fn press(&mut self, code: KeyCode) {
        self.send(key(code)).await;
    }

    pub(super) async fn chars(&mut self, s: &str) {
        for c in s.chars() {
            self.press(KeyCode::Char(c)).await;
        }
    }

    pub(super) async fn ctrl_s(&mut self) {
        self.send(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL))
            .await;
    }

    /// Effets d'une touche, sans les exécuter (pour simuler des réponses tardives).
    pub(super) fn press_deferred(&mut self, code: KeyCode) -> Vec<Effect> {
        let effects = self.human_key(key(code));
        self.deferred += effects.len();
        effects
    }

    /// Livre enfin des effets différés.
    pub(super) async fn settle_deferred(&mut self, effects: Vec<Effect>) {
        self.deferred -= effects.len();
        self.settle(effects).await;
    }

    pub(super) fn keys_in(&self, loc: Option<u64>) -> Vec<&str> {
        self.app
            .data
            .issues_in(loc)
            .iter()
            .map(|i| i.key.as_str())
            .collect()
    }

    pub(super) fn selected(&self) -> Option<&str> {
        self.app.selected_issue().map(|i| i.key.as_str())
    }

    pub(super) fn confirm_op(&self) -> &WriteOp {
        match &self.app.popup {
            Some(Popup::Confirm { op, .. }) => op,
            other => panic!("popup de confirmation attendue, obtenu {other:?}"),
        }
    }

    /// Sélectionne une issue du backlog par sa clé.
    pub(super) fn select_backlog(&mut self, key: &str) {
        let rows = self.app.backlog_rows();
        let pos = rows
            .iter()
            .position(|r| match r {
                BacklogRow::Issue { loc, index } => {
                    self.app.data.issues_in(*loc)[*index].key == key
                }
                _ => false,
            })
            .expect("issue présente dans le backlog");
        self.app.backlog.cursor = pos;
    }

    pub(super) fn render(&self, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|f| ui::draw(f, &self.app)).unwrap();
        let buf = terminal.backend().buffer();
        (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

// ------------------------------------------------------------------ Démarrage

#[tokio::test]
async fn startup_loads_board() {
    let h = H::started().await;
    let ctx = h.app.ctx.as_ref().unwrap();
    assert_eq!((ctx.project.key.as_str(), ctx.board.id), ("DEMO", 1));
    assert_eq!(h.app.data.active.as_ref().map(|s| s.id), Some(10));
    assert_eq!(
        h.app.data.future.iter().map(|s| s.id).collect::<Vec<_>>(),
        vec![11]
    );
    assert_eq!(
        h.keys_in(None),
        ["DEMO-8", "DEMO-9", "DEMO-10", "DEMO-11", "DEMO-12"]
    );
    assert_eq!(h.keys_in(Some(11)), ["DEMO-6", "DEMO-7"]);
    assert_eq!(h.app.tab, Tab::Backlog);
    assert!(h.api.writes().is_empty(), "aucune écriture au démarrage");
}

#[tokio::test]
async fn picker_switch_project() {
    let mut h = H::started().await;
    h.press(KeyCode::Char('p')).await;
    assert_eq!(h.app.screen, Screen::Picker);
    h.press(KeyCode::Char('j')).await;
    h.press(KeyCode::Enter).await; // OPS → board unique
    assert_eq!(h.app.ctx.as_ref().unwrap().project.key, "OPS");
    assert_eq!(h.keys_in(None), ["OPS-1"]);
    assert!(h.app.data.active.is_none());
}

// ------------------------------------------------------------------ Backlog

#[tokio::test]
async fn rank_up_in_backlog() {
    let mut h = H::started().await;
    h.select_backlog("DEMO-9");
    h.press(KeyCode::Char('K')).await;
    assert_eq!(
        *h.confirm_op(),
        WriteOp::Rank {
            key: "DEMO-9".into(),
            position: RankPosition::Before("DEMO-8".into())
        }
    );
    h.press(KeyCode::Enter).await;
    assert_eq!(h.keys_in(None)[..2], ["DEMO-9", "DEMO-8"]);
    assert_eq!(h.selected(), Some("DEMO-9"), "la sélection suit l'issue");
    assert_eq!(h.app.status.text, "Rang de DEMO-9 mis à jour");
    assert_eq!(h.api.writes().len(), 1);

    // Déjà en tête : pas de popup.
    h.press(KeyCode::Char('K')).await;
    assert!(h.app.popup.is_none());
    assert!(h.app.status.text.contains("déjà en tête"));
}

#[tokio::test]
async fn cancel_never_writes() {
    let mut h = H::started().await;
    h.select_backlog("DEMO-9");
    h.press(KeyCode::Char('J')).await;
    h.press(KeyCode::Esc).await;
    assert!(h.app.popup.is_none());

    // Formulaire → confirmation → annuler revient au formulaire, Esc le ferme.
    h.press(KeyCode::Char('c')).await;
    h.chars("Brouillon").await;
    h.ctrl_s().await;
    assert!(matches!(h.confirm_op(), WriteOp::CreateIssue(_)));
    h.press(KeyCode::Char('n')).await;
    assert!(matches!(h.app.popup, Some(Popup::Form(_))));
    h.press(KeyCode::Esc).await;
    assert!(h.app.popup.is_none());

    // Déplacement : liste → confirmation → annuler revient à la liste.
    h.press(KeyCode::Char('m')).await;
    h.press(KeyCode::Enter).await;
    h.press(KeyCode::Esc).await;
    assert!(matches!(h.app.popup, Some(Popup::Move { .. })));
    h.press(KeyCode::Esc).await;

    // Clôture de sprint annulée.
    h.press(KeyCode::Char('2')).await;
    h.press(KeyCode::Char('x')).await;
    h.press(KeyCode::Enter).await;
    h.press(KeyCode::Esc).await;
    h.press(KeyCode::Esc).await;
    assert!(h.app.popup.is_none());

    assert!(
        h.api.writes().is_empty(),
        "aucune écriture après annulations"
    );
    assert_eq!(
        h.keys_in(None),
        ["DEMO-8", "DEMO-9", "DEMO-10", "DEMO-11", "DEMO-12"]
    );
}

#[tokio::test]
async fn move_backlog_issue_to_active_sprint_and_back() {
    let mut h = H::started().await;
    h.select_backlog("DEMO-8");
    h.press(KeyCode::Char('m')).await;
    let Some(Popup::Move { dests, .. }) = &h.app.popup else {
        panic!("popup de déplacement")
    };
    assert_eq!(
        dests.iter().map(MoveDest::label).collect::<Vec<_>>(),
        ["DEMO Sprint 4 (actif)", "DEMO Sprint 5"],
        "pas de destination « Backlog » pour une issue déjà au backlog"
    );
    h.press(KeyCode::Enter).await;
    assert!(matches!(
        h.confirm_op(),
        WriteOp::MoveToSprint { sprint_id: 10, .. }
    ));
    h.press(KeyCode::Enter).await;
    assert!(h.keys_in(Some(10)).contains(&"DEMO-8"));
    assert!(!h.keys_in(None).contains(&"DEMO-8"));

    // Retour au backlog depuis l'onglet Sprint (DEMO-8 est « À faire »).
    h.press(KeyCode::Char('2')).await;
    assert_eq!(h.app.sprint.column, 0);
    h.press(KeyCode::Char('G')).await;
    assert_eq!(h.selected(), Some("DEMO-8"));
    h.press(KeyCode::Char('m')).await;
    h.press(KeyCode::Char('G')).await;
    h.press(KeyCode::Enter).await;
    assert_eq!(
        *h.confirm_op(),
        WriteOp::MoveToBacklog {
            keys: vec!["DEMO-8".into()]
        }
    );
    h.press(KeyCode::Char('o')).await;
    assert!(h.keys_in(None).contains(&"DEMO-8"));
    assert_eq!(
        h.api.writes(),
        [
            "move_to_sprint 10 [\"DEMO-8\"]",
            "move_to_backlog [\"DEMO-8\"]"
        ]
    );
}

#[tokio::test]
async fn create_issue_with_form() {
    let mut h = H::started().await;
    h.press(KeyCode::Char('c')).await;
    h.ctrl_s().await;
    let Some(Popup::Form(form)) = &h.app.popup else {
        panic!("formulaire")
    };
    assert_eq!(form.error.as_deref(), Some("le résumé est obligatoire"));

    h.chars("Nouvelle story").await;
    h.press(KeyCode::Tab).await; // type
    h.press(KeyCode::Tab).await; // points
    h.chars("3").await;
    h.press(KeyCode::Enter).await; // Entrée dans un champ simple = valider
    let WriteOp::CreateIssue(new) = h.confirm_op() else {
        panic!("création attendue")
    };
    assert_eq!(
        (new.summary.as_str(), new.story_points),
        ("Nouvelle story", Some(3.0))
    );
    h.press(KeyCode::Enter).await;
    assert_eq!(h.app.status.text, "Issue DEMO-100 créée");
    assert!(!h.app.status.error);
    assert_eq!(
        h.selected(),
        Some("DEMO-100"),
        "nouvelle issue sélectionnée"
    );
    let created = h.app.data.find_issue("DEMO-100").unwrap();
    assert_eq!(created.issue_type.name, "Story");
}

#[tokio::test]
async fn edit_issue_sends_only_changes() {
    let mut h = H::started().await;
    h.select_backlog("DEMO-8");
    h.press(KeyCode::Char('e')).await;
    h.chars(" !").await;
    h.press(KeyCode::Enter).await;
    let WriteOp::UpdateIssue { key, update, .. } = h.confirm_op() else {
        panic!("modification attendue")
    };
    assert_eq!(key, "DEMO-8");
    assert_eq!(
        *update,
        IssueUpdate {
            summary: Some("Traduction anglaise !".into()),
            ..Default::default()
        }
    );
    h.press(KeyCode::Enter).await;
    assert_eq!(
        h.app.data.find_issue("DEMO-8").unwrap().summary,
        "Traduction anglaise !"
    );
    assert_eq!(h.api.writes(), ["update_issue DEMO-8"]);
}

// ------------------------------------------------------------------ Sprint actif

#[tokio::test]
async fn transition_in_sprint_board() {
    let mut h = H::started().await;
    h.press(KeyCode::Char('2')).await;
    let cols = h
        .app
        .sprint_columns()
        .map(|c| c.iter().map(|i| i.key.clone()).collect::<Vec<_>>());
    assert_eq!(cols[0], ["DEMO-4", "DEMO-5"]);
    assert_eq!(cols[1], ["DEMO-2", "DEMO-3"]);
    assert_eq!(cols[2], ["DEMO-1"]);

    h.press(KeyCode::Char('t')).await;
    let Some(Popup::Transitions {
        items: Some(items), ..
    }) = &h.app.popup
    else {
        panic!("transitions")
    };
    let pos = items.iter().position(|t| t.to.name == "En cours").unwrap();
    for _ in 0..pos {
        h.press(KeyCode::Char('j')).await;
    }
    h.press(KeyCode::Enter).await;
    assert!(
        matches!(h.confirm_op(), WriteOp::Transition { to_status, .. } if to_status == "En cours")
    );
    h.press(KeyCode::Enter).await;
    assert_eq!(
        h.app.data.find_issue("DEMO-4").unwrap().status.category,
        StatusCategory::InProgress
    );
    assert_eq!(
        h.app.sprint.column, 1,
        "la sélection suit l'issue dans sa nouvelle colonne"
    );
    assert_eq!(h.selected(), Some("DEMO-4"));
    assert_eq!(h.app.status.text, "DEMO-4 → En cours");
}

#[tokio::test]
async fn close_sprint_to_future_then_start_next() {
    let mut h = H::started().await;
    h.press(KeyCode::Char('2')).await;
    h.press(KeyCode::Char('x')).await;
    let Some(Popup::CloseSprint {
        unfinished,
        done,
        dests,
        ..
    }) = &h.app.popup
    else {
        panic!("popup de clôture")
    };
    assert_eq!(*done, 1);
    assert_eq!(unfinished, &["DEMO-2", "DEMO-3", "DEMO-4", "DEMO-5"]);
    assert_eq!(
        dests.iter().map(MoveDest::label).collect::<Vec<_>>(),
        ["Backlog", "DEMO Sprint 5"]
    );
    h.press(KeyCode::Char('j')).await;
    h.press(KeyCode::Enter).await;
    assert!(matches!(
        h.confirm_op(),
        WriteOp::CloseSprint {
            destination: Some((11, _)),
            ..
        }
    ));
    h.press(KeyCode::Enter).await;

    // Déplacement explicite PUIS clôture (on ne dépend pas de close_sprint pour déplacer).
    assert_eq!(
        h.api.writes(),
        [
            "move_to_sprint 11 [\"DEMO-2\", \"DEMO-3\", \"DEMO-4\", \"DEMO-5\"]",
            "close_sprint 10"
        ]
    );
    assert!(h.app.data.active.is_none());
    assert_eq!(
        h.keys_in(Some(11)),
        ["DEMO-2", "DEMO-3", "DEMO-4", "DEMO-5", "DEMO-6", "DEMO-7"]
    );
    assert!(
        h.render(120, 30)
            .contains("Aucun sprint actif. s : démarrer un sprint futur.")
    );

    // Démarrage : un seul sprint futur → formulaire direct, dates par défaut.
    h.press(KeyCode::Char('s')).await;
    assert!(matches!(h.app.popup, Some(Popup::Form(_))));
    h.ctrl_s().await;
    let WriteOp::StartSprint {
        sprint_id, params, ..
    } = h.confirm_op()
    else {
        panic!("démarrage")
    };
    assert_eq!(*sprint_id, 11);
    assert_eq!(params.start_date, "2026-11-11T09:00:00.000+00:00");
    assert_eq!(params.end_date, "2026-11-25T17:00:00.000+00:00");
    h.press(KeyCode::Enter).await;
    assert_eq!(h.app.data.active.as_ref().map(|s| s.id), Some(11));
    assert!(h.app.data.future.is_empty());
    assert_eq!(h.app.status.text, "DEMO Sprint 5 démarré");

    // Un sprint actif existe : refus local, sans popup.
    h.press(KeyCode::Char('s')).await;
    assert!(h.app.popup.is_none());
    assert!(h.app.status.error);
}

#[tokio::test]
async fn close_sprint_to_backlog() {
    let mut h = H::started().await;
    h.press(KeyCode::Char('2')).await;
    h.press(KeyCode::Char('x')).await;
    h.press(KeyCode::Enter).await; // Backlog (premier choix)
    h.press(KeyCode::Enter).await;
    let writes = h.api.writes();
    assert_eq!(writes.len(), 2);
    assert!(writes[0].starts_with("move_to_backlog"));
    assert_eq!(writes[1], "close_sprint 10");
    for k in ["DEMO-2", "DEMO-3", "DEMO-4", "DEMO-5"] {
        assert!(h.keys_in(None).contains(&k), "{k} renvoyée au backlog");
    }
    assert!(
        h.app.data.find_issue("DEMO-1").is_none(),
        "l'issue terminée reste dans le sprint clos"
    );
}

// ------------------------------------------------------------------ Versions

#[tokio::test]
async fn versions_list_create_and_release() {
    let mut h = H::started().await;
    h.press(KeyCode::Char('3')).await;
    let names: Vec<_> = h
        .app
        .data
        .versions_ordered()
        .iter()
        .map(|v| v.name.clone())
        .collect();
    assert_eq!(names, ["1.1.0", "2.0.0", "1.0.0"], "non livrées d'abord");
    assert_eq!(h.app.versions.issues_for.as_deref(), Some("101"));
    assert_eq!(h.app.versions.issues.len(), 5);

    h.press(KeyCode::Char('j')).await;
    assert_eq!(h.app.versions.issues_for.as_deref(), Some("102"));
    let keys: Vec<_> = h
        .app
        .versions
        .issues
        .iter()
        .map(|i| i.key.as_str())
        .collect();
    assert_eq!(keys, ["DEMO-6", "DEMO-7", "DEMO-8", "DEMO-11"]);

    // Création.
    h.press(KeyCode::Char('c')).await;
    h.chars("3.0.0").await;
    h.press(KeyCode::Tab).await;
    h.press(KeyCode::Tab).await;
    h.chars("2027-06-30").await;
    h.ctrl_s().await;
    h.press(KeyCode::Enter).await;
    assert_eq!(h.app.status.text, "Version 3.0.0 créée");
    let v = h.app.selected_version().unwrap();
    assert_eq!(
        (v.name.as_str(), v.release_date.as_deref()),
        ("3.0.0", Some("2027-06-30"))
    );

    // Édition du nom.
    h.press(KeyCode::Char('e')).await;
    h.chars("-rc").await;
    h.ctrl_s().await;
    h.press(KeyCode::Enter).await;
    assert_eq!(h.app.selected_version().unwrap().name, "3.0.0-rc");

    // Livraison de 1.1.0 (date existante conservée) : issues non terminées → choix « aucune ».
    h.press(KeyCode::Char('g')).await;
    h.press(KeyCode::Char('r')).await;
    let Some(Popup::ReleaseVersion {
        unfinished, dests, ..
    }) = &h.app.popup
    else {
        panic!("choix de destination attendu")
    };
    assert_eq!(unfinished, &["DEMO-2", "DEMO-3", "DEMO-4", "DEMO-5"]);
    assert_eq!(
        dests.iter().map(|d| d.label.as_str()).collect::<Vec<_>>(),
        ["Aucune (restent dans 1.1.0)", "2.0.0", "3.0.0-rc"]
    );
    h.press(KeyCode::Enter).await;
    assert_eq!(
        *h.confirm_op(),
        WriteOp::ReleaseVersion {
            version_id: "101".into(),
            version_name: "1.1.0".into(),
            release_date: None,
            unfinished: vec![
                "DEMO-2".into(),
                "DEMO-3".into(),
                "DEMO-4".into(),
                "DEMO-5".into()
            ],
            move_to: None,
        }
    );
    h.press(KeyCode::Enter).await;
    assert_eq!(h.app.status.text, "Version 1.1.0 livrée");
    let released: Vec<_> = h
        .app
        .data
        .versions_ordered()
        .iter()
        .filter(|v| v.released)
        .map(|v| v.name.clone())
        .collect();
    assert_eq!(released, ["1.0.0", "1.1.0"]);

    // Version déjà livrée : pas de popup.
    h.press(KeyCode::Char('G')).await;
    h.press(KeyCode::Char('r')).await;
    assert!(h.app.popup.is_none());
    assert_eq!(
        h.api.writes(),
        [
            "create_version 3.0.0",
            "update_version 200",
            "update_version 101"
        ]
    );
}

// ------------------------------------------------------------------ Réponses périmées

#[tokio::test]
async fn stale_board_response_is_ignored() {
    let mut h = H::started().await;
    let refresh = h.press_deferred(KeyCode::Char('R'));
    assert!(matches!(refresh[..], [Effect::LoadBoard { .. }]));
    // L'utilisateur change de board avant la réponse.
    h.press(KeyCode::Char('p')).await;
    h.press(KeyCode::Char('j')).await;
    h.press(KeyCode::Enter).await;
    assert_eq!(h.app.ctx.as_ref().unwrap().project.key, "OPS");
    h.settle_deferred(refresh).await;
    assert_eq!(
        h.keys_in(None),
        ["OPS-1"],
        "les données de DEMO arrivées en retard sont ignorées"
    );
}

#[tokio::test]
async fn stale_version_issues_are_ignored() {
    let mut h = H::started().await;
    let first = h.press_deferred(KeyCode::Char('3'));
    let second = h.press_deferred(KeyCode::Char('j'));
    assert!(
        matches!(&first[..], [Effect::LoadVersionIssues { version_id, .. }] if version_id == "101")
    );
    assert!(
        matches!(&second[..], [Effect::LoadVersionIssues { version_id, .. }] if version_id == "102")
    );
    h.settle_deferred(second).await;
    let after_second = h.app.versions.issues.clone();
    h.settle_deferred(first).await;
    assert_eq!(h.app.versions.issues_for.as_deref(), Some("102"));
    assert_eq!(
        h.app.versions.issues, after_second,
        "réponse de 1.1.0 ignorée"
    );
}

// ------------------------------------------------------------------ Erreurs

#[tokio::test]
async fn forbidden_write_shows_clear_status() {
    let mut h = H::started().await;
    h.api.fail_on("close_sprint", 403);
    h.press(KeyCode::Char('2')).await;
    h.press(KeyCode::Char('x')).await;
    h.press(KeyCode::Enter).await;
    h.press(KeyCode::Enter).await;
    assert!(h.app.status.error);
    assert!(
        h.app.status.text.contains("Clôturer le sprint"),
        "{}",
        h.app.status.text
    );
    assert!(h.app.status.text.contains("403"));
    assert!(h.app.status.text.contains("droits Jira insuffisants"));
    // Le déplacement a eu lieu avant l'échec : l'état est rafraîchi quand même.
    assert!(h.keys_in(None).contains(&"DEMO-2"));
    assert!(h.app.data.active.is_some());
    let screen = h.render(160, 30);
    let last = screen.lines().last().unwrap();
    assert!(last.contains("403"), "barre de statut : {last}");
}

/// Jira signale certains refus de droits en 400/404 (pas 403) : indice « droits ? » ajouté.
#[tokio::test]
async fn permission_hint_on_400_mentioning_permission() {
    let mut h = H::started().await;
    h.api.fail_on_msg(
        "update_version",
        400,
        "You do not have permission to edit versions.",
    );
    h.press(KeyCode::Char('3')).await;
    h.press(KeyCode::Char('r')).await;
    h.press(KeyCode::Enter).await; // Aucune
    h.press(KeyCode::Enter).await;
    let status = &h.app.status.text;
    assert!(h.app.status.error);
    assert!(status.starts_with("Échec — Livrer la version"), "{status}");
    assert!(status.ends_with("droits Jira insuffisants ?"), "{status}");

    // 400 sans rapport avec les droits : pas d'indice.
    h.api
        .fail_on_msg("rank", 400, "rankBeforeIssue : issue inconnue");
    h.press(KeyCode::Char('1')).await;
    h.select_backlog("DEMO-9");
    h.press(KeyCode::Char('K')).await;
    h.press(KeyCode::Enter).await;
    assert!(h.app.status.error);
    assert!(
        !h.app.status.text.contains("droits"),
        "{}",
        h.app.status.text
    );
}

#[test]
fn error_text_permission_hints() {
    use runtime::error_text;
    let forbidden = error_text(&Error::Forbidden("non".into()));
    assert!(forbidden.ends_with("droits Jira insuffisants pour cette action"));
    let not_found = error_text(&Error::NotFound(
        "Version does not exist or you do not have permission to see it".into(),
    ));
    assert!(
        not_found.ends_with("droits Jira insuffisants ?"),
        "{not_found}"
    );
    let plain = error_text(&Error::Api {
        status: 400,
        message: "name : A version with this name already exists".into(),
    });
    assert!(!plain.contains("droits"), "{plain}");
    assert!(!error_text(&Error::RateLimited { retry_after: None }).contains("droits"));
}

/// Livraison : issues de la version non chargées (échec) → refus local, aucune écriture.
#[tokio::test]
async fn release_refused_when_version_issues_failed_to_load() {
    let mut h = H::started().await;
    h.api.fail_on("version_issues", 500);
    h.press(KeyCode::Char('3')).await;
    assert!(h.app.versions.issues_failed);
    h.press(KeyCode::Char('r')).await;
    assert!(h.app.popup.is_none());
    assert!(h.app.status.error);
    assert!(h.app.status.text.contains("R pour réessayer"));
    assert!(h.api.writes().is_empty());
}

/// Livraison avec déplacement des issues non terminées (MockJira).
#[tokio::test]
async fn release_moves_unfinished_to_other_version() {
    let mut h = H::started().await;
    h.press(KeyCode::Char('3')).await;
    h.press(KeyCode::Char('r')).await;
    h.press(KeyCode::Char('j')).await; // 2.0.0
    h.press(KeyCode::Enter).await;
    let confirm = h.render(200, 40);
    assert!(
        confirm.contains(
            "Jira déplacera les issues non résolues (≈ 4 d'après leur statut) vers 2.0.0"
        ),
        "{confirm}"
    );
    assert!(
        !confirm.contains("DEMO-2, DEMO-3"),
        "pas de liste affirmée : {confirm}"
    );
    // Annuler revient au choix de destination.
    h.press(KeyCode::Esc).await;
    assert!(matches!(
        h.app.popup,
        Some(Popup::ReleaseVersion { cursor: 1, .. })
    ));
    assert!(h.render(120, 40).contains("Livrer 1.1.0"));
    h.press(KeyCode::Enter).await;
    h.press(KeyCode::Enter).await;
    assert_eq!(
        h.app.status.text,
        "Version 1.1.0 livrée (issues non résolues → 2.0.0)"
    );
    assert_eq!(
        h.app.data.find_issue("DEMO-3").unwrap().fix_versions[0].name,
        "2.0.0"
    );
    assert_eq!(h.api.writes(), ["update_version 101"]);
}

#[tokio::test]
async fn load_error_shows_in_status_bar() {
    let mut h = H::new();
    h.api.fail_on("projects", 500);
    let effects = update::init(&mut h.app);
    h.settle(effects).await;
    assert!(h.app.status.error);
    assert!(h.app.status.text.starts_with("Projets : erreur Jira 500"));
    assert!(h.render(100, 20).contains("erreur Jira 500"));
}

#[tokio::test]
async fn transitions_error_closes_popup() {
    let mut h = H::started().await;
    h.api.fail_on("transitions", 404);
    h.select_backlog("DEMO-8");
    h.press(KeyCode::Char('t')).await;
    assert!(h.app.popup.is_none());
    assert!(h.app.status.error);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn task_panic_quits_with_error() {
    let api = Arc::new(TestApi::default());
    *api.panic_on.lock().unwrap() = Some("projects");
    let dyn_api: Arc<dyn JiraApi> = api;
    let (tx, mut rx) = unbounded_channel();
    let mut app = App::new(date(2026, 11, 11), TimeZone::UTC);
    for effect in update::init(&mut app) {
        dispatch(&dyn_api, &tx, effect);
    }
    let event = rx.recv().await.unwrap();
    assert!(matches!(&event, AppEvent::Msg(Msg::TaskFailed(_))));
    update::update(&mut app, event);
    assert!(app.should_quit);
    assert!(app.fatal.as_deref().unwrap().contains("panic"));
}

#[tokio::test]
async fn dispatch_delivers_result_through_channel() {
    let dyn_api: Arc<dyn JiraApi> = Arc::new(TestApi::default());
    let (tx, mut rx) = unbounded_channel();
    dispatch(&dyn_api, &tx, Effect::LoadProjects);
    let Some(AppEvent::Msg(Msg::Projects(Ok(projects)))) = rx.recv().await else {
        panic!("projets attendus")
    };
    assert_eq!(projects.len(), 2);
}

#[test]
fn input_error_quits() {
    let mut app = App::new(date(2026, 11, 11), TimeZone::UTC);
    update::update(&mut app, AppEvent::InputError("tty fermé".into()));
    assert!(app.should_quit);
    assert!(app.fatal.is_some());
}

// ------------------------------------------------------------------ Rendus

#[tokio::test]
async fn render_main_views() {
    let mut h = H::started().await;
    let backlog = h.render(120, 30);
    assert!(backlog.contains("1 Backlog"));
    assert!(backlog.contains("DEMO · DEMO board"));
    assert!(backlog.contains("DEMO Sprint 5"));
    assert!(backlog.contains("Traduction anglaise"));
    assert!(backlog.contains("? aide"));

    h.press(KeyCode::Char('2')).await;
    let sprint = h.render(120, 30);
    assert!(sprint.contains("DEMO Sprint 4"));
    assert!(sprint.contains("04/11/2026 → 18/11/2026 (J-7)"));
    assert!(sprint.contains("À faire"));
    assert!(sprint.contains("En cours"));
    assert!(sprint.contains("Terminé"));
    assert!(sprint.contains("5/19 pts terminés"));

    h.press(KeyCode::Char('3')).await;
    let versions = h.render(120, 30);
    assert!(versions.contains("Non livrées"));
    assert!(versions.contains("Livrées"));
    assert!(versions.contains("Issues de 1.1.0"));
    assert!(versions.contains("30/11/2026"));
}

#[tokio::test]
async fn render_popups() {
    let mut h = H::started().await;
    h.press(KeyCode::Char('?')).await;
    assert!(h.render(120, 40).contains("Aide — raccourcis"));
    h.press(KeyCode::Esc).await;

    h.select_backlog("DEMO-8");
    h.press(KeyCode::Enter).await;
    let detail = h.render(120, 40);
    assert!(detail.contains("Description fictive de DEMO-8."));
    h.press(KeyCode::Esc).await;

    h.press(KeyCode::Char('J')).await;
    let confirm = h.render(120, 40);
    assert!(confirm.contains("Confirmer — Changer le rang"));
    assert!(confirm.contains("Placer DEMO-8 après DEMO-9"));
    h.press(KeyCode::Esc).await;

    h.press(KeyCode::Char('c')).await;
    let form = h.render(120, 40);
    assert!(form.contains("Nouvelle issue (DEMO)"));
    assert!(form.contains("Story points"));

    // Petit terminal : pas de panique.
    let _ = h.render(30, 8);
}

#[tokio::test]
async fn render_loading_indicator() {
    let mut h = H::new();
    let _pending = update::init(&mut h.app);
    let screen = h.render(100, 20);
    assert!(screen.contains("Chargement…"));
}

// ------------------------------------------------------------------ B2 : confirmation fantôme

fn ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

/// Rafale « résumé ⏎ suite » (collage sans bracketed paste) : l'Entrée soumet le formulaire,
/// mais aucune frappe du même lot ne peut valider la confirmation, jamais dessinée.
#[tokio::test]
async fn burst_after_submit_never_confirms() {
    let mut h = H::started().await;
    h.press(KeyCode::Char('c')).await;
    h.chars("Corriger le bug").await;
    let burst: Vec<KeyEvent> = std::iter::once(key(KeyCode::Enter))
        .chain("voir log".chars().map(|c| key(KeyCode::Char(c))))
        .chain([key(KeyCode::Enter), key(KeyCode::Char('y'))])
        .collect();
    h.burst(&burst).await;
    assert!(matches!(h.confirm_op(), WriteOp::CreateIssue(n) if n.summary == "Corriger le bug"));
    assert!(
        h.api.writes().is_empty(),
        "aucune écriture sans confirmation vue"
    );
    assert!(h.app.status.text.contains("Validation ignorée"));

    // Une fois la confirmation affichée et lue : validation acceptée.
    h.press(KeyCode::Enter).await;
    assert_eq!(h.api.writes(), ["create_issue Corriger le bug"]);
}

/// `m`, Entrée, Entrée tapés d'un bloc : la destination est choisie, mais rien n'est déplacé.
#[tokio::test]
async fn typed_ahead_move_is_not_confirmed() {
    let mut h = H::started().await;
    h.select_backlog("DEMO-8");
    h.burst(&[
        key(KeyCode::Char('m')),
        key(KeyCode::Enter),
        key(KeyCode::Enter),
    ])
    .await;
    assert!(matches!(h.confirm_op(), WriteOp::MoveToSprint { .. }));
    assert!(h.api.writes().is_empty());
}

/// Confirmation dessinée mais validée trop vite (< délai), ou après une frappe parasite :
/// ignorée ; chaque frappe prématurée repousse l'échéance.
#[tokio::test]
async fn confirm_needs_delay_after_display() {
    let mut h = H::started().await;
    h.select_backlog("DEMO-9");
    h.press(KeyCode::Char('K')).await;
    let t0 = h.clock + Duration::from_secs(1);
    h.app.mark_drawn(t0);
    let at = |ms| t0 + Duration::from_millis(ms);
    let fx = update::update(&mut h.app, AppEvent::Key(key(KeyCode::Enter), at(100)));
    assert!(fx.is_empty());
    let fx = update::update(&mut h.app, AppEvent::Key(key(KeyCode::Char('o')), at(450)));
    assert!(fx.is_empty(), "délai repoussé par la frappe à 100 ms");
    let fx = update::update(&mut h.app, AppEvent::Key(key(KeyCode::Char('x')), at(900)));
    assert!(fx.is_empty());
    let fx = update::update(&mut h.app, AppEvent::Key(key(KeyCode::Enter), at(1200)));
    assert!(fx.is_empty(), "frappe parasite à 900 ms : délai repoussé");
    let fx = update::update(&mut h.app, AppEvent::Key(key(KeyCode::Enter), at(1700)));
    assert!(matches!(&fx[..], [Effect::Write(WriteOp::Rank { .. })]));
    h.clock = at(1700);
    h.settle(fx).await;
    assert_eq!(h.api.writes().len(), 1);
}

/// Jamais dessinée : même longtemps après, aucune validation (la popup n'a pas été vue).
#[tokio::test]
async fn undrawn_confirm_cannot_be_validated() {
    let mut h = H::started().await;
    h.select_backlog("DEMO-9");
    let t = h.clock + Duration::from_secs(1);
    let fx = update::update(&mut h.app, AppEvent::Key(key(KeyCode::Char('K')), t));
    assert!(fx.is_empty());
    let later = t + Duration::from_secs(60);
    let fx = update::update(&mut h.app, AppEvent::Key(key(KeyCode::Enter), later));
    assert!(fx.is_empty());
    assert!(matches!(
        h.app.popup,
        Some(Popup::Confirm { shown_at: None, .. })
    ));
}

/// Bracketed paste : texte inséré dans le champ actif, sauts de ligne → espace dans un champ
/// d'une ligne, conservés dans la description ; jamais interprété comme des touches.
#[tokio::test]
async fn paste_goes_into_active_field() {
    let mut h = H::started().await;
    h.paste("q").await;
    assert!(!h.app.should_quit, "collage hors formulaire ignoré");
    h.press(KeyCode::Char('c')).await;
    h.paste("Corriger le bug\r\nvoir log").await;
    let Some(Popup::Form(form)) = &h.app.popup else {
        panic!("le formulaire reste ouvert : {:?}", h.app.popup)
    };
    assert_eq!(
        form.text(super::form::FieldId::Summary),
        "Corriger le bug voir log"
    );
    for _ in 0..4 {
        h.press(KeyCode::Tab).await;
    }
    h.paste("l1\nl2").await;
    let Some(Popup::Form(form)) = &h.app.popup else {
        panic!("formulaire")
    };
    assert_eq!(form.text(super::form::FieldId::Description), "l1\nl2");
    h.ctrl_s().await;
    // Collage pendant la confirmation : ignoré, et repousse la validation.
    let before = match &h.app.popup {
        Some(Popup::Confirm { shown_at, .. }) => *shown_at,
        _ => None,
    };
    h.paste("\n").await;
    let Some(Popup::Confirm { shown_at, .. }) = &h.app.popup else {
        panic!("confirmation")
    };
    assert!(shown_at > &before);
    assert!(h.api.writes().is_empty());
}

// ------------------------------------------------------------------ B3 : clôture de sprint

/// Le sprint change entre la confirmation et l'exécution : abandon, aucune écriture.
#[tokio::test]
async fn close_sprint_aborts_when_unfinished_set_changed() {
    let mut h = H::started().await;
    h.press(KeyCode::Char('2')).await;
    h.press(KeyCode::Char('x')).await;
    h.press(KeyCode::Enter).await; // Backlog
    assert!(matches!(h.confirm_op(), WriteOp::CloseSprint { .. }));
    // Un collègue ajoute une issue au sprint pendant que la confirmation est affichée.
    h.api
        .inner
        .move_to_sprint(10, &["DEMO-8".into()])
        .await
        .unwrap();
    h.press(KeyCode::Enter).await;
    assert!(h.app.status.error);
    let status = h.app.status.text.clone();
    assert!(status.contains("abandonnée"), "{status}");
    assert!(status.contains("en plus : DEMO-8"), "{status}");
    assert!(h.api.writes().is_empty(), "rien écrit");
    assert_eq!(h.app.data.active.as_ref().map(|s| s.id), Some(10));
}

async fn subtask(key: &str, sprint: u64) -> Issue {
    let mut i = MockJira::new().sprint_issues(10).await.unwrap()[3].clone();
    i.id = format!("9{key}");
    i.key = key.into();
    i.issue_type = IssueType {
        id: "10009".into(),
        name: "Sous-tâche".into(),
        subtask: true,
    };
    i.sprint_id = Some(sprint);
    i
}

/// Les sous-tâches non terminées suivent leur parent : jamais déplacées explicitement.
#[tokio::test]
async fn close_sprint_excludes_subtasks() {
    let api = Arc::new(TestApi::default());
    let sub = subtask("DEMO-50", 10).await;
    api.extra_sprint_issues
        .lock()
        .unwrap()
        .insert(10, vec![sub]);
    let mut h = H::started_with(api).await;
    assert!(h.keys_in(Some(10)).contains(&"DEMO-50"));
    h.press(KeyCode::Char('2')).await;
    h.press(KeyCode::Char('x')).await;
    let Some(Popup::CloseSprint {
        unfinished, done, ..
    }) = &h.app.popup
    else {
        panic!("popup de clôture")
    };
    assert_eq!(unfinished, &["DEMO-2", "DEMO-3", "DEMO-4", "DEMO-5"]);
    assert_eq!(*done, 1);
    h.press(KeyCode::Enter).await;
    h.press(KeyCode::Enter).await;
    assert!(!h.app.status.error, "{}", h.app.status.text);
    assert_eq!(
        h.api.writes(),
        [
            "move_to_backlog [\"DEMO-2\", \"DEMO-3\", \"DEMO-4\", \"DEMO-5\"]",
            "close_sprint 10"
        ]
    );
}

fn extra_sprint(id: u64, board: u64, state: SprintState) -> Sprint {
    Sprint {
        id,
        board_id: Some(board),
        name: format!("Sprint {id} (board {board})"),
        state,
        goal: None,
        start_date: None,
        end_date: None,
    }
}

/// Sprints d'un autre board ignorés ; sprints parallèles du board : clôture désactivée.
#[tokio::test]
async fn foreign_and_parallel_active_sprints() {
    let api = Arc::new(TestApi::default());
    api.extra_sprints.lock().unwrap().extend([
        extra_sprint(77, 2, SprintState::Active),
        extra_sprint(79, 2, SprintState::Future),
    ]);
    let mut h = H::started_with(api).await;
    assert_eq!(h.app.data.active.as_ref().map(|s| s.id), Some(10));
    assert_eq!(
        h.app.data.future.iter().map(|s| s.id).collect::<Vec<_>>(),
        [11]
    );
    assert_eq!(h.app.data.foreign_sprints, 2);
    assert!(h.app.data.parallel_active.is_empty());

    h.api
        .extra_sprints
        .lock()
        .unwrap()
        .push(extra_sprint(78, 1, SprintState::Active));
    h.press(KeyCode::Char('R')).await;
    assert_eq!(h.app.data.active.as_ref().map(|s| s.id), Some(10));
    assert_eq!(h.app.data.parallel_active.len(), 1);
    h.press(KeyCode::Char('2')).await;
    assert!(h.render(200, 30).contains("clôture désactivée"));
    h.press(KeyCode::Char('x')).await;
    assert!(
        h.app.popup.is_none(),
        "pas de clôture d'un sprint au hasard"
    );
    assert!(h.app.status.error);
    assert!(h.app.status.text.contains("Sprints actifs en parallèle"));
    assert!(h.api.writes().is_empty());
}

// ------------------------------------------------------------------ B4 : écritures en vol

/// Une seule écriture à la fois (file ordonnée) ; `q`/Ctrl+C pendant une écriture : sortie
/// différée à la fin des écritures.
#[tokio::test]
async fn writes_are_serialized_and_quit_waits() {
    let mut h = H::started().await;
    h.select_backlog("DEMO-9");
    h.press(KeyCode::Char('K')).await;
    let first = h.press_deferred(KeyCode::Enter);
    assert!(matches!(&first[..], [Effect::Write(WriteOp::Rank { .. })]));
    assert!(h.app.write_in_flight);

    h.select_backlog("DEMO-12");
    h.press(KeyCode::Char('K')).await;
    h.press(KeyCode::Enter).await;
    assert_eq!(h.app.write_queue.len(), 1, "deuxième écriture en file");
    assert!(h.app.status.text.contains("en file d'attente"));

    h.press(KeyCode::Char('q')).await;
    assert!(!h.app.should_quit);
    assert!(h.app.status.text.starts_with("Écriture en cours (2)"));
    h.send(ctrl('c')).await;
    assert!(!h.app.should_quit);
    assert!(h.api.writes().is_empty());

    h.settle_deferred(first).await;
    assert_eq!(
        h.api.writes(),
        [
            "rank [\"DEMO-9\"] Before(\"DEMO-8\")",
            "rank [\"DEMO-12\"] Before(\"DEMO-11\")"
        ],
        "exécutées dans l'ordre, l'une après l'autre"
    );
    assert!(h.app.should_quit, "sortie une fois la file vide");
}

/// Sortie différée annulée si la dernière écriture échoue (l'erreur reste visible).
#[tokio::test]
async fn deferred_quit_cancelled_on_failed_write() {
    let mut h = H::started().await;
    h.api.fail_on("rank", 500);
    h.select_backlog("DEMO-9");
    h.press(KeyCode::Char('K')).await;
    let fx = h.press_deferred(KeyCode::Enter);
    h.press(KeyCode::Char('q')).await;
    h.settle_deferred(fx).await;
    assert!(!h.app.should_quit);
    assert!(h.app.status.error);
    h.press(KeyCode::Char('q')).await;
    assert!(h.app.should_quit);
}

// ------------------------------------------------------------------ I3 : réponses périmées

/// Deux rafraîchissements du même board : la réponse de l'ancien, arrivée en dernier, est
/// ignorée.
#[tokio::test]
async fn older_refresh_of_same_board_is_ignored() {
    let mut h = H::started().await;
    let old = h.press_deferred(KeyCode::Char('R'));
    let new = h.press_deferred(KeyCode::Char('R'));
    h.settle_deferred(new).await;
    assert!(h.keys_in(Some(11)).contains(&"DEMO-6"));
    // Le serveur change ensuite ; l'ancienne requête s'exécute maintenant et répond en dernier.
    h.api
        .inner
        .move_to_backlog(&["DEMO-6".into()])
        .await
        .unwrap();
    h.settle_deferred(old).await;
    assert!(
        h.keys_in(Some(11)).contains(&"DEMO-6"),
        "réponse de la requête dépassée ignorée"
    );
}

/// Même version redemandée (101 → 102 → 101) : seule la dernière requête est acceptée.
#[tokio::test]
async fn older_request_for_same_version_is_ignored() {
    let mut h = H::started().await;
    let first = h.press_deferred(KeyCode::Char('3'));
    let second = h.press_deferred(KeyCode::Char('j'));
    let third = h.press_deferred(KeyCode::Char('k'));
    assert!(
        matches!(&third[..], [Effect::LoadVersionIssues { version_id, .. }] if version_id == "101")
    );
    h.settle_deferred(third).await;
    assert_eq!(h.app.versions.issues.len(), 5);
    let update = IssueUpdate {
        fix_version_ids: Some(vec![]),
        ..IssueUpdate::default()
    };
    h.api.inner.update_issue("DEMO-1", &update).await.unwrap();
    h.settle_deferred(first).await;
    h.settle_deferred(second).await;
    assert_eq!(h.app.versions.issues_for.as_deref(), Some("101"));
    assert_eq!(
        h.app.versions.issues.len(),
        5,
        "réponse périmée de 101 ignorée"
    );
}

// ------------------------------------------------------------------ I6 / I7

/// Plusieurs fixVersions : champ en lecture seule, `fix_version_ids` jamais envoyé.
#[tokio::test]
async fn multiple_fix_versions_are_never_overwritten() {
    let mut h = H::started().await;
    let update = IssueUpdate {
        fix_version_ids: Some(vec!["101".into(), "102".into()]),
        ..IssueUpdate::default()
    };
    h.api.inner.update_issue("DEMO-8", &update).await.unwrap();
    h.press(KeyCode::Char('R')).await;
    h.select_backlog("DEMO-8");
    h.press(KeyCode::Char('e')).await;
    let screen = h.render(160, 40);
    assert!(screen.contains("plusieurs versions"), "{screen}");
    h.chars(" !").await;
    h.ctrl_s().await;
    let WriteOp::UpdateIssue { update, .. } = h.confirm_op() else {
        panic!("modification attendue")
    };
    assert_eq!(update.fix_version_ids, None);
    h.press(KeyCode::Enter).await;
    assert_eq!(
        h.app.data.find_issue("DEMO-8").unwrap().fix_versions.len(),
        2
    );
}

/// La confirmation affiche le nom de la version, pas son id.
#[tokio::test]
async fn edit_confirmation_shows_version_names() {
    let mut h = H::started().await;
    h.select_backlog("DEMO-9");
    h.press(KeyCode::Char('e')).await;
    h.press(KeyCode::Tab).await; // points
    h.press(KeyCode::Tab).await; // version
    h.press(KeyCode::Right).await;
    h.ctrl_s().await;
    let (_, lines) = h.confirm_op().describe();
    let line = lines.iter().find(|l| l.contains("version →")).unwrap();
    assert!(line.ends_with(".0"), "nom de version attendu : {line}");
    assert!(!line.contains("10"), "pas d'id : {line}");
}

// ------------------------------------------------------------------ Mineurs

/// Ctrl+C dans un formulaire modifié : confirmation ; non modifié : sortie directe.
#[tokio::test]
async fn ctrl_c_in_modified_form_asks_first() {
    let mut h = H::started().await;
    h.press(KeyCode::Char('c')).await;
    h.chars("Brouillon").await;
    h.send(ctrl('c')).await;
    assert!(!h.app.should_quit);
    assert!(matches!(h.app.popup, Some(Popup::QuitConfirm { .. })));
    assert!(h.render(120, 40).contains("Quitter ?"));
    h.press(KeyCode::Esc).await;
    assert!(
        matches!(h.app.popup, Some(Popup::Form(_))),
        "retour au formulaire"
    );
    // Derrière une confirmation aussi.
    h.ctrl_s().await;
    h.send(ctrl('c')).await;
    assert!(matches!(h.app.popup, Some(Popup::QuitConfirm { .. })));
    h.press(KeyCode::Enter).await;
    assert!(h.app.should_quit);
    assert!(h.api.writes().is_empty());

    let mut h = H::started().await;
    h.press(KeyCode::Char('c')).await;
    h.send(ctrl('c')).await;
    assert!(h.app.should_quit, "formulaire vierge : sortie directe");
}

#[test]
fn tick_refreshes_today() {
    let mut app = App::new(date(2026, 11, 11), TimeZone::UTC);
    update::update(&mut app, AppEvent::Tick(date(2026, 11, 11)));
    assert_eq!(app.today, date(2026, 11, 11));
    update::update(&mut app, AppEvent::Tick(date(2026, 11, 12)));
    assert_eq!(app.today, date(2026, 11, 12));
}

/// Chaque popup rendue dans des terminaux minuscules : aucune panique.
#[tokio::test]
async fn tiny_terminals_never_panic_with_any_popup() {
    let mut h = H::started().await;
    let issue = h.app.data.find_issue("DEMO-8").unwrap().clone();
    let version = h.app.data.versions[1].clone();
    let sprint = h.app.data.active.clone().unwrap();
    let form = super::form::Form::edit_issue(&issue, &h.app.data.versions);
    let op = WriteOp::Rank {
        key: "DEMO-8".into(),
        position: RankPosition::After("DEMO-9".into()),
    };
    let popups = vec![
        Popup::Help,
        Popup::Detail(Box::new(issue.clone())),
        Popup::Transitions {
            key: "DEMO-8".into(),
            items: None,
            cursor: 0,
        },
        Popup::Transitions {
            key: "DEMO-8".into(),
            items: Some(vec![Transition {
                id: "1".into(),
                name: "Démarrer".into(),
                to: issue.status.clone(),
                has_screen: true,
            }]),
            cursor: 0,
        },
        Popup::Move {
            keys: vec!["DEMO-8".into()],
            dests: vec![MoveDest::Backlog],
            cursor: 0,
        },
        Popup::PickSprint {
            sprints: vec![sprint.clone()],
            cursor: 0,
        },
        Popup::CloseSprint {
            sprint: Box::new(sprint),
            unfinished: vec!["DEMO-2".into()],
            done: 1,
            dests: vec![MoveDest::Backlog],
            cursor: 0,
        },
        Popup::ReleaseVersion {
            version: Box::new(version),
            unfinished: vec!["DEMO-2".into()],
            dests: vec![VersionDest {
                id: None,
                label: "Aucune".into(),
            }],
            cursor: 0,
        },
        Popup::Form(Box::new(form.clone())),
        Popup::confirm(op, None),
        Popup::QuitConfirm {
            back: Box::new(Popup::Form(Box::new(form))),
        },
    ];
    for tab in Tab::ALL {
        h.app.tab = tab;
        for p in &popups {
            h.app.popup = Some(p.clone());
            for (w, hgt) in [(1, 1), (10, 3), (0, 0), (2, 1)] {
                let _ = h.render(w, hgt);
            }
        }
    }
}
