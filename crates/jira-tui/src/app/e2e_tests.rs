//! Tests de bout en bout TUI → `JiraHttp` → `jira-fake` (faux Jira Cloud HTTP en mémoire).
//!
//! Chaque test démarre son propre serveur sur un port éphémère de 127.0.0.1 (jamais d'autre
//! interface, jamais de vrai Jira) et rejoue les scénarios via le même harnais que `tests.rs`
//! (`update` → `runtime::execute` → `Msg`). Le but est de détecter les divergences entre
//! `MockJira` et le comportement HTTP réel du client.

use std::sync::Arc;
use std::time::Duration;

use crossterm::event::KeyCode;
use jira_client::mock::MockJira;
use jira_client::model::*;
use jira_client::{JiraHttp, RetryPolicy, SecretString};
use jira_fake::Options;

use super::action::{Effect, Msg, WriteOp};
use super::runtime;
use super::state::*;
use super::tests::H;

const UNFINISHED_SPRINT_4: [&str; 4] = ["DEMO-2", "DEMO-3", "DEMO-4", "DEMO-5"];

/// Démarre jira-fake sur 127.0.0.1:0 et renvoie son URL de base.
async fn start_fake(opts: Options) -> String {
    let listener = jira_fake::bind(0).await.expect("bind 127.0.0.1:0");
    let addr = listener.local_addr().unwrap();
    assert!(addr.ip().is_loopback(), "jira-fake doit rester en loopback");
    tokio::spawn(jira_fake::serve(
        listener,
        Options {
            quiet: true,
            ..opts
        },
        std::future::pending(),
    ));
    format!("http://{addr}")
}

fn http_client(base: &str, retry: RetryPolicy) -> JiraHttp {
    JiraHttp::new(
        base,
        "alice@example.com",
        SecretString::new("jeton-factice"),
    )
    .expect("client HTTP loopback")
    .with_retry_policy(retry)
}

/// Pas de nouvel essai lent dans les tests (aucun 429 attendu par défaut).
fn fast_retry(max_attempts: u32) -> RetryPolicy {
    RetryPolicy {
        max_attempts,
        base_delay: Duration::from_millis(5),
        max_delay: Duration::from_millis(50),
    }
}

async fn fake_api(opts: Options) -> Arc<JiraHttp> {
    let base = start_fake(opts).await;
    Arc::new(http_client(&base, fast_retry(1)))
}

async fn started(opts: Options) -> H<JiraHttp> {
    H::started_with(fake_api(opts).await).await
}

/// Données du board DEMO chargées par le runtime, quel que soit l'implémentation.
async fn load_demo(api: &dyn jira_client::JiraApi) -> BoardData {
    let msg = runtime::execute(
        api,
        Effect::LoadBoard {
            seq: 1,
            board_id: 1,
            project_key: "DEMO".into(),
        },
    )
    .await;
    match msg {
        Msg::Board { result: Ok(d), .. } => *d,
        other => panic!("board attendu, obtenu {other:?}"),
    }
}

fn keys(issues: &[Issue]) -> Vec<&str> {
    issues.iter().map(|i| i.key.as_str()).collect()
}

// ------------------------------------------------------------------ Parité MockJira / HTTP

/// Issue sans les données propres au jeu de test (identité de l'assignee : ids de compte et
/// e-mail fictifs différents entre `MockJira` et jira-fake ; seul le nom affiché compte).
fn normalized(i: &Issue) -> Issue {
    let mut i = i.clone();
    if let Some(a) = &mut i.assignee {
        a.account_id.clear();
        a.email = None;
    }
    i
}

/// Compare champ par champ les données chargées par `MockJira` et par `JiraHttp` → jira-fake.
#[tokio::test]
async fn mock_and_http_load_the_same_board() {
    let http = fake_api(Options::default()).await;
    let mock = MockJira::new();
    let a = load_demo(&mock).await;
    let b = load_demo(http.as_ref()).await;

    assert_eq!(a.active, b.active, "sprint actif");
    assert_eq!(a.future, b.future, "sprints futurs");
    assert_eq!(keys(&a.backlog), keys(&b.backlog), "ordre du backlog");
    let mut sprint_ids: Vec<_> = a.sprint_issues.keys().copied().collect();
    sprint_ids.sort_unstable();
    let mut http_ids: Vec<_> = b.sprint_issues.keys().copied().collect();
    http_ids.sort_unstable();
    assert_eq!(sprint_ids, http_ids);
    for id in sprint_ids {
        assert_eq!(
            keys(&a.sprint_issues[&id]),
            keys(&b.sprint_issues[&id]),
            "ordre du sprint {id}"
        );
    }
    for ia in a.backlog.iter().chain(a.sprint_issues.values().flatten()) {
        let ib = b.find_issue(&ia.key).expect("issue présente côté HTTP");
        assert_eq!(normalized(ia), normalized(ib), "issue {}", ia.key);
    }
    // Le jeu de données de jira-fake renseigne en plus description et date de début : seuls
    // les champs communs aux deux jeux sont comparés.
    let ver = |v: &Version| {
        (
            v.id.clone(),
            v.project_id.clone(),
            v.name.clone(),
            v.released,
            v.archived,
            v.release_date.clone(),
        )
    };
    assert_eq!(
        a.versions.iter().map(ver).collect::<Vec<_>>(),
        b.versions.iter().map(ver).collect::<Vec<_>>(),
        "versions"
    );
    assert_eq!(a.issue_types, b.issue_types, "types d'issue");
}

/// Mêmes transitions proposées (ids, cibles, écran requis) des deux côtés ; seuls les libellés
/// diffèrent (« Passer à « En cours » » côté mock, nom du statut côté jira-fake).
#[tokio::test]
async fn mock_and_http_offer_the_same_transitions() {
    use jira_client::JiraApi;
    let http = fake_api(Options::default()).await;
    let mock = MockJira::new();
    let strip = |v: Vec<Transition>| -> Vec<(String, Status, bool)> {
        v.into_iter().map(|t| (t.id, t.to, t.has_screen)).collect()
    };
    for key in ["DEMO-1", "DEMO-2", "DEMO-4"] {
        let a = strip(mock.transitions(key).await.unwrap());
        let b = strip(http.transitions(key).await.unwrap());
        assert_eq!(a, b, "transitions de {key}");
    }
    let todo = strip(http.transitions("DEMO-4").await.unwrap());
    assert!(
        todo.iter()
            .any(|(_, to, screen)| to.name == "Terminé" && *screen),
        "À faire → Terminé exige un écran (résolution)"
    );
}

// ------------------------------------------------------------------ Démarrage

#[tokio::test]
async fn startup_loads_project_and_board() {
    let h = started(Options::default()).await;
    let ctx = h.app.ctx.as_ref().unwrap();
    assert_eq!((ctx.project.key.as_str(), ctx.board.id), ("DEMO", 1));
    assert_eq!(ctx.board.name, "DEMO board");
    assert_eq!(h.app.data.active.as_ref().map(|s| s.id), Some(10));
    assert_eq!(
        h.keys_in(None),
        ["DEMO-8", "DEMO-9", "DEMO-10", "DEMO-11", "DEMO-12"]
    );
    assert_eq!(h.keys_in(Some(11)), ["DEMO-6", "DEMO-7"]);
    assert_eq!(
        h.keys_in(Some(10)),
        ["DEMO-1", "DEMO-2", "DEMO-3", "DEMO-4", "DEMO-5"]
    );
    assert!(!h.app.status.error, "{}", h.app.status.text);
    let screen = h.render(120, 30);
    assert!(screen.contains("DEMO · DEMO board"));
    assert!(screen.contains("Traduction anglaise"));
}

#[tokio::test]
async fn picker_switches_to_ops() {
    let mut h = started(Options::default()).await;
    h.press(KeyCode::Char('p')).await;
    h.press(KeyCode::Char('j')).await;
    h.press(KeyCode::Enter).await;
    assert_eq!(h.app.ctx.as_ref().unwrap().project.key, "OPS");
    assert_eq!(h.keys_in(None), ["OPS-1"]);
    assert!(h.app.data.active.is_none());
    assert!(!h.app.status.error, "{}", h.app.status.text);
}

// ------------------------------------------------------------------ Backlog

#[tokio::test]
async fn rank_up_and_down_in_backlog() {
    let mut h = started(Options::default()).await;
    h.select_backlog("DEMO-9");
    h.press(KeyCode::Char('K')).await;
    h.press(KeyCode::Enter).await;
    assert_eq!(h.app.status.text, "Rang de DEMO-9 mis à jour");
    assert_eq!(h.keys_in(None)[..2], ["DEMO-9", "DEMO-8"]);
    assert_eq!(h.selected(), Some("DEMO-9"));

    // Descente de deux crans : DEMO-9 passe après DEMO-8 puis après DEMO-10.
    h.press(KeyCode::Char('J')).await;
    h.press(KeyCode::Enter).await;
    h.press(KeyCode::Char('J')).await;
    h.press(KeyCode::Enter).await;
    assert_eq!(
        h.keys_in(None),
        ["DEMO-8", "DEMO-10", "DEMO-9", "DEMO-11", "DEMO-12"]
    );
    assert!(!h.app.status.error, "{}", h.app.status.text);
}

#[tokio::test]
async fn rank_inside_sprint_column() {
    let mut h = started(Options::default()).await;
    h.press(KeyCode::Char('2')).await;
    // Colonne « À faire » : DEMO-4, DEMO-5.
    h.press(KeyCode::Char('j')).await;
    assert_eq!(h.selected(), Some("DEMO-5"));
    h.press(KeyCode::Char('K')).await;
    h.press(KeyCode::Enter).await;
    let col: Vec<_> = h.app.sprint_columns()[0]
        .iter()
        .map(|i| i.key.clone())
        .collect();
    assert_eq!(col, ["DEMO-5", "DEMO-4"]);
    assert_eq!(h.selected(), Some("DEMO-5"));
}

#[tokio::test]
async fn move_backlog_to_sprints_and_back() {
    let mut h = started(Options::default()).await;
    // Backlog → sprint actif.
    h.select_backlog("DEMO-8");
    h.press(KeyCode::Char('m')).await;
    h.press(KeyCode::Enter).await;
    h.press(KeyCode::Enter).await;
    assert!(!h.app.status.error, "{}", h.app.status.text);
    assert!(h.keys_in(Some(10)).contains(&"DEMO-8"));
    assert!(!h.keys_in(None).contains(&"DEMO-8"));
    assert_eq!(h.app.data.find_issue("DEMO-8").unwrap().sprint_id, Some(10));

    // Backlog → sprint futur.
    h.select_backlog("DEMO-12");
    h.press(KeyCode::Char('m')).await;
    h.press(KeyCode::Char('j')).await;
    h.press(KeyCode::Enter).await;
    assert!(matches!(
        h.confirm_op(),
        WriteOp::MoveToSprint { sprint_id: 11, .. }
    ));
    h.press(KeyCode::Enter).await;
    assert_eq!(h.keys_in(Some(11)), ["DEMO-6", "DEMO-7", "DEMO-12"]);
    assert_eq!(h.selected(), Some("DEMO-12"), "la sélection suit l'issue");

    // Sprint actif → backlog (DEMO-8 « À faire », dernière de sa colonne).
    h.press(KeyCode::Char('2')).await;
    h.press(KeyCode::Char('G')).await;
    assert_eq!(h.selected(), Some("DEMO-8"));
    h.press(KeyCode::Char('m')).await;
    h.press(KeyCode::Char('G')).await;
    h.press(KeyCode::Enter).await;
    assert!(matches!(h.confirm_op(), WriteOp::MoveToBacklog { .. }));
    h.press(KeyCode::Enter).await;
    assert!(h.keys_in(None).contains(&"DEMO-8"));
    assert_eq!(h.app.data.find_issue("DEMO-8").unwrap().sprint_id, None);
    assert!(!h.app.status.error, "{}", h.app.status.text);
}

// ------------------------------------------------------------------ Transitions

/// Sélectionne l'issue `key` dans l'onglet Sprint et choisit la transition vers `to`.
async fn transition(h: &mut H<JiraHttp>, key: &str, to: &str) {
    h.press(KeyCode::Char('2')).await;
    let (c, r) = h
        .app
        .sprint_columns()
        .iter()
        .enumerate()
        .find_map(|(c, col)| col.iter().position(|i| i.key == key).map(|r| (c, r)))
        .expect("issue dans le sprint actif");
    h.app.sprint.column = c;
    h.app.sprint.rows[c] = r;
    h.press(KeyCode::Char('t')).await;
    let Some(Popup::Transitions {
        items: Some(items), ..
    }) = &h.app.popup
    else {
        panic!("transitions chargées attendues : {:?}", h.app.popup)
    };
    let pos = items.iter().position(|t| t.to.name == to).expect(to);
    for _ in 0..pos {
        h.press(KeyCode::Char('j')).await;
    }
    h.press(KeyCode::Enter).await;
    assert!(matches!(h.confirm_op(), WriteOp::Transition { to_status, .. } if to_status == to));
    h.press(KeyCode::Enter).await;
}

#[tokio::test]
async fn transitions_success_and_resolution_required() {
    let mut h = started(Options::default()).await;
    transition(&mut h, "DEMO-4", "En cours").await;
    assert_eq!(h.app.status.text, "DEMO-4 → En cours");
    assert_eq!(
        h.app.data.find_issue("DEMO-4").unwrap().status.category,
        StatusCategory::InProgress
    );
    assert_eq!(h.selected(), Some("DEMO-4"));

    // En cours → Terminé : sans écran, passe.
    transition(&mut h, "DEMO-4", "Terminé").await;
    assert_eq!(h.app.status.text, "DEMO-4 → Terminé");

    // À faire → Terminé : Jira exige la résolution (écran) → 400 affiché en barre de statut.
    transition(&mut h, "DEMO-5", "Terminé").await;
    assert!(h.app.status.error);
    let status = h.app.status.text.clone();
    assert!(status.starts_with("Échec — Transition"), "{status}");
    assert!(status.contains("400"), "{status}");
    assert!(status.to_lowercase().contains("resolution"), "{status}");
    assert!(
        !status.contains("droits"),
        "pas un problème de droits : {status}"
    );
    assert_eq!(
        h.app.data.find_issue("DEMO-5").unwrap().status.name,
        "À faire",
        "statut inchangé après l'échec"
    );
    let screen = h.render(200, 30);
    assert!(screen.lines().last().unwrap().contains("400"), "{screen}");
}

// ------------------------------------------------------------------ Issues

#[tokio::test]
async fn create_then_edit_issue() {
    let mut h = started(Options::default()).await;
    h.press(KeyCode::Char('c')).await;
    h.chars("Story e2e").await;
    h.press(KeyCode::Tab).await; // type
    h.press(KeyCode::Right).await; // Bug
    h.press(KeyCode::Tab).await; // points
    h.chars("3").await;
    h.press(KeyCode::Tab).await; // version
    h.press(KeyCode::Right).await; // 1.1.0
    h.press(KeyCode::Tab).await; // description
    h.chars("Créée par le test e2e.").await;
    h.ctrl_s().await;
    let WriteOp::CreateIssue(new) = h.confirm_op() else {
        panic!("création attendue")
    };
    assert_eq!(new.fix_version_ids, ["101"]);
    h.press(KeyCode::Enter).await;
    assert!(!h.app.status.error, "{}", h.app.status.text);
    let key = h
        .selected()
        .expect("nouvelle issue sélectionnée")
        .to_string();
    assert_eq!(h.app.status.text, format!("Issue {key} créée"));
    let created = h.app.data.find_issue(&key).unwrap().clone();
    assert_eq!(created.summary, "Story e2e");
    assert_eq!(created.issue_type.name, "Bug");
    assert_eq!(created.story_points, Some(3.0));
    assert_eq!(
        created.description.as_deref(),
        Some("Créée par le test e2e.")
    );
    assert!(created.description_is_lossless);
    assert_eq!(
        created
            .fix_versions
            .iter()
            .map(|v| v.name.as_str())
            .collect::<Vec<_>>(),
        ["1.1.0"]
    );
    assert_eq!(created.status.category, StatusCategory::ToDo);
    assert_eq!(created.sprint_id, None);
    assert_eq!(
        h.keys_in(None).last(),
        Some(&key.as_str()),
        "en fin de backlog"
    );

    // Édition : résumé, points vidés, version retirée, description changée.
    h.press(KeyCode::Char('e')).await;
    h.chars(" (modifiée)").await;
    h.press(KeyCode::Tab).await; // points
    h.press(KeyCode::Backspace).await;
    h.press(KeyCode::Tab).await; // version
    h.press(KeyCode::Left).await; // (aucune)
    h.press(KeyCode::Tab).await; // description
    h.chars(" Suite.").await;
    h.ctrl_s().await;
    let WriteOp::UpdateIssue { update, .. } = h.confirm_op() else {
        panic!("modification attendue")
    };
    assert_eq!(update.story_points, Some(None));
    assert_eq!(update.fix_version_ids.as_deref(), Some(&[][..]));
    h.press(KeyCode::Enter).await;
    assert!(!h.app.status.error, "{}", h.app.status.text);
    let edited = h.app.data.find_issue(&key).unwrap();
    assert_eq!(edited.summary, "Story e2e (modifiée)");
    assert_eq!(edited.story_points, None);
    assert!(edited.fix_versions.is_empty());
    assert_eq!(
        edited.description.as_deref(),
        Some("Créée par le test e2e. Suite.")
    );
}

// ------------------------------------------------------------------ Sprints

#[tokio::test]
async fn close_sprint_moving_unfinished_to_next_then_start_it() {
    let mut h = started(Options::default()).await;
    h.press(KeyCode::Char('2')).await;
    h.press(KeyCode::Char('x')).await;
    let Some(Popup::CloseSprint { unfinished, .. }) = &h.app.popup else {
        panic!("popup de clôture")
    };
    assert_eq!(unfinished, &UNFINISHED_SPRINT_4);
    h.press(KeyCode::Char('j')).await; // DEMO Sprint 5
    h.press(KeyCode::Enter).await;
    h.press(KeyCode::Enter).await;
    assert_eq!(h.app.status.text, "DEMO Sprint 4 clôturé");
    assert!(h.app.data.active.is_none());
    assert_eq!(
        h.keys_in(Some(11)),
        ["DEMO-2", "DEMO-3", "DEMO-4", "DEMO-5", "DEMO-6", "DEMO-7"]
    );
    assert!(
        h.app.data.find_issue("DEMO-1").is_none(),
        "terminée : reste dans le sprint clos"
    );
    assert!(
        !h.keys_in(None)
            .iter()
            .any(|k| UNFINISHED_SPRINT_4.contains(k))
    );

    // Démarrage du sprint suivant (dates par défaut du formulaire).
    h.press(KeyCode::Char('s')).await;
    h.ctrl_s().await;
    assert!(matches!(
        h.confirm_op(),
        WriteOp::StartSprint { sprint_id: 11, .. }
    ));
    h.press(KeyCode::Enter).await;
    assert!(!h.app.status.error, "{}", h.app.status.text);
    assert_eq!(h.app.status.text, "DEMO Sprint 5 démarré");
    let active = h.app.data.active.as_ref().unwrap();
    assert_eq!(active.id, 11);
    assert!(
        active
            .start_date
            .as_deref()
            .unwrap()
            .starts_with("2026-11-11")
    );
    assert!(
        active
            .end_date
            .as_deref()
            .unwrap()
            .starts_with("2026-11-25")
    );
    assert!(h.app.data.future.is_empty());
    assert_eq!(
        h.app.sprint_columns().iter().map(Vec::len).sum::<usize>(),
        6
    );
}

#[tokio::test]
async fn close_sprint_moving_unfinished_to_backlog() {
    let mut h = started(Options::default()).await;
    h.press(KeyCode::Char('2')).await;
    h.press(KeyCode::Char('x')).await;
    h.press(KeyCode::Enter).await; // Backlog
    h.press(KeyCode::Enter).await;
    assert!(!h.app.status.error, "{}", h.app.status.text);
    assert!(h.app.data.active.is_none());
    for k in UNFINISHED_SPRINT_4 {
        assert!(h.keys_in(None).contains(&k), "{k} au backlog");
    }
    assert!(h.app.data.find_issue("DEMO-1").is_none());
}

// ------------------------------------------------------------------ Versions

#[tokio::test]
async fn versions_create_edit_and_release() {
    let mut h = started(Options::default()).await;
    h.press(KeyCode::Char('3')).await;
    let names: Vec<_> = h
        .app
        .data
        .versions_ordered()
        .iter()
        .map(|v| v.name.clone())
        .collect();
    assert_eq!(names, ["1.1.0", "2.0.0", "1.0.0"]);
    assert_eq!(h.app.versions.issues_for.as_deref(), Some("101"));
    assert_eq!(
        keys(&h.app.versions.issues),
        ["DEMO-1", "DEMO-2", "DEMO-3", "DEMO-4", "DEMO-5"]
    );
    h.press(KeyCode::Char('j')).await;
    assert_eq!(
        keys(&h.app.versions.issues),
        ["DEMO-6", "DEMO-7", "DEMO-8", "DEMO-11"]
    );

    // Création.
    h.press(KeyCode::Char('c')).await;
    h.chars("3.0.0").await;
    h.press(KeyCode::Tab).await;
    h.chars("2027-03-01").await;
    h.press(KeyCode::Tab).await;
    h.chars("2027-06-30").await;
    h.press(KeyCode::Tab).await;
    h.chars("Grande refonte").await;
    h.ctrl_s().await;
    h.press(KeyCode::Enter).await;
    assert_eq!(h.app.status.text, "Version 3.0.0 créée");
    let v = h.app.selected_version().unwrap().clone();
    assert_eq!(v.name, "3.0.0");
    assert_eq!(v.start_date.as_deref(), Some("2027-03-01"));
    assert_eq!(v.release_date.as_deref(), Some("2027-06-30"));
    assert_eq!(v.description.as_deref(), Some("Grande refonte"));
    assert_eq!(v.project_id, "10000");
    assert!(!v.released);
    assert!(h.app.versions.issues.is_empty());

    // Édition.
    h.press(KeyCode::Char('e')).await;
    h.chars("-rc").await;
    h.ctrl_s().await;
    h.press(KeyCode::Enter).await;
    assert!(!h.app.status.error, "{}", h.app.status.text);
    assert_eq!(h.app.selected_version().unwrap().name, "3.0.0-rc");

    // Nom dupliqué : 400 de Jira (errors.name) affiché.
    h.press(KeyCode::Char('c')).await;
    h.chars("2.0.0").await;
    h.ctrl_s().await;
    h.press(KeyCode::Enter).await;
    assert!(h.app.status.error);
    assert!(h.app.status.text.contains("400"), "{}", h.app.status.text);

    // Livraison d'une version sans issue non terminée : confirmation directe.
    let pos = h
        .app
        .data
        .versions_ordered()
        .iter()
        .position(|v| v.name == "3.0.0-rc")
        .unwrap();
    h.app.versions.cursor = pos;
    h.press(KeyCode::Char('j')).await;
    h.press(KeyCode::Char('k')).await;
    h.press(KeyCode::Char('r')).await;
    let WriteOp::ReleaseVersion { unfinished, .. } = h.confirm_op() else {
        panic!("livraison attendue")
    };
    assert!(unfinished.is_empty());
    h.press(KeyCode::Enter).await;
    assert_eq!(h.app.status.text, "Version 3.0.0-rc livrée");
    let released: Vec<_> = h
        .app
        .data
        .versions_ordered()
        .iter()
        .filter(|v| v.released)
        .map(|v| v.name.clone())
        .collect();
    assert_eq!(released, ["1.0.0", "3.0.0-rc"]);
}

#[tokio::test]
async fn release_version_moves_unfinished_issues() {
    let mut h = started(Options::default()).await;
    h.press(KeyCode::Char('3')).await; // 1.1.0 sélectionnée
    h.press(KeyCode::Char('r')).await;
    let Some(Popup::ReleaseVersion {
        unfinished, dests, ..
    }) = &h.app.popup
    else {
        panic!("choix de destination attendu : {:?}", h.app.popup)
    };
    assert_eq!(unfinished, &UNFINISHED_SPRINT_4);
    assert_eq!(
        dests.iter().map(|d| d.label.as_str()).collect::<Vec<_>>(),
        ["Aucune (restent dans 1.1.0)", "2.0.0"]
    );
    h.press(KeyCode::Char('j')).await;
    h.press(KeyCode::Enter).await;
    let WriteOp::ReleaseVersion { move_to, .. } = h.confirm_op() else {
        panic!("livraison attendue")
    };
    assert_eq!(move_to.as_ref().map(|(id, _)| id.as_str()), Some("102"));
    h.press(KeyCode::Enter).await;
    assert!(!h.app.status.error, "{}", h.app.status.text);
    assert_eq!(
        h.app.status.text,
        "Version 1.1.0 livrée (issues non résolues → 2.0.0)"
    );

    // Après rafraîchissement, 2.0.0 (première non livrée) est sélectionnée et contient les
    // issues déplacées ; 1.1.0 (dernière livrée) ne garde que l'issue terminée.
    assert!(
        h.app
            .data
            .versions
            .iter()
            .any(|v| v.id == "101" && v.released)
    );
    assert_eq!(h.app.selected_version().unwrap().id, "102");
    let got = keys(&h.app.versions.issues);
    for k in UNFINISHED_SPRINT_4 {
        assert!(got.contains(&k), "{k} déplacée vers 2.0.0 : {got:?}");
    }
    assert_eq!(
        h.app.data.find_issue("DEMO-3").unwrap().fix_versions[0].name,
        "2.0.0"
    );
    h.press(KeyCode::Char('G')).await;
    assert_eq!(h.app.selected_version().unwrap().id, "101");
    assert_eq!(keys(&h.app.versions.issues), ["DEMO-1"]);
}

#[tokio::test]
async fn release_version_keeping_unfinished_issues() {
    let mut h = started(Options::default()).await;
    h.press(KeyCode::Char('3')).await;
    h.press(KeyCode::Char('r')).await;
    h.press(KeyCode::Enter).await; // Aucune
    let WriteOp::ReleaseVersion { move_to, .. } = h.confirm_op() else {
        panic!("livraison attendue")
    };
    assert!(move_to.is_none());
    h.press(KeyCode::Enter).await;
    assert_eq!(h.app.status.text, "Version 1.1.0 livrée");
    assert_eq!(
        h.app.data.find_issue("DEMO-3").unwrap().fix_versions[0].name,
        "1.1.0"
    );
}

// ------------------------------------------------------------------ 429 et 403

/// 429 aléatoires (50 %) : les lectures sont rejouées par le client, la TUI ne voit rien.
#[tokio::test]
async fn rate_limited_reads_are_retried() {
    let base = start_fake(Options {
        fail_rate: 0.5,
        retry_after_secs: 0,
        ..Options::default()
    })
    .await;
    let api = Arc::new(http_client(&base, fast_retry(30)));
    let h = H::started_with(api).await;
    assert!(!h.app.status.error, "{}", h.app.status.text);
    assert_eq!(h.keys_in(Some(11)), ["DEMO-6", "DEMO-7"]);
}

/// 429 persistant : message clair en barre de statut, pas de blocage.
#[tokio::test]
async fn rate_limit_exhausted_shows_in_status_bar() {
    let base = start_fake(Options {
        fail_rate: 1.0,
        retry_after_secs: 0,
        ..Options::default()
    })
    .await;
    let mut h = H::with_api(Arc::new(http_client(&base, fast_retry(2))));
    let effects = super::update::init(&mut h.app);
    h.settle(effects).await;
    assert!(h.app.status.error);
    let status = &h.app.status.text;
    assert!(status.starts_with("Projets : "), "{status}");
    assert!(status.contains("429"), "{status}");
    assert!(h.render(120, 20).contains("429"));
}

/// `--forbid-writes` : lectures OK, chaque écriture → 403 expliqué, état rafraîchi.
#[tokio::test]
async fn forbidden_writes_show_clear_message() {
    let mut h = started(Options {
        forbid_writes: true,
        ..Options::default()
    })
    .await;
    assert!(
        !h.app.status.error,
        "lectures autorisées : {}",
        h.app.status.text
    );

    h.select_backlog("DEMO-9");
    h.press(KeyCode::Char('K')).await;
    h.press(KeyCode::Enter).await;
    let status = h.app.status.text.clone();
    assert!(h.app.status.error);
    assert!(status.starts_with("Échec — Changer le rang"), "{status}");
    assert!(status.contains("403"), "{status}");
    assert!(status.contains("droits Jira insuffisants"), "{status}");
    assert_eq!(
        h.keys_in(None)[..2],
        ["DEMO-8", "DEMO-9"],
        "rien n'a changé"
    );

    // Transition (POST) refusée aussi.
    transition(&mut h, "DEMO-4", "En cours").await;
    assert!(h.app.status.text.contains("403"), "{}", h.app.status.text);

    // Clôture : le déplacement préalable est refusé, le sprint reste actif.
    h.press(KeyCode::Char('x')).await;
    h.press(KeyCode::Enter).await;
    h.press(KeyCode::Enter).await;
    assert!(h.app.status.text.contains("403"), "{}", h.app.status.text);
    assert_eq!(h.app.data.active.as_ref().map(|s| s.id), Some(10));
    assert_eq!(h.keys_in(Some(10)).len(), 5);
}

// ------------------------------------------------------------------ Lecture seule

/// Client en lecture seule : les lectures passent, chaque méthode d'écriture du trait échoue en
/// `Error::ReadOnly` et l'état du serveur est strictement inchangé.
#[tokio::test]
async fn read_only_client_refuses_every_write_without_touching_jira() {
    use jira_client::{Error, JiraApi};

    let base = start_fake(Options::default()).await;
    let ro = http_client(&base, fast_retry(1)).with_read_only(true);
    let rw = http_client(&base, fast_retry(1));
    assert!(ro.is_read_only() && !rw.is_read_only());

    // Lectures (dont POST search/jql) : autorisées.
    assert_eq!(ro.myself().await.unwrap().display_name, "Alice Martin");
    let projects = ro.projects().await.unwrap();
    let demo = projects.iter().find(|p| p.key == "DEMO").unwrap();
    let types = ro.issue_types("DEMO").await.unwrap();
    let before = load_demo(&ro).await;
    let versions_before = rw.versions("DEMO").await.unwrap();
    let sprints_before = rw.sprints(1, &[]).await.unwrap();
    assert!(
        !before.backlog.is_empty(),
        "search/jql passe en lecture seule"
    );
    let transition_id = ro.transitions("DEMO-4").await.unwrap()[0].id.clone();

    let keys = vec!["DEMO-8".to_string()];
    let ro_err = |r: Result<(), Error>, what: &str| match r {
        Err(Error::ReadOnly(_)) => {}
        other => panic!("{what} : Error::ReadOnly attendu, obtenu {other:?}"),
    };
    ro_err(ro.move_to_sprint(11, &keys).await, "move_to_sprint");
    ro_err(
        ro.move_to_backlog(&["DEMO-1".into()]).await,
        "move_to_backlog",
    );
    ro_err(
        ro.rank(&keys, &RankPosition::Before("DEMO-9".into())).await,
        "rank",
    );
    let start = StartSprint {
        start_date: "2026-11-12T09:00:00.000Z".into(),
        end_date: "2026-11-26T17:00:00.000Z".into(),
        goal: None,
    };
    ro_err(ro.start_sprint(11, &start).await.map(drop), "start_sprint");
    ro_err(ro.close_sprint(10).await.map(drop), "close_sprint");
    ro_err(
        ro.create_sprint(1, "Sprint RO", None).await.map(drop),
        "create_sprint",
    );
    let su = SprintUpdate {
        name: Some("Renommé".into()),
        ..SprintUpdate::default()
    };
    ro_err(ro.update_sprint(11, &su).await.map(drop), "update_sprint");
    let ni = NewIssue {
        project_key: "DEMO".into(),
        issue_type_id: types[0].id.clone(),
        summary: "ne doit pas exister".into(),
        story_points: Some(3.0),
        ..NewIssue::default()
    };
    ro_err(ro.create_issue(&ni).await.map(drop), "create_issue");
    let iu = IssueUpdate {
        summary: Some("modifiée".into()),
        ..IssueUpdate::default()
    };
    ro_err(ro.update_issue("DEMO-8", &iu).await, "update_issue");
    ro_err(
        ro.transition_issue("DEMO-4", &transition_id).await,
        "transition_issue",
    );
    let nv = NewVersion {
        project_id: demo.id.clone(),
        name: "9.9.9".into(),
        ..NewVersion::default()
    };
    ro_err(ro.create_version(&nv).await.map(drop), "create_version");
    let target = versions_before
        .iter()
        .find(|v| !v.released && v.id != versions_before[0].id)
        .map(|v| v.id.clone());
    let vu = VersionUpdate {
        released: Some(true),
        move_unfixed_issues_to: target,
        ..VersionUpdate::default()
    };
    ro_err(
        ro.update_version(&versions_before[0].id, &vu)
            .await
            .map(drop),
        "update_version",
    );
    ro_err(
        ro.delete_version(&versions_before[0].id, None).await,
        "delete_version",
    );

    // Rien n'a changé côté serveur (relu par un client normal).
    assert_eq!(load_demo(&rw).await, before);
    assert_eq!(rw.versions("DEMO").await.unwrap(), versions_before);
    assert_eq!(rw.sprints(1, &[]).await.unwrap(), sprints_before);
}

/// TUI en lecture seule : la confirmation n'émet aucune écriture, un message l'explique,
/// et l'en-tête affiche le mode.
#[tokio::test]
async fn read_only_tui_blocks_confirmed_writes() {
    let base = start_fake(Options::default()).await;
    let api = Arc::new(http_client(&base, fast_retry(1)).with_read_only(true));
    let mut h = H::started_with(api).await;
    h.app.read_only = true;
    assert!(h.render(160, 20).contains("[LECTURE SEULE]"));

    h.select_backlog("DEMO-9");
    h.press(KeyCode::Char('K')).await;
    let effects = h.press_deferred(KeyCode::Enter);
    assert!(effects.is_empty(), "aucun effet d'écriture : {effects:?}");
    assert!(h.app.status.error);
    assert!(
        h.app.status.text.starts_with("Mode lecture seule"),
        "{}",
        h.app.status.text
    );
    assert_eq!(h.keys_in(None)[..2], ["DEMO-8", "DEMO-9"]);

    transition(&mut h, "DEMO-4", "En cours").await;
    assert!(h.app.status.text.starts_with("Mode lecture seule"));
    assert_eq!(
        h.app.data.find_issue("DEMO-4").unwrap().status.category,
        StatusCategory::ToDo
    );
}

/// Défense en profondeur : même si la TUI laissait passer une écriture, le client la refuse.
#[tokio::test]
async fn read_only_client_blocks_even_if_tui_lets_write_through() {
    let base = start_fake(Options::default()).await;
    let api = Arc::new(http_client(&base, fast_retry(1)).with_read_only(true));
    let mut h = H::started_with(api).await;
    assert!(!h.app.read_only);

    transition(&mut h, "DEMO-4", "En cours").await;
    assert!(h.app.status.error);
    assert!(
        h.app.status.text.contains("lecture seule"),
        "{}",
        h.app.status.text
    );
    assert_eq!(
        h.app.data.find_issue("DEMO-4").unwrap().status.category,
        StatusCategory::ToDo
    );
}

// ------------------------------------------------------------------ Corrections de relecture

/// B2 : un collage « résumé⏎suite » reste du texte ; une rafale Entrée/Entrée soumet le
/// formulaire mais ne confirme rien ; rien n'atteint jira-fake avant une confirmation vue.
#[tokio::test]
async fn paste_and_typed_ahead_enter_never_create_without_seen_confirmation() {
    let mut h = started(Options::default()).await;
    let backlog_before: Vec<String> = h.keys_in(None).iter().map(|k| k.to_string()).collect();
    h.press(KeyCode::Char('c')).await;
    h.paste("Corriger le bug\nvoir log").await;
    assert!(matches!(h.app.popup, Some(Popup::Form(_))));
    h.burst(&[
        super::tests::key(KeyCode::Enter),
        super::tests::key(KeyCode::Enter),
    ])
    .await;
    assert!(matches!(h.confirm_op(), WriteOp::CreateIssue(_)));
    // Relecture côté serveur : rien de créé.
    h.press(KeyCode::Esc).await;
    h.press(KeyCode::Esc).await;
    h.press(KeyCode::Char('R')).await;
    assert_eq!(h.keys_in(None), backlog_before);

    h.press(KeyCode::Char('c')).await;
    h.paste("Corriger le bug\nvoir log").await;
    h.press(KeyCode::Enter).await;
    h.press(KeyCode::Enter).await;
    let key = h.selected().unwrap().to_string();
    assert_eq!(
        h.app.data.find_issue(&key).unwrap().summary,
        "Corriger le bug voir log"
    );
}

/// B3 : le sprint est modifié par un tiers entre la confirmation et l'exécution → abandon,
/// rien n'est déplacé ni clôturé.
#[tokio::test]
async fn close_sprint_aborts_when_sprint_changed_meanwhile() {
    use jira_client::JiraApi;
    let base = start_fake(Options::default()).await;
    let other = http_client(&base, fast_retry(1));
    let mut h = H::started_with(Arc::new(http_client(&base, fast_retry(1)))).await;
    h.press(KeyCode::Char('2')).await;
    h.press(KeyCode::Char('x')).await;
    h.press(KeyCode::Enter).await; // Backlog
    assert!(matches!(h.confirm_op(), WriteOp::CloseSprint { .. }));
    other.move_to_backlog(&["DEMO-5".into()]).await.unwrap();
    h.press(KeyCode::Enter).await;
    let status = h.app.status.text.clone();
    assert!(h.app.status.error);
    assert!(status.contains("abandonnée"), "{status}");
    assert!(status.contains("en moins : DEMO-5"), "{status}");
    assert_eq!(h.app.data.active.as_ref().map(|s| s.id), Some(10));
    assert_eq!(
        h.keys_in(Some(10)),
        ["DEMO-1", "DEMO-2", "DEMO-3", "DEMO-4"],
        "rien déplacé par la TUI"
    );
}

/// I6 : issue à deux fixVersions éditée → les deux sont conservées côté serveur.
#[tokio::test]
async fn edit_keeps_multiple_fix_versions() {
    use jira_client::JiraApi;
    let base = start_fake(Options::default()).await;
    let other = http_client(&base, fast_retry(1));
    let update = IssueUpdate {
        fix_version_ids: Some(vec!["101".into(), "102".into()]),
        ..IssueUpdate::default()
    };
    other.update_issue("DEMO-8", &update).await.unwrap();
    let mut h = H::started_with(Arc::new(http_client(&base, fast_retry(1)))).await;
    assert_eq!(
        h.app.data.find_issue("DEMO-8").unwrap().fix_versions.len(),
        2
    );
    h.select_backlog("DEMO-8");
    h.press(KeyCode::Char('e')).await;
    h.chars(" !").await;
    h.ctrl_s().await;
    let WriteOp::UpdateIssue { update, .. } = h.confirm_op() else {
        panic!("modification attendue")
    };
    assert_eq!(update.fix_version_ids, None);
    h.press(KeyCode::Enter).await;
    assert!(!h.app.status.error, "{}", h.app.status.text);
    let names: Vec<_> = h
        .app
        .data
        .find_issue("DEMO-8")
        .unwrap()
        .fix_versions
        .iter()
        .map(|v| v.name.clone())
        .collect();
    assert_eq!(names, ["1.1.0", "2.0.0"]);
}

/// B4 : écritures sérialisées contre jira-fake, sortie différée jusqu'à la fin.
#[tokio::test]
async fn writes_serialized_and_quit_deferred_over_http() {
    let mut h = started(Options::default()).await;
    h.select_backlog("DEMO-9");
    h.press(KeyCode::Char('K')).await;
    let first = h.press_deferred(KeyCode::Enter);
    h.select_backlog("DEMO-12");
    h.press(KeyCode::Char('K')).await;
    h.press(KeyCode::Enter).await;
    assert_eq!(h.app.writes_pending(), 2);
    h.press(KeyCode::Char('q')).await;
    assert!(!h.app.should_quit);
    h.settle_deferred(first).await;
    assert!(h.app.should_quit);
    // L'app quitte sans recharger : état relu directement sur le serveur.
    let data = load_demo(h.api.as_ref()).await;
    assert_eq!(
        keys(&data.backlog),
        ["DEMO-9", "DEMO-8", "DEMO-10", "DEMO-12", "DEMO-11"]
    );
}
