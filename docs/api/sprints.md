# Sprints

Objet `SprintBean` (agile) : `id` (int64), `self`, `state` (`future` | `active` |
`closed`), `name`, `startDate?`, `endDate?`, `completeDate?`, `createdDate?`,
`originBoardId` (int64), `goal?`. Dates ISO 8601 (`2026-11-02T08:00:00.000Z`).
Un sprint `future` n'a en general ni `startDate` ni `endDate` -> `Option`.

**Divergence de forme** : la valeur du champ custom Sprint dans une issue
(`customfield_XXXXX`) est un objet different : `{id, name, state, boardId, goal,
startDate?, endDate?, completeDate?}` (`boardId` et pas `originBoardId`, pas de `self`).
Prevoir deux structs ou une struct tolerante.

---

## GET /rest/agile/1.0/board/{boardId}/sprint

Exemple : `GET /rest/agile/1.0/board/1/sprint?state=active,future`

| Param query | Type | Defaut | Notes |
|---|---|---|---|
| `startAt` | int64 | 0 | |
| `maxResults` | int32 | 50 | |
| `state` | string | tous | `future`, `active`, `closed`, separes par virgule |

- **200** : `{ maxResults, startAt, isLast, values[SprintBean] }`. **Ne pas compter
  sur `total`** (absent en pratique pour cette liste, present dans certains exemples) :
  paginer avec `isLast`.
- Codes : 200, 400 (board pas scrum / sprints non supportes), 401, 403, 404.
- Permission : voir le board.
- **Piege** : un board peut lister des sprints dont `originBoardId` != boardId
  (sprints partages entre boards dont les filtres se recoupent). Pour start/close,
  n'agir que si `originBoardId == boardId` ou avertir.
- Plusieurs sprints `active` simultanes possibles (option "parallel sprints").

Fixture : `crates/jira-client/tests/fixtures/sprints/board_sprints_active_future.json`

Source : https://developer.atlassian.com/cloud/jira/software/rest/api-group-board/#api-rest-agile-1-0-board-boardid-sprint-get

---

## Lecture des issues d'un sprint

### GET /rest/software/1.0/sprint/{sprintId}/issue (a utiliser)

Endpoint "enhanced", meme contrat que le backlog enhanced (voir `backlog.md`) :
`nextPageToken`, `maxResults` (plafond 5000), `reconcileIssues` (max 50),
`jql`, `validateQuery` (defaut true), `fields`, `expand`. Reponse
`{ issues[], nextPageToken?, isLast }`, tri par rang par defaut, champs Software
(`sprint`, `closedSprints`, `flagged`, `epic`) inclus.
Codes : 200, 400, 401, 403, 404. Scopes de reference : `read:sprint:jira-software`,
`read:issue-details:jira`, `read:jql:jira`.

Fixture : `crates/jira-client/tests/fixtures/sprints/sprint_issues_enhanced.json`

Source : https://developer.atlassian.com/cloud/jira/software/rest/api-group-sprint/#api-rest-software-1-0-sprint-sprintid-issue-get

### GET /rest/agile/1.0/sprint/{sprintId}/issue (DEPRECIE)

Marque `deprecated: true` dans la spec. Params `startAt`, `maxResults` (plafonne par
`jira.search.views.default.max` de l'instance), `jql`, `validateQuery`, `fields`,
`expand`. Reponse `{ expand, startAt, maxResults, total, issues[] }`.
Codes : 200, 401, 403, 404. A n'utiliser qu'en repli.

Fixture : `crates/jira-client/tests/fixtures/sprints/sprint_issues_agile_deprecated.json`

Alternative universelle : `POST /rest/api/3/search/jql` avec
`jql = "sprint = 2 ORDER BY Rank ASC"` (voir `issues.md`) -> representation v3 (ADF),
mais sans les champs Software `sprint`/`closedSprints`/`flagged` (utiliser le champ
custom Sprint).

---

## POST /rest/agile/1.0/sprint/{sprintId}/issue

Deplace des issues dans un sprint (`future` ou `active` uniquement) et les range.

Corps (`additionalProperties: false`) :
```json
{ "issues": ["DEMO-4", "DEMO-5"], "rankAfterIssue": "DEMO-3", "rankCustomFieldId": 10019 }
```
- `issues` : string[], **max 50**.
- `rankBeforeIssue` / `rankAfterIssue` (string, optionnels, un seul) ;
  `rankCustomFieldId` (int64, optionnel).
- **204** sans corps ; **400** (requete invalide, ex. sprint ferme), **401**, **403**
  (pas de licence ou pas la permission d'assigner au sprint), **404** (sprint absent
  ou non visible).
- Permission : *Schedule Issues* + *Edit Issues*. Scope : `write:sprint:jira-software`.

Fixture : `crates/jira-client/tests/fixtures/sprints/move_to_sprint_request.json`

Source : https://developer.atlassian.com/cloud/jira/software/rest/api-group-sprint/#api-rest-agile-1-0-sprint-sprintid-issue-post

---

## POST /rest/agile/1.0/sprint/{sprintId} (mise a jour partielle : start / close)

Les champs absents du corps ne sont **pas** modifies (contrairement a `PUT` qui met a
`null` tout champ omis : **ne jamais utiliser PUT pour start/close**).

Corps (`SprintBean`, `additionalProperties: false`) : `name`, `goal`, `state`,
`startDate`, `endDate`, + `id`, `self`, `originBoardId`, `createdDate`,
`completeDate` (ces derniers ne servent pas ; `completeDate` ne peut pas etre fixe).
Ne pas envoyer de champ inconnu (rejet probable a cause de `additionalProperties: false`).

Regles (doc officielle) :
- **Demarrer** : `state: "active"`. Le sprint doit etre `future` **et** avoir
  `startDate` et `endDate` : les envoyer dans la meme requete s'ils ne sont pas deja
  renseignes (cas normal pour un sprint future).
  ```json
  { "state": "active", "startDate": "2026-11-16T08:00:00.000Z", "endDate": "2026-11-30T17:00:00.000Z", "goal": "..." }
  ```
- **Cloturer** : `state: "closed"`. Le sprint doit etre `active` ; `completeDate` est
  fixe a l'heure de la requete.
  ```json
  { "state": "closed" }
  ```
- Toute autre transition d'etat est refusee (pas de reouverture, pas de
  future -> closed).
- Sprint ferme : seuls `name` et `goal` sont modifiables ; le reste est ignore.

Reponses : **200** (sprint mis a jour, `SprintBean`), **400** (requete invalide, ex.
regle ci-dessus violee), **401**, **403** (pas de licence / pas la permission
*Manage Sprints*), **404**.

Permission : *Manage Sprints* sur tous les projets du filtre du board. Scope :
`write:sprint:jira-software`.

**Points non documentes (a traiter prudemment)** :
- Sort des issues non terminees a la cloture via l'API : la doc ne dit rien (l'UI
  propose de les deplacer). Recommandation : avant `closed`, deplacer explicitement les
  issues non `done` vers le sprint suivant (`POST /sprint/{next}/issue`) ou le backlog
  (`POST /backlog/issue`), puis cloturer.
- Fuseau horaire des dates : envoyer en UTC avec suffixe `Z`.
- La doc de *Create sprint* precise que, quand on demarre un sprint depuis l'UI,
  l'`endDate` fixee via l'API est ignoree ; sans incidence pour la TUI.

Fixtures : `crates/jira-client/tests/fixtures/sprints/sprint_start_request.json`,
`crates/jira-client/tests/fixtures/sprints/sprint_start_response.json`,
`crates/jira-client/tests/fixtures/sprints/sprint_close_request.json`,
`crates/jira-client/tests/fixtures/sprints/sprint_close_response.json`

Source : https://developer.atlassian.com/cloud/jira/software/rest/api-group-sprint/#api-rest-agile-1-0-sprint-sprintid-post

## Ajouts phase 2b

Sources lues (2026-11-11) : spec OpenAPI Jira Software Cloud
(https://developer.atlassian.com/cloud/jira/software/swagger.v3.json, operations
`createSprint`, `partiallyUpdateSprint`, `updateSprint`) et page
https://developer.atlassian.com/cloud/jira/software/rest/api-group-sprint/ .

### POST /rest/agile/1.0/sprint (creation d'un sprint futur)

Requete exacte :
```
POST /rest/agile/1.0/sprint
Content-Type: application/json
Accept: application/json
```
Pas de parametre query. Corps `SprintCreateBean` (`additionalProperties: false`) :

| Champ | Type | Requis | Notes |
|---|---|---|---|
| `name` | string | **oui** (selon la description) | "The name is trimmed" : espaces de tete/fin retires par Jira |
| `originBoardId` | int64 | **oui** (selon la description) | board Scrum qui possede le sprint |
| `goal` | string | non | |
| `startDate` | string ISO 8601 | non | |
| `endDate` | string ISO 8601 | non | |

```json
{ "name": "DEMO Sprint 4", "originBoardId": 1, "goal": "Stabiliser le backlog",
  "startDate": "2026-11-30T08:00:00.000Z", "endDate": "2026-12-14T17:00:00.000Z" }
```
- Le schema ne porte **aucune liste `required`** : l'obligation de `name` et
  `originBoardId` n'est que dans le texte ("Sprint name and origin board id are
  required"). Valider cote client (le contrat le fait deja pour `name`).
- Le sprint cree est toujours `state: "future"` ; `state` n'est pas un champ du
  bean de creation (`additionalProperties: false` -> ne pas l'envoyer).
- La doc indique que, si le sprint est ensuite demarre **depuis l'UI**, l'`endDate`
  fixee par cet appel est ignoree (duree du sprint precedent utilisee).

Reponses :
- **201** "Created sprint" : `SprintBean` (`id`, `self`, `state: "future"`, `name`,
  `startDate?`, `endDate?`, `originBoardId`, `goal?`, `createdDate?`).
  **Code 201, pas 200** : ne pas tester `== 200`.
- **400** "Returned if the request is invalid." (nom manquant, dates invalides, board
  non Scrum : corps exact non documente).
- **401** "Returned if the user is not logged in."
- **403** "Returned if the user does not have a valid license." (en pratique aussi
  permission *Manage Sprints* manquante : non ecrit dans la spec).
- **404** "Returned if the board does not exist or the user does not have permission
  to view it."

Permission : *Manage Sprints* (meme regle que les autres ecritures de sprint ; la
spec de cette operation ne la repete pas). Scope OAuth : `write:sprint:jira-software`.

**Ecarts / incertitudes relevees dans la doc** :
- L'exemple officiel de reponse a `"id": 37` mais `"self": ".../sprint/23"` :
  incoherence de l'exemple ; ne jamais deriver l'id de `self`.
- Les exemples utilisent des dates avec decalage (`2015-04-11T15:22:00.000+10:00`) ;
  Jira peut renvoyer un decalage different de celui envoye (normalisation dans le
  fuseau du serveur/utilisateur : **non documente**). Parser en `DateTime<FixedOffset>`
  et comparer en UTC, ne pas comparer les chaines.
- `createdDate` absent de l'exemple officiel mais present dans le schema -> `Option`.
- Contrainte `endDate > startDate` : **non documentee** pour la creation. La fixture
  `p2b_sprint_400_invalid.json` est un corps representatif (message invente au format
  `ErrorCollection`), pas une chaine garantie.

Fixtures :
- `crates/jira-client/tests/fixtures/sprints/p2b_sprint_create_request.json` (complet)
- `crates/jira-client/tests/fixtures/sprints/p2b_sprint_create_minimal_request.json` (name + originBoardId, = ce qu'envoie le contrat sans goal)
- `crates/jira-client/tests/fixtures/sprints/p2b_sprint_create_response.json` (201)
- `crates/jira-client/tests/fixtures/sprints/p2b_sprint_create_minimal_response.json` (201, sans dates ni goal)
- `crates/jira-client/tests/fixtures/sprints/p2b_sprint_create_404_board.json` (404, message representatif)
- `crates/jira-client/tests/fixtures/sprints/p2b_sprint_400_invalid.json` (400, representatif)

Source : https://developer.atlassian.com/cloud/jira/software/rest/api-group-sprint/#api-rest-agile-1-0-sprint-post

### POST /rest/agile/1.0/sprint/{sprintId} : mise a jour partielle (nom, objectif, dates)

Meme endpoint que start/close (section precedente), utilise ici **sans `state`**.

Requete exacte :
```
POST /rest/agile/1.0/sprint/4
Content-Type: application/json
```
```json
{ "name": "DEMO Sprint 4 - Release 1.1", "goal": "Livrer la 1.1",
  "startDate": "2026-12-01T08:00:00.000Z", "endDate": "2026-12-15T17:00:00.000Z" }
```
Description officielle (citations) :
- "Performs a partial update of a sprint. A partial update means that fields not
  present in the request JSON will not be updated."
- "For closed sprints, only the name and goal can be updated; changes to other fields
  will be ignored."
- "Other changes to state are not allowed." / "The completeDate field cannot be
  updated manually."
- Exemple officiel de requete : `{"name":"new name"}` (un seul champ : confirme la
  semantique partielle).

**Difference POST vs PUT** :
| | `POST /sprint/{id}` (partiallyUpdateSprint) | `PUT /sprint/{id}` (updateSprint) |
|---|---|---|
| champs absents | inchanges | "**will be set to null**" |
| corps | `SprintBean`, `additionalProperties: false` | idem |
| regles d'etat / sprint clos | identiques | identiques |
| reponses | 200, 400, 401, 403, 404 | 200, 400, 401, 403, 404 |

=> Toujours POST. Un PUT `{ "name": "x" }` effacerait `goal`, `startDate`, `endDate`.

Sprint par etat :
- `future` : `name`, `goal`, `startDate`, `endDate` modifiables.
- `active` : idem (la doc ne restreint que les sprints clos). Modifier `startDate`
  d'un sprint actif est accepte par la doc ; effet sur les rapports non documente.
- `closed` : seuls `name`/`goal` sont appliques ; **les dates envoyees sont ignorees
  silencieusement (200, pas 400)**. Voir `p2b_sprint_update_closed_*` : `endDate`
  envoyee, inchangee dans la reponse. La TUI doit donc comparer la reponse a la
  demande ou desactiver l'edition des dates pour un sprint clos.
- Erreurs liees a l'etat : un `state` different de la transition autorisee
  (`future -> active`, `active -> closed`) -> 400. Sans `state` dans le corps, aucun
  400 d'etat n'est attendu.

Vider l'objectif : **non documente**. `goal: null` en POST partiel est ambigu (null
pourrait etre traite comme "absent" ou comme "vider"). Le client envoie `""` : choix
raisonnable mais **a verifier** sur une instance de test autorisee.

Contraintes de dates : la doc ne mentionne **aucune** contrainte (ordre start/end,
dates passees, format). Seule regle ecrite : un sprint ne peut etre demarre que s'il a
`startDate` et `endDate`. Valider cote client `endDate > startDate` ; traiter un 400
generique (`errorMessages` et/ou `errors.startDate`/`errors.endDate`, cles
**supposees**).

Reponses : **200** "Updated sprint" (`SprintBean` complet), **400** "Returned if the
request is invalid.", **401**, **403** "Returned if the user does not have a valid
license.", **404** "Returned if the sprint does not exist."

Fixtures :
- `crates/jira-client/tests/fixtures/sprints/p2b_sprint_update_request.json`
- `crates/jira-client/tests/fixtures/sprints/p2b_sprint_update_response.json` (200, sprint future)
- `crates/jira-client/tests/fixtures/sprints/p2b_sprint_update_closed_request.json` (name + goal + endDate sur sprint clos)
- `crates/jira-client/tests/fixtures/sprints/p2b_sprint_update_closed_response.json` (200, name/goal appliques, endDate inchangee)
- `crates/jira-client/tests/fixtures/sprints/p2b_sprint_400_invalid.json` (400 representatif)

Sources :
https://developer.atlassian.com/cloud/jira/software/rest/api-group-sprint/#api-rest-agile-1-0-sprint-sprintid-post ,
https://developer.atlassian.com/cloud/jira/software/rest/api-group-sprint/#api-rest-agile-1-0-sprint-sprintid-put
