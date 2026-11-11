# Backlog et rang

## ATTENTION : endpoint de lecture du backlog deprecie

D'apres la spec OpenAPI Jira Software Cloud (lue en 2026-11) :

- `GET /rest/agile/1.0/board/{boardId}/backlog` est marque **`deprecated: true`**
  (pagination offset `startAt`/`total`).
- Remplacant : `GET /rest/software/1.0/board/{boardId}/backlog` "Get issues for
  backlog (enhanced)", pagination par **`nextPageToken`/`isLast`**, sans `total`.
- Contexte : RFC-127 (community.developer.atlassian.com, fev. 2026) annoncait cette
  migration pour 8 endpoints agile de lecture d'issues avec une periode de
  depreciation cible de **6 mois** puis suppression. **Aucune date de suppression
  officielle trouvee dans le changelog** : a surveiller.

=> Implementer **directement l'endpoint enhanced** ; garder l'ancien uniquement en
repli (si 404 sur `/rest/software/1.0`).

---

## GET /rest/software/1.0/board/{boardId}/backlog (a utiliser)

Issues du backlog = issues du filtre du board non assignees a un sprint futur ou actif,
triees par rang par defaut.

| Param query | Type | Defaut | Notes |
|---|---|---|---|
| `nextPageToken` | string | absent (1re page) | token renvoye par la page precedente |
| `maxResults` | int32 | non precise par la doc | l'API peut renvoyer moins ; plafond 5000 issues |
| `jql` | string | | filtre additionnel ; un `ORDER BY` remplace l'ordre par rang ; `accountId` uniquement |
| `validateQuery` | bool | true | |
| `fields` | string[] | tous les champs navigables + champs Software | **toujours restreindre** |
| `expand` | string | | |
| `reconcileIssues` | int64[] | | max 50 ids, coherence lecture-apres-ecriture |

- **200** : `SoftwareIssueResults` : `issues[]`, `nextPageToken?` (absent/null sur la
  derniere page), `isLast`, `expand?`, `names?`, `schema?`, `warningMessages?`.
  **Pas de `total`** : pour un compteur,
  `GET /rest/software/1.0/board/{boardId}/backlog/approximate-count` (param `jql`
  optionnel) -> `{ "count": 153 }` (approximatif, eventuellement en retard).
- Codes : 200, 400, 401, 403, 404.
- Permission : voir le board ; seules les issues visibles sont renvoyees.
- Champs "Software" ajoutes dans `fields` : `sprint` (sprint actif/futur, objet ou null),
  `closedSprints[]`, `flagged`, `epic`.
- **Coherence eventuelle** : apres un rang/deplacement, la relecture peut ne pas
  refleter l'ecriture -> passer les ids touches dans `reconcileIssues`.
- **Piege representation** : les endpoints agile renvoient des issues au format
  "v2" (liens `self` en `/rest/api/2/...`, `description` en **string wiki**, pas en
  ADF). Ne pas demander `description` ici ; la lire via l'API v3.
  (Constate sur les exemples officiels agile 1.0 ; non explicite pour
  `/rest/software/1.0` : **a verifier**.)
- Format du parametre `fields` en query : liste separee par virgules
  (`fields=summary,status,...`) ; la spec le type "array of objects", formulation
  ambigue : **a verifier**, la forme CSV est la forme usuelle des endpoints agile.

Fixtures :
- `crates/jira-client/tests/fixtures/backlog/backlog_enhanced_page1.json` (avec `nextPageToken`)
- `crates/jira-client/tests/fixtures/backlog/backlog_enhanced_last.json` (`isLast: true`, sans token)

Source : https://developer.atlassian.com/cloud/jira/software/rest/api-group-board/#api-rest-software-1-0-board-boardid-backlog-get

## GET /rest/agile/1.0/board/{boardId}/backlog (deprecie, repli)

Params : `startAt` (int64), `maxResults` (int32, defaut 50), `jql`, `validateQuery`
(defaut true), `fields` (array), `expand` ("currently not used").
Reponse : `{ expand, startAt, maxResults, total, issues[] }` (**pas de `isLast`** :
arreter quand `startAt + issues.len() >= total` ou `issues` vide). Codes : 200, 400,
401, 403, 404.

Fixture : `crates/jira-client/tests/fixtures/backlog/backlog_agile_deprecated.json`

Source : https://developer.atlassian.com/cloud/jira/software/rest/api-group-board/#api-rest-agile-1-0-board-boardid-backlog-get

---

## POST /rest/agile/1.0/backlog/issue

Deplace des issues vers le backlog (equivaut a retirer leurs sprints futurs et actifs).

Corps (`additionalProperties: false`) :
```json
{ "issues": ["DEMO-2", "DEMO-3"] }
```
- `issues` : string[] (cles ou ids), **requis**, valeurs uniques, **max 50** par
  appel (decouper au-dela).
- **204** sans corps. **400** (requete invalide), **401**, **403**, **404**.
- Permission : *Schedule Issues* (et en pratique *Edit Issues*) sur les projets
  concernes ; scope de reference `write:board-scope:jira-software`.
- Ne permet pas de choisir la position : enchainer avec `PUT /rest/agile/1.0/issue/rank`.

Fixture requete : `crates/jira-client/tests/fixtures/backlog/move_to_backlog_request.json`

Source : https://developer.atlassian.com/cloud/jira/software/rest/api-group-backlog/#api-rest-agile-1-0-backlog-issue-post

---

## PUT /rest/agile/1.0/issue/rank

Place des issues avant OU apres une issue de reference.

Corps (`IssueRankRequestBean`) :
```json
{ "issues": ["DEMO-5"], "rankBeforeIssue": "DEMO-4", "rankCustomFieldId": 10019 }
```
- `issues` : string[], **max 50**.
- `rankBeforeIssue` **ou** `rankAfterIssue` (string) : en fournir un seul.
- `rankCustomFieldId` (int64, optionnel) : id numerique du champ Rank (sans prefixe
  `customfield_`) ; defaut = champ Rank par defaut. Le prendre dans la config du board.
- Reponses :
  - **204** : tout a reussi, corps vide.
  - **207** : au moins une issue a echoue ; corps `PartialSuccessBean` :
    `{ "entries": [ { "issueId": int64, "issueKey": string, "status": int32, "errors": [string]? } ] }`.
    Toujours parser 207 et remonter les entrees en echec.
  - **400**, **401**, **403**.
- Permission : *Schedule Issues*.
- **Incertitudes** : (1) la valeur de `status` pour une entree reussie n'est pas
  documentee (traiter tout `2xx` comme succes) ; (2) le comportement si l'ancre fait
  partie de `issues` n'est pas documente : l'eviter cote client.
- La valeur de Rank (`customfield_10019`, string LexoRank) est opaque : le tri
  lexicographique de la chaine suffit ; ne jamais la calculer cote client.

Fixtures : `crates/jira-client/tests/fixtures/backlog/rank_request.json`,
`crates/jira-client/tests/fixtures/backlog/rank_partial_207.json`

Source : https://developer.atlassian.com/cloud/jira/software/rest/api-group-issue/#api-rest-agile-1-0-issue-rank-put

## Ajouts phase 2b

### `PUT /rest/agile/1.0/issue/rank` : forme exacte du 207 (spec OpenAPI)

Source : https://developer.atlassian.com/cloud/jira/software/swagger.v3.json
(operation `rankIssues`, schemas `PartialSuccessBean` et `Entry`), relue 2026-11-11.

Description (citations) : "Moves (ranks) issues before or after a given issue. At
most 50 issues may be ranked at once." ; en cas d'echec partiel, "the 207 status code
is returned for the whole response" avec le detail par issue ; sans
`rankCustomFieldId`, le champ Rank par defaut est utilise.

Schemas (tous deux `additionalProperties: false`, **aucune liste `required`**) :
```
PartialSuccessBean { entries: Entry[] }
Entry { issueId: int64, issueKey: string, status: int32, errors: string[] }
```
- Toutes les proprietes sont optionnelles au sens du schema : deserialiser
  `issueId`, `issueKey`, `status` en `Option`, `errors` en `#[serde(default)]`
  (le client le fait deja).
- `status` est un **code HTTP par entree** : l'exemple officiel montre `200` pour les
  issues rangees et `503` pour une issue en echec.
- `errors` est **absent** sur les entrees reussies dans l'exemple officiel (et non `[]`).
- L'exemple officiel liste **toutes** les issues de la requete (reussies et en echec),
  pas seulement les echecs.

Exemple officiel (cles fictivisees ; message conserve) :
```json
{ "entries": [
  { "issueId": 10000, "issueKey": "PR-1", "status": 200 },
  { "issueId": 10001, "issueKey": "PR-2", "status": 200 },
  { "issueId": 10002, "issueKey": "PR-3", "status": 503,
    "errors": ["JIRA Agile cannot execute the rank operation at this time. Please try again later."] } ] }
```
Reponses : 204 "Empty response is returned if operation was successful." ; 207
"Returns the list of issue with status of rank operation." ; 400 "Returned if the
request is invalid." ; 401 "Returned if the user is not logged in." ; 403 "Returned
if the user does not have a valid license or does not have permission to rank."
(**pas de 404** documente : une cle d'issue inconnue apparait probablement en entree
207 ou en 400, **non documente**).

Consequences client :
- Une entree `status: 503` est **transitoire** : relancer le rang pour ces seules
  issues apres un delai est raisonnable (la doc dit "Please try again later") ; les
  autres codes (`403`, `400`, `404`) sont definitifs.
- 207 avec toutes les entrees en `2xx` : non documente mais a tolerer (= succes) ;
  le client actuel le fait (filtre `200..300`).

Fixtures :
- `crates/jira-client/tests/fixtures/backlog/p2b_rank_partial_207_official_shape.json` (forme de l'exemple officiel : 2 x 200 sans `errors`, 1 x 503 avec `errors`)
- `crates/jira-client/tests/fixtures/backlog/p2b_rank_207_all_ok.json` (cas defensif non documente : 207 sans echec, une entree avec `errors: []`)

Source : https://developer.atlassian.com/cloud/jira/software/rest/api-group-issue/#api-rest-agile-1-0-issue-rank-put
