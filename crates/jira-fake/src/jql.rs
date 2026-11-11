//! Sous-ensemble de JQL reconnu par le faux serveur (celui qu'émet `jira-rust`).

/// Requête JQL reconnue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Query {
    /// `sprint = {id} ORDER BY Rank ASC`
    Sprint(u64),
    /// `filter = {fid} AND (sprint is EMPTY OR <exclusion>) AND statusCategory != Done
    /// [AND issuetype not in subtaskIssueTypes()] ORDER BY Rank ASC`, `<exclusion>` valant
    /// `sprint not in openSprints()` (forme historique) ou
    /// `(sprint not in openSprints() AND sprint not in futureSprints())` (forme émise par jira-rust).
    ///
    /// Sémantique Jira Cloud simulée : `openSprints()` = sprints **actifs** seulement,
    /// `futureSprints()` = sprints futurs. La forme historique laisse donc dans le backlog les
    /// issues d'un sprint futur (doublon backlog / sprint futur).
    Backlog {
        filter: u64,
        exclude_future: bool,
        exclude_subtasks: bool,
    },
    /// `project = "{key}" AND fixVersion = {vid} ORDER BY Rank ASC`
    Version { project: String, version: u64 },
}

pub const SUPPORTED: &str = "sprint = {id} ORDER BY Rank ASC ; \
filter = {fid} AND (sprint is EMPTY OR (sprint not in openSprints() AND sprint not in futureSprints())) \
AND statusCategory != Done [AND issuetype not in subtaskIssueTypes()] ORDER BY Rank ASC \
(ou forme historique `sprint is EMPTY OR sprint not in openSprints()`) ; \
project = \"{key}\" AND fixVersion = {vid} ORDER BY Rank ASC";

/// Espaces normalisés autour des opérateurs, casse ignorée.
fn normalize(jql: &str) -> String {
    let spaced = jql
        .replace("!=", "\u{1}")
        .replace('=', " = ")
        .replace('\u{1}', " != ")
        .replace('(', " ( ")
        .replace(')', " ) ");
    spaced
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
        // Re-coller les appels de fonction : `opensprints ( )` -> `opensprints()`.
        .replace(" ( ) ", "() ")
        .replace(" ( )", "()")
}

pub fn parse(jql: &str) -> Option<Query> {
    let n = normalize(jql);
    let body = n
        .strip_suffix(" order by rank asc")
        .or_else(|| n.strip_suffix(" order by rank"))
        .unwrap_or(&n)
        .trim();

    if let Some(rest) = body.strip_prefix("sprint = ") {
        return rest.parse().ok().map(Query::Sprint);
    }
    if let Some(rest) = body.strip_prefix("filter = ") {
        let (fid, tail) = rest.split_once(' ')?;
        let fid: u64 = fid.parse().ok()?;
        const BACKLOG_OPEN: &str =
            "and ( sprint is empty or sprint not in opensprints() ) and statuscategory != done";
        const BACKLOG_OPEN_FUTURE: &str = "and ( sprint is empty or ( sprint not in opensprints() \
             and sprint not in futuresprints() ) ) and statuscategory != done";
        let (exclude_future, tail) = if let Some(t) = tail.strip_prefix(BACKLOG_OPEN_FUTURE) {
            (true, t.trim())
        } else {
            (false, tail.strip_prefix(BACKLOG_OPEN)?.trim())
        };
        let exclude_subtasks = match tail {
            "" => false,
            "and issuetype not in subtaskissuetypes()" => true,
            _ => return None,
        };
        return Some(Query::Backlog {
            filter: fid,
            exclude_future,
            exclude_subtasks,
        });
    }
    if let Some(rest) = body.strip_prefix("project = ") {
        let (project, version) = rest.split_once(" and fixversion = ")?;
        let project = project.trim().trim_matches('"').to_uppercase();
        if project.is_empty() || project.contains(' ') {
            return None;
        }
        return Some(Query::Version {
            project,
            version: version.trim().trim_matches('"').parse().ok()?,
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognised() {
        assert_eq!(
            parse("sprint = 10 ORDER BY Rank ASC"),
            Some(Query::Sprint(10))
        );
        assert_eq!(parse("sprint=10"), Some(Query::Sprint(10)));
        assert_eq!(
            parse(
                "filter = 10001 AND (sprint is EMPTY OR sprint not in openSprints()) \
                 AND statusCategory != Done ORDER BY Rank ASC"
            ),
            Some(Query::Backlog {
                filter: 10001,
                exclude_future: false,
                exclude_subtasks: false
            })
        );
        assert_eq!(
            parse(
                "filter = 10001 AND (sprint is EMPTY OR sprint not in openSprints()) \
                 AND statusCategory != Done AND issuetype not in subtaskIssueTypes() ORDER BY Rank ASC"
            ),
            Some(Query::Backlog {
                filter: 10001,
                exclude_future: false,
                exclude_subtasks: true
            })
        );
        // Forme émise par jira-rust (espaces compactés comme dans une saisie manuelle).
        assert_eq!(
            parse(
                "filter = 10001 AND (sprint is EMPTY OR (sprint not in openSprints() AND \
                 sprint not in futureSprints())) AND statusCategory != Done \
                 AND issuetype not in subtaskIssueTypes() ORDER BY Rank ASC"
            ),
            Some(Query::Backlog {
                filter: 10001,
                exclude_future: true,
                exclude_subtasks: true
            })
        );
        assert_eq!(
            parse(
                "filter=10001 AND (sprint is EMPTY OR(sprint not in openSprints()AND sprint not in \
                 futureSprints()))AND statusCategory!=Done"
            ),
            Some(Query::Backlog {
                filter: 10001,
                exclude_future: true,
                exclude_subtasks: false
            })
        );
        assert_eq!(
            parse("project = \"DEMO\" AND fixVersion = 101 ORDER BY Rank ASC"),
            Some(Query::Version {
                project: "DEMO".into(),
                version: 101
            })
        );
    }

    #[test]
    fn rejected() {
        assert_eq!(parse("project = DEMO"), None);
        assert_eq!(parse("sprint in openSprints()"), None);
        assert_eq!(parse("filter = 1 AND assignee = currentUser()"), None);
        // futureSprints() seul (sans openSprints()) : non reconnu.
        assert_eq!(
            parse(
                "filter = 1 AND (sprint is EMPTY OR sprint not in futureSprints()) \
                 AND statusCategory != Done"
            ),
            None
        );
        assert_eq!(parse(""), None);
    }
}
