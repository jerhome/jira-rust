# Reference API Jira Cloud pour la TUI

Notes derivees uniquement de la doc publique (developer.atlassian.com : Jira Cloud
platform REST API v3, Jira Software Cloud REST API, page rate limiting, specs OpenAPI
publiees) + quelques precisions communautaires signalees comme telles. Aucune instance
reelle n'a ete interrogee. Consultation : 2026-11-11.

Conventions des fixtures : site fictif `https://jira.example.test`, projet `DEMO`
(id 10000), board `1`, sprints 1 (closed) / 2 (active) / 3 (future), versions
`0.9.0` (10099, released), `1.0.0` (10100), `1.1.0` (10101), utilisateurs
alice (`712020:00000000-0000-0000-0000-00000000a11c`) et bob
(`712020:00000000-0000-0000-0000-000000000b0b`). Champs custom fictifs :
Sprint `customfield_10020`, Story point estimate `customfield_10016`,
Story Points (legacy) `customfield_10028`, Rank `customfield_10019`.

## Index

| Domaine | Endpoint | Doc | Fixtures (`crates/jira-client/tests/fixtures/`) |
|---|---|---|---|
| auth | `GET /rest/api/3/myself` | [auth.md](auth.md) | `auth/myself.json` |
| auth | `GET /rest/api/3/field` | [auth.md](auth.md) | `auth/fields.json` |
| projects | `GET /rest/api/3/project/search` | [projects.md](projects.md) | `projects/project_search.json` |
| projects | `GET /rest/agile/1.0/board?type=scrum&projectKeyOrId=` | [projects.md](projects.md) | `projects/boards_scrum.json` |
| projects | `GET /rest/agile/1.0/board/{id}/configuration` (complement) | [projects.md](projects.md) | `projects/board_configuration.json` |
| backlog | `GET /rest/software/1.0/board/{id}/backlog` (enhanced) | [backlog.md](backlog.md) | `backlog/backlog_enhanced_page1.json`, `backlog/backlog_enhanced_last.json` |
| backlog | `GET /rest/agile/1.0/board/{id}/backlog` (**deprecie**) | [backlog.md](backlog.md) | `backlog/backlog_agile_deprecated.json` |
| backlog | `POST /rest/agile/1.0/backlog/issue` | [backlog.md](backlog.md) | `backlog/move_to_backlog_request.json` |
| backlog | `PUT /rest/agile/1.0/issue/rank` | [backlog.md](backlog.md) | `backlog/rank_request.json`, `backlog/rank_partial_207.json` |
| sprints | `GET /rest/agile/1.0/board/{id}/sprint?state=active,future` | [sprints.md](sprints.md) | `sprints/board_sprints_active_future.json` |
| sprints | `GET /rest/software/1.0/sprint/{id}/issue` (enhanced) | [sprints.md](sprints.md) | `sprints/sprint_issues_enhanced.json` |
| sprints | `GET /rest/agile/1.0/sprint/{id}/issue` (**deprecie**) | [sprints.md](sprints.md) | `sprints/sprint_issues_agile_deprecated.json` |
| sprints | `POST /rest/agile/1.0/sprint/{id}/issue` | [sprints.md](sprints.md) | `sprints/move_to_sprint_request.json` |
| sprints | `POST /rest/agile/1.0/sprint/{id}` (start/close) | [sprints.md](sprints.md) | `sprints/sprint_start_request.json`, `sprints/sprint_start_response.json`, `sprints/sprint_close_request.json`, `sprints/sprint_close_response.json` |
| issues | `POST /rest/api/3/search/jql` | [issues.md](issues.md) | `issues/search_jql_request.json`, `issues/search_jql_page1.json`, `issues/search_jql_last.json` |
| issues | `POST /rest/api/3/issue` | [issues.md](issues.md) | `issues/create_request.json`, `issues/create_response.json` |
| issues | `PUT /rest/api/3/issue/{key}` | [issues.md](issues.md) | `issues/edit_request.json` |
| issues | `GET/POST /rest/api/3/issue/{key}/transitions` | [issues.md](issues.md) | `issues/transitions.json`, `issues/transition_request.json` |
| issues | `GET /rest/api/3/issue/createmeta/{project}/issuetypes[/{id}]` | [issues.md](issues.md) | `issues/createmeta_issuetypes.json`, `issues/createmeta_fields_story.json` |
| issues | ADF minimal | [issues.md](issues.md) | `issues/adf_paragraph.json` |
| versions | `GET /rest/api/3/project/{key}/version` | [versions.md](versions.md) | `versions/project_versions_page.json`, `versions/project_versions_last.json` |
| versions | `POST /rest/api/3/version` | [versions.md](versions.md) | `versions/version_create_request.json`, `versions/version_created.json` |
| versions | `PUT /rest/api/3/version/{id}` (release) | [versions.md](versions.md) | `versions/version_release_request.json`, `versions/version_released.json` |
| errors | format, 401/403/404/429 | [errors.md](errors.md) | `errors/*.json` (`429_envelope.json` = enveloppe de test, pas un corps Jira) |

## Trois styles de pagination a gerer

| Style | Endpoints | Fin de pagination |
|---|---|---|
| Offset + `isLast` | `project/search`, `project/{k}/version`, `agile/board`, `agile/board/{id}/sprint` | `isLast == true` |
| Offset + `total` (sans `isLast`) | agile 1.0 listes d'issues (deprecies), createmeta (`issueTypes` / `fields`) | `startAt + n >= total` ou page vide |
| Token | `search/jql`, `/rest/software/1.0/...` | `isLast == true` ou `nextPageToken` absent |

## Points d'attention transverses

1. **Depreciation agile** : la lecture d'issues backlog/sprint via `/rest/agile/1.0`
   est deprecie ; utiliser `/rest/software/1.0/...` (token). `/rest/api/3/search`
   (offset) est supprime : seul `/rest/api/3/search/jql` existe.
2. **Custom fields** : ids variables par instance ; resoudre a l'execution (board
   configuration puis `/field`). Story point estimate = `jsw-story-points`, pas `float`.
3. **Deux representations d'issue** : v3 (ADF, liens `/rest/api/3`) vs agile/software
   (format v2 : `description` string wiki). Un seul modele Rust avec `description`
   en enum ou non demande cote agile.
4. **Types heterogenes** : ids d'issue/projet/version = strings en v3, ids de
   board/sprint et `location.projectId`/`projectId` de version = entiers.
5. **Coherence eventuelle** des recherches : `reconcileIssues` apres ecriture.
6. **`fields` par defaut de `search/jql` = `id` seulement.**
7. Lots de 50 max pour backlog/sprint/rank.
8. Ecritures sensibles aux permissions (Manage Sprints, Administer Projects) :
   gerer 403 proprement.

## Incertitudes et divergences relevees

- `com.pyxis.greenhopper.jira:jsw-story-points` n'est pas documente officiellement
  (source communautaire).
- RFC-127 : date de suppression des endpoints agile de lecture non publiee.
- Corps POST de `search/jql` : defauts supposes identiques au GET ; `expand` en string.
- 207 de `issue/rank` : valeur `status` des entrees reussies non documentee.
- Cloture de sprint via API : sort des issues non terminees non documente.
- `moveUnfixedIssuesTo` (release) : URI de version, detail non relu dans la spec.
- `total` absent/present selon les exemples pour `board/{id}/sprint`.
- Rate limiting : base de backoff 2 s (texte) vs 5 s (pseudo-code) ; retry sans
  `Retry-After` contradictoire.
- Corps des 401/429 non specifies (peuvent ne pas etre JSON).
- Jetons API scopes : base URL `api.atlassian.com/ex/jira/{cloudId}` a verifier.

## Sources principales

- https://developer.atlassian.com/cloud/jira/platform/rest/v3/intro/
- https://developer.atlassian.com/cloud/jira/software/rest/intro/
- Spec OpenAPI Software : https://dac-static.atlassian.com/cloud/jira/software/swagger.v3.json
- Spec OpenAPI Platform v3 : https://dac-static.atlassian.com/cloud/jira/platform/swagger-v3.v3.json
- https://developer.atlassian.com/cloud/jira/platform/rate-limiting/
- https://developer.atlassian.com/cloud/jira/platform/search-and-reconcile/
- https://developer.atlassian.com/cloud/jira/platform/apis/document/structure/
- RFC-127 (communautaire) : https://community.developer.atlassian.com/t/rfc-127-jira-software-cloud-api-changes-improving-scalability-for-software-space-issue-results/98871
