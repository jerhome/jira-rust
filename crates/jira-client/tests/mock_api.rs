#![cfg(feature = "mock")]
//! Vérifie que MockJira respecte la sémantique attendue du contrat JiraApi.

use jira_client::mock::MockJira;
use jira_client::model::*;
use jira_client::{Error, JiraApi};

fn keys(issues: &[Issue]) -> Vec<&str> {
    issues.iter().map(|i| i.key.as_str()).collect()
}

#[tokio::test]
async fn navigation() {
    let m = MockJira::new();
    assert_eq!(m.projects().await.unwrap().len(), 2);
    let boards = m.scrum_boards("DEMO").await.unwrap();
    assert_eq!(boards[0].id, 1);
    let sprints = m
        .sprints(1, &[SprintState::Active, SprintState::Future])
        .await
        .unwrap();
    assert_eq!(
        sprints.iter().map(|s| s.id).collect::<Vec<_>>(),
        vec![10, 11]
    );
}

#[tokio::test]
async fn backlog_sprint_moves_and_rank() {
    let m = MockJira::new();
    let backlog = m.backlog_issues(1).await.unwrap();
    assert_eq!(
        keys(&backlog),
        vec!["DEMO-8", "DEMO-9", "DEMO-10", "DEMO-11", "DEMO-12"]
    );

    m.rank(&["DEMO-12".into()], &RankPosition::Before("DEMO-8".into()))
        .await
        .unwrap();
    m.rank(&["DEMO-9".into()], &RankPosition::After("DEMO-11".into()))
        .await
        .unwrap();
    let backlog = m.backlog_issues(1).await.unwrap();
    assert_eq!(
        keys(&backlog),
        vec!["DEMO-12", "DEMO-8", "DEMO-10", "DEMO-11", "DEMO-9"]
    );

    m.move_to_sprint(10, &["DEMO-12".into()]).await.unwrap();
    assert!(keys(&m.sprint_issues(10).await.unwrap()).contains(&"DEMO-12"));
    m.move_to_backlog(&["DEMO-12".into()]).await.unwrap();
    assert!(!keys(&m.sprint_issues(10).await.unwrap()).contains(&"DEMO-12"));
}

#[tokio::test]
async fn sprint_lifecycle() {
    let m = MockJira::new();
    let params = StartSprint {
        start_date: "2026-11-18T09:00:00.000Z".into(),
        end_date: "2026-12-02T17:00:00.000Z".into(),
        goal: Some("Tableau de bord".into()),
    };
    assert!(
        m.start_sprint(11, &params).await.is_err(),
        "un sprint est déjà actif"
    );

    // Dates invalides : 400 (fin <= début, format).
    for (start, end) in [
        ("2026-12-02T17:00:00.000Z", "2026-11-18T09:00:00.000Z"),
        ("2026-11-18T09:00:00.000Z", "2026-11-18T09:00:00.000Z"),
        ("18/11/2026", "2026-12-02T17:00:00.000Z"),
    ] {
        let bad = StartSprint {
            start_date: start.into(),
            end_date: end.into(),
            goal: None,
        };
        assert!(matches!(
            m.start_sprint(11, &bad).await,
            Err(Error::Api { status: 400, .. })
        ));
    }

    // La TUI déplace d'abord les issues non terminées vers le sprint suivant…
    m.move_to_sprint(11, &["DEMO-4".into()]).await.unwrap();
    let closed = m.close_sprint(10).await.unwrap();
    assert_eq!(closed.state, SprintState::Closed);
    // …la clôture ne déplace rien : le sprint clôturé garde son historique,
    assert_eq!(
        keys(&m.sprint_issues(10).await.unwrap()),
        vec!["DEMO-1", "DEMO-2", "DEMO-3", "DEMO-5"]
    );
    // et les issues non terminées restées dedans n'ont plus de sprint ouvert (donc au backlog).
    let backlog = m.backlog_issues(1).await.unwrap();
    assert!(keys(&backlog).contains(&"DEMO-5"));
    assert!(
        !keys(&backlog).contains(&"DEMO-1"),
        "terminée : hors backlog"
    );
    assert!(keys(&m.sprint_issues(11).await.unwrap()).contains(&"DEMO-4"));

    let started = m.start_sprint(11, &params).await.unwrap();
    assert_eq!(started.state, SprintState::Active);
}

#[tokio::test]
async fn issues_and_transitions() {
    let m = MockJira::new();
    let types = m.issue_types("DEMO").await.unwrap();
    let key = m
        .create_issue(&NewIssue {
            project_key: "DEMO".into(),
            issue_type_id: types[0].id.clone(),
            summary: "Nouvelle story".into(),
            description: None,
            story_points: Some(3.0),
            fix_version_ids: vec!["101".into()],
        })
        .await
        .unwrap();
    assert!(key.starts_with("DEMO-"));

    m.update_issue(
        &key,
        &IssueUpdate {
            story_points: Some(None),
            summary: Some("Renommée".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let issue = m
        .backlog_issues(1)
        .await
        .unwrap()
        .into_iter()
        .find(|i| i.key == key)
        .unwrap();
    assert_eq!(issue.summary, "Renommée");
    assert_eq!(issue.story_points, None);

    // « À faire » -> « Terminé » exige un écran de résolution : signalé et refusé (400).
    let tr = m.transitions(&key).await.unwrap();
    let done = tr
        .iter()
        .find(|t| t.to.category == StatusCategory::Done)
        .unwrap();
    assert!(done.has_screen);
    assert!(matches!(
        m.transition_issue(&key, &done.id).await,
        Err(Error::Api { status: 400, .. })
    ));

    let in_progress = tr
        .iter()
        .find(|t| t.to.category == StatusCategory::InProgress)
        .unwrap();
    assert!(!in_progress.has_screen);
    m.transition_issue(&key, &in_progress.id).await.unwrap();
    let tr = m.transitions(&key).await.unwrap();
    let done = tr
        .iter()
        .find(|t| t.to.category == StatusCategory::Done)
        .unwrap();
    assert!(!done.has_screen);
    m.transition_issue(&key, &done.id).await.unwrap();
    assert!(m.transition_issue(&key, &done.id).await.is_err());
    // Terminée : sort du backlog.
    assert!(!keys(&m.backlog_issues(1).await.unwrap()).contains(&key.as_str()));

    // fixVersion d'un autre projet refusée (création et mise à jour), sans effet de bord.
    let ops = NewIssue {
        project_key: "OPS".into(),
        issue_type_id: types[0].id.clone(),
        summary: "Hors projet".into(),
        fix_version_ids: vec!["101".into()],
        ..Default::default()
    };
    assert!(matches!(
        m.create_issue(&ops).await,
        Err(Error::Api { status: 400, .. })
    ));
    let upd = IssueUpdate {
        fix_version_ids: Some(vec!["101".into()]),
        ..Default::default()
    };
    assert!(matches!(
        m.update_issue("OPS-1", &upd).await,
        Err(Error::Api { status: 400, .. })
    ));
}

#[tokio::test]
async fn writes_validate_all_keys_first() {
    let m = MockJira::new();
    let before = keys(&m.backlog_issues(1).await.unwrap())
        .into_iter()
        .map(String::from)
        .collect::<Vec<_>>();
    let bad = vec!["DEMO-8".to_string(), "DEMO-999".to_string()];
    assert!(matches!(
        m.move_to_sprint(11, &bad).await,
        Err(Error::NotFound(_))
    ));
    assert!(matches!(
        m.rank(&bad, &RankPosition::Before("DEMO-12".into())).await,
        Err(Error::NotFound(_))
    ));
    assert!(matches!(
        m.rank(&["DEMO-8".into()], &RankPosition::After("DEMO-999".into()))
            .await,
        Err(Error::NotFound(_))
    ));
    assert_eq!(keys(&m.backlog_issues(1).await.unwrap()), before);

    let bad = vec!["DEMO-1".to_string(), "NOPE-1".to_string()];
    assert!(m.move_to_backlog(&bad).await.is_err());
    assert!(keys(&m.sprint_issues(10).await.unwrap()).contains(&"DEMO-1"));
}

#[tokio::test]
async fn create_and_update_sprint() {
    let m = MockJira::new();
    let s = m
        .create_sprint(1, "DEMO Sprint 6", Some("Objectif".into()))
        .await
        .unwrap();
    assert_eq!(s.state, SprintState::Future);
    assert_eq!(s.board_id, Some(1));
    assert!(m.create_sprint(99, "x", None).await.is_err());
    assert!(m.create_sprint(1, " ", None).await.is_err());

    let u = m
        .update_sprint(
            s.id,
            &SprintUpdate {
                name: Some("Renommé".into()),
                goal: Some(None),
                start_date: Some("2026-12-02T09:00:00.000Z".into()),
                end_date: Some("2026-12-16T17:00:00.000Z".into()),
            },
        )
        .await
        .unwrap();
    assert_eq!(u.name, "Renommé");
    assert_eq!(u.goal, None);
    assert_eq!(u.state, SprintState::Future);
    assert!(
        m.update_sprint(
            s.id,
            &SprintUpdate {
                end_date: Some("2026-11-01T09:00:00.000Z".into()),
                ..Default::default()
            }
        )
        .await
        .is_err()
    );
    let all = m.sprints(1, &[]).await.unwrap();
    assert!(all.iter().any(|x| x.id == s.id && x.name == "Renommé"));
}

#[tokio::test]
async fn versions_lifecycle() {
    let m = MockJira::new();
    let v = m
        .create_version(&NewVersion {
            project_id: "10000".into(),
            name: "3.0.0".into(),
            description: None,
            start_date: None,
            release_date: Some("2027-06-30".into()),
        })
        .await
        .unwrap();
    assert!(!v.released);
    assert!(
        m.create_version(&NewVersion {
            name: "3.0.0".into(),
            ..NewVersion {
                project_id: "10000".into(),
                name: String::new(),
                description: None,
                start_date: None,
                release_date: None
            }
        })
        .await
        .is_err()
    );

    let released = m
        .update_version(
            &v.id,
            &VersionUpdate {
                released: Some(true),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(released.released);
    assert_eq!(m.versions("DEMO").await.unwrap().len(), 4);
    assert_eq!(m.version_issues("DEMO", "101").await.unwrap().len(), 5);

    // Release de 1.1.0 avec report des issues non résolues sur 2.0.0 (DEMO-1 est terminée).
    let r = m
        .update_version(
            "101",
            &VersionUpdate {
                released: Some(true),
                archived: Some(false),
                start_date: Some(Some("2026-11-01".into())),
                move_unfixed_issues_to: Some("102".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(r.start_date.as_deref(), Some("2026-11-01"));
    assert_eq!(
        keys(&m.version_issues("DEMO", "101").await.unwrap()),
        vec!["DEMO-1"]
    );
    assert!(keys(&m.version_issues("DEMO", "102").await.unwrap()).contains(&"DEMO-5"));

    // Suppression avec report vers 2.0.0 ; version inconnue / identique refusées.
    assert!(m.delete_version("101", Some("101".into())).await.is_err());
    assert!(m.delete_version("101", Some("999".into())).await.is_err());
    m.delete_version("101", Some("102".into())).await.unwrap();
    assert!(keys(&m.version_issues("DEMO", "102").await.unwrap()).contains(&"DEMO-1"));
    assert!(
        m.versions("DEMO")
            .await
            .unwrap()
            .iter()
            .all(|v| v.id != "101")
    );
    m.delete_version(&v.id, None).await.unwrap();
    assert!(matches!(
        m.delete_version(&v.id, None).await,
        Err(Error::NotFound(_))
    ));
}

#[tokio::test]
async fn update_closed_sprint_dates_rejected() {
    let m = MockJira::new();
    // Nom seul : accepté.
    let s = m
        .update_sprint(
            9,
            &SprintUpdate {
                name: Some("DEMO Sprint 3 (archive)".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(s.state, SprintState::Closed);
    // Date différente : ignorée par Jira, signalée par le client ; le nom est tout de même appliqué.
    let r = m
        .update_sprint(
            9,
            &SprintUpdate {
                name: Some("Renommé".into()),
                end_date: Some("2026-11-30T17:00:00.000Z".into()),
                ..Default::default()
            },
        )
        .await;
    assert!(matches!(r, Err(Error::Invalid(ref msg)) if msg.contains("sprint fermé")));
    let all = m.sprints(1, &[SprintState::Closed]).await.unwrap();
    let s9 = all.iter().find(|x| x.id == 9).unwrap();
    assert_eq!(s9.name, "Renommé");
    assert_eq!(s9.end_date.as_deref(), Some("2026-10-28T17:00:00.000Z"));
    // Même instant, autre décalage : pas d'erreur.
    m.update_sprint(
        9,
        &SprintUpdate {
            end_date: Some("2026-10-28T18:00:00+01:00".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
}
