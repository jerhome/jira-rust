//! Bout en bout `jira-client` (`JiraHttp`) -> jira-fake, sur 127.0.0.1 uniquement.

use std::collections::HashSet;

use jira_client::model::{IssueUpdate, SprintState};
use jira_client::{JiraApi, JiraHttp, SecretString};
use jira_fake::Options;

async fn start() -> JiraHttp {
    let listener = jira_fake::bind(0).await.unwrap();
    let addr = listener.local_addr().unwrap();
    assert!(addr.ip().is_loopback());
    tokio::spawn(jira_fake::serve(
        listener,
        Options {
            quiet: true,
            ..Options::default()
        },
        std::future::pending(),
    ));
    JiraHttp::new(
        &format!("http://{addr}"),
        "alice@example.com",
        SecretString::new("jeton-factice"),
    )
    .unwrap()
}

/// Aucune issue ne doit apparaître à la fois dans le backlog et dans un sprint actif ou futur.
/// Avec l'ancienne JQL (`sprint not in openSprints()` seul), DEMO-6 et DEMO-7 (sprint futur 11)
/// apparaissaient aussi dans le backlog.
#[tokio::test]
async fn backlog_and_open_sprints_are_disjoint() {
    let c = start().await;
    let boards = c.scrum_boards("DEMO").await.unwrap();
    assert_eq!(boards.len(), 1);
    assert_eq!(boards[0].board_type.as_deref(), Some("scrum"));
    let board = boards[0].id;

    let backlog: HashSet<String> = c
        .backlog_issues(board)
        .await
        .unwrap()
        .into_iter()
        .map(|i| i.key)
        .collect();
    let sprints = c
        .sprints(board, &[SprintState::Active, SprintState::Future])
        .await
        .unwrap();
    assert!(sprints.iter().any(|s| s.state == SprintState::Future));
    let mut in_sprints = HashSet::new();
    for s in &sprints {
        for i in c.sprint_issues(s.id).await.unwrap() {
            assert_eq!(i.sprint_id, Some(s.id), "{}", i.key);
            in_sprints.insert(i.key);
        }
    }
    assert!(in_sprints.contains("DEMO-6") && in_sprints.contains("DEMO-7"));
    let dup: Vec<_> = backlog.intersection(&in_sprints).collect();
    assert!(dup.is_empty(), "issues en double backlog/sprint : {dup:?}");
    assert!(!backlog.is_empty());
}

/// Après une écriture, la recherche suivante porte `reconcileIssues` (accepté par jira-fake).
#[tokio::test]
async fn reconcile_issues_are_accepted_after_writes() {
    let c = start().await;
    let backlog = c.backlog_issues(1).await.unwrap();
    let first = backlog[0].key.clone();
    c.update_issue(
        &first,
        &IssueUpdate {
            story_points: Some(Some(21.0)),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let again = c.backlog_issues(1).await.unwrap();
    let updated = again.iter().find(|i| i.key == first).unwrap();
    assert_eq!(updated.story_points, Some(21.0));
}
