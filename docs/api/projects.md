# Projets et boards

## GET /rest/api/3/project/search

Liste paginee (offset) des projets visibles.

| Param query | Type | Defaut | Notes |
|---|---|---|---|
| `startAt` | int64 | 0 | |
| `maxResults` | int32 | 50 | |
| `query` | string | | filtre insensible a la casse sur key/name |
| `typeKey` | string | | `software` pour ne garder que les projets Jira Software |
| `orderBy` | string | `key` | `name`, `key`, `-lastIssueUpdatedTime`, ... |
| `keys` | string[] | | filtre par cles (max 50) |
| `action` | string | `view` | `view`, `browse`, `edit`, `create` |
| `expand` | string | | `description,lead,issueTypes,url,projectKeys,permissions,insight` |

- **200** : `PageBeanProject` : `self`, `nextPage?`, `maxResults`, `startAt`, `total`,
  `isLast`, `values[]`. Projet : `id` (**string**), `key`, `name`, `projectTypeKey`,
  `simplified` (true = team-managed), `style` (`classic`/`next-gen`), `avatarUrls`.
- Erreurs documentees : 400 (param invalide), 401, 404. **Incertitude** : la doc
  associe 404 a "aucun projet trouve" ; en pratique attendre plutot 200 avec
  `values: []`. Traiter les deux comme "liste vide".
- Permission : *Browse Projects* sur chaque projet renvoye (sinon simplement absent).
- Pagination : continuer tant que `isLast == false` (ou suivre `nextPage`).
- **Piege** : `simplified: true` = projet team-managed. Les champs custom y sont
  specifiques au projet (le champ story points peut differer). Cible du projet :
  company-managed ; afficher un avertissement sinon.

Fixture : `crates/jira-client/tests/fixtures/projects/project_search.json`

Source : https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-projects/#api-rest-api-3-project-search-get

---

## GET /rest/agile/1.0/board

Exemple : `GET /rest/agile/1.0/board?type=scrum&projectKeyOrId=DEMO`

| Param query | Type | Defaut | Notes |
|---|---|---|---|
| `startAt` | int64 | 0 | |
| `maxResults` | int32 | 50 | |
| `type` | string | | `scrum`, `kanban`, `simple` ; plusieurs valeurs separees par virgule |
| `name` | string | | filtre partiel |
| `projectKeyOrId` | string | | boards dont la localisation/le filtre concerne ce projet |
| `orderBy` | enum | | `name`, `-name`, `+name` |
| `expand` | string | | `admins`, `permissions` |
| autres | | | `accountIdLocation`, `projectLocation`, `includePrivate`, `negateLocationFiltering`, `projectTypeLocation`, `filterId` |

- **200** : `{ maxResults, startAt, total, isLast, values[] }` ; board :
  `id` (int64), `self`, `name`, `type`, `location { projectId (int), projectKey,
  projectName, displayName, projectTypeKey, avatarURI, name }`.
- **400** (param invalide), **401**, **403**.
- Permission : seuls les boards que l'utilisateur peut voir (filtre partage).
- Scopes OAuth de reference : `read:board-scope:jira-software`, `read:project:jira`.
- **Divergence de types** : `location.projectId` est un **entier** ici, alors que
  `project.id` est une **string** dans l'API platform v3. Normaliser cote client.
- `location` peut etre absent (board base sur un filtre, "user location") -> `Option`.

Fixture : `crates/jira-client/tests/fixtures/projects/boards_scrum.json`

Source : https://developer.atlassian.com/cloud/jira/software/rest/api-group-board/#api-rest-agile-1-0-board-get

---

## GET /rest/agile/1.0/board/{boardId}/configuration (complement recommande)

Pas demande explicitement mais c'est la source la plus fiable pour :
- `estimation.field.fieldId` : le champ d'estimation du board (story points) ;
  `estimation.type` vaut `field` ou `none` ;
- `ranking.rankCustomFieldId` : id numerique du champ Rank (a passer en
  `rankCustomFieldId` lors des rangs) ;
- `columnConfig.columns[].statuses[].id` : mapping colonnes -> statuts ;
- `filter.id` : filtre JQL du board.

Codes : 200, 401, 403, 404. Permission : voir le board.

**Incertitude** : la presence de `estimation`/`ranking` n'est assuree que pour les
boards scrum ; fixture construite d'apres la structure de l'exemple officiel, allegee.

Fixture : `crates/jira-client/tests/fixtures/projects/board_configuration.json`

Source : https://developer.atlassian.com/cloud/jira/software/rest/api-group-board/#api-rest-agile-1-0-board-boardid-configuration-get
