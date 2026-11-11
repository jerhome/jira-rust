# Issues (API platform v3)

## Forme d'une issue (v3)

```
{ id: string, key: string, self, expand?, fields: { ... } }
```
Champs utiles dans `fields` (tous `Option` : absents si non demandes, `null` si vides) :

| Champ | Forme | Notes |
|---|---|---|
| `summary` | string | |
| `description` | ADF (objet) ou null | **v3 = ADF**, v2/agile = string wiki |
| `status` | `{ id, name, statusCategory: { id: int, key, colorName, name } }` | `statusCategory.key` : `new`, `indeterminate`, `done` (`undefined` rare) - a utiliser pour la logique, jamais `status.name` (localise/personnalisable) |
| `issuetype` | `{ id, name, subtask, hierarchyLevel, iconUrl }` | `hierarchyLevel` : 1 = epic, 0 = standard, -1 = sous-tache |
| `priority` | `{ id, name, iconUrl }` ou null | |
| `assignee` | `User` ou null | `accountId`, `displayName`, `emailAddress?` |
| `fixVersions` | `[{ id, name, released, archived, releaseDate? }]` | |
| `customfield_<sprint>` | `null` ou `[{ id, name, state, boardId, goal, startDate?, endDate?, completeDate? }]` | historique : sprints fermes + actif/futur |
| `customfield_<points>` | `null` ou number | float meme si entier (`3.0`) |
| `customfield_<rank>` | string LexoRank | ex. `"0|i0000f:"` |

Ids des custom fields : resolus a l'execution (voir `auth.md`).

---

## POST /rest/api/3/search/jql (recherche JQL "enhanced")

Remplace `GET/POST /rest/api/3/search`, **supprime** (CHANGE-2046 ; les appels
renvoient desormais une erreur indiquant que l'API a ete retiree). Ne plus utiliser
`startAt`/`total`.

Corps (`SearchAndReconcileRequestBean`) :
```json
{
  "jql": "project = DEMO AND sprint in openSprints() ORDER BY Rank ASC",
  "fields": ["summary", "status", "issuetype", "priority", "assignee", "fixVersions",
             "customfield_10020", "customfield_10016", "customfield_10019"],
  "maxResults": 50,
  "nextPageToken": "<token de la page precedente, absent pour la 1re>",
  "fieldsByKeys": false
}
```

| Champ | Type | Defaut | Notes |
|---|---|---|---|
| `jql` | string | | **doit etre borne** (restriction type `project = DEMO`) ; `ORDER BY` max 7 champs ; une JQL non bornee est rejetee (400) |
| `nextPageToken` | string | | absent sur la derniere page |
| `maxResults` | int32 | 50 | l'API peut renvoyer moins quand beaucoup de champs sont demandes ; plafond 5000 (atteint seulement avec `id`/`key`) |
| `fields` | string[] | **`id` seulement** | piege majeur : sans `fields`, les issues n'ont que `id` ; `*navigable`, `*all`, `-champ` acceptes |
| `expand` | string | | ex. `names,schema` (CSV) |
| `properties` | string[] | | max 5 |
| `fieldsByKeys` | bool | false | |
| `failFast` | bool | false | |
| `reconcileIssues` | int64[] | | max 50 ids ; coherence lecture-apres-ecriture |

**Incertitude** : le schema detaille du corps POST n'a pas pu etre lu dans la spec
(page rendue en JS) ; defauts ci-dessus confirmes pour la variante GET
(`fields` defaut `id`, `maxResults` defaut 50, max 5000, `reconcileIssues` max 50,
`includeArchivedProjects` defaut false), supposes identiques en POST. `expand` est une
string CSV dans le POST (a confirmer).

Reponse **200** (`SearchAndReconcileResults`) : `issues[]`, `nextPageToken?`, `isLast`,
`names?`, `schema?`. **Pas de `total`** ; compteur via
`POST /rest/api/3/search/approximate-count` (`{ "jql": ... }` -> `{ "count": n }`).

Pagination : boucle tant que `isLast == false` **et** `nextPageToken` present ;
renvoyer le meme `jql`/`fields` + le token. Des retours communautaires signalent que
`nextPageToken` peut **expirer** (400 "invalid or expired") : en cas de 400 sur une page
> 1, relancer depuis le debut plutot que d'echouer silencieusement.

Codes : 200, 400 (JQL invalide, champ inconnu, JQL non bornee, token invalide), 401.
Pas de 404 : une JQL valide sans resultat -> `issues: []`.
Permissions : *Browse Projects* (+ securite d'issue) ; issues non visibles filtrees.
Coherence **eventuelle** : une issue creee/modifiee juste avant peut manquer -> passer
son id dans `reconcileIssues`.

JQL utiles pour la TUI :
- sprint actif : `project = DEMO AND sprint in openSprints() ORDER BY Rank ASC`
- sprint donne : `sprint = 2 ORDER BY Rank ASC`
- par version : `project = DEMO AND fixVersion = 10100 ORDER BY Rank ASC`
- Utiliser `accountId` pour les utilisateurs (`assignee = "712020:..."` ou `currentUser()`).

Fixtures : `crates/jira-client/tests/fixtures/issues/search_jql_request.json`,
`crates/jira-client/tests/fixtures/issues/search_jql_page1.json` (token),
`crates/jira-client/tests/fixtures/issues/search_jql_last.json` (`isLast: true`)

Source : https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-issue-search/#api-rest-api-3-search-jql-post
Guide : https://developer.atlassian.com/cloud/jira/platform/search-and-reconcile/

---

## POST /rest/api/3/issue (creation)

Query : `updateHistory` (bool, defaut false).
Corps :
```json
{ "fields": {
    "project": { "key": "DEMO" },
    "issuetype": { "id": "10001" },
    "summary": "...",
    "description": { "type": "doc", "version": 1, "content": [ ... ] },
    "assignee": { "accountId": "712020:..." },
    "priority": { "id": "3" },
    "fixVersions": [{ "id": "10100" }],
    "customfield_10016": 3,
    "customfield_10020": 2
} }
```
- `fields` et/ou `update` (operations) ; un meme champ ne peut pas etre dans les deux.
- Sprint a la creation : valeur = **id numerique du sprint** (pas un objet).
  Alternative plus robuste : creer, puis `POST /rest/agile/1.0/sprint/{id}/issue`.
- **201** : `{ id, key, self }` (+ `transition?` si une transition a ete demandee).
- **400** : champ requis manquant, valeur invalide, ou **champ absent de l'ecran de
  creation** ("Field 'customfield_10016' cannot be set. It is not on the appropriate
  screen, or unknown.") -> verifier via createmeta avant d'envoyer ; n'envoyer que les
  champs presents dans createmeta.
- **401**, **403** (permission manquante), **422** (limite depassee, documente sur les
  operations de creation recentes : a verifier).
- Permission : *Browse Projects* + *Create Issues*.

Fixtures : `crates/jira-client/tests/fixtures/issues/create_request.json`,
`crates/jira-client/tests/fixtures/issues/create_response.json`

Source : https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-issues/#api-rest-api-3-issue-post

---

## PUT /rest/api/3/issue/{issueIdOrKey} (edition)

Query : `notifyUsers` (defaut true ; false exige des droits admin),
`overrideScreenSecurity` (defaut false, admin/app), `overrideEditableFlag` (defaut
false, admin/app), `returnIssue` (defaut false), `expand`.

Corps :
```json
{ "fields": { "summary": "...", "customfield_10016": 5 },
  "update": { "fixVersions": [ { "set": [ { "id": "10100" } ] } ] } }
```
- `fields` = remplacement ; `update` = operations (`set`, `add`, `remove`, `edit`)
  utiles pour les tableaux (`fixVersions`, `labels`).
- Desassigner : `"assignee": null`. Vider les points : `"customfield_10016": null`.
- **204** sans corps (ou **200** + issue si `returnIssue=true`).
- **400** (champ non editable / hors ecran d'edition / valeur invalide), **401**,
  **403** (pas *Edit Issues* ou options override sans droit), **404**, **409** (conflit
  d'edition concurrente), **422**.
- Permission : *Browse Projects* + *Edit Issues*.
- **Ne pas** modifier le statut ici (passer par les transitions), ni le rang (agile
  `/issue/rank`). Le Sprint peut techniquement s'editer via le champ custom (id
  numerique) mais preferer `POST /rest/agile/1.0/sprint/{id}/issue`.
- Champs editables : `GET /rest/api/3/issue/{key}/editmeta` (non couvert ici).

Fixture : `crates/jira-client/tests/fixtures/issues/edit_request.json`

Source : https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-issues/#api-rest-api-3-issue-issueidorkey-put

---

## GET /rest/api/3/issue/{issueIdOrKey}/transitions

Query : `expand` (`transitions.fields` pour connaitre les champs d'ecran),
`transitionId`, `skipRemoteOnlyCondition`, `includeUnavailableTransitions` (defaut
false), `sortByOpsBarAndStatus` (defaut false).

- **200** : `{ expand, transitions: [ { id (string), name, to: Status (avec
  statusCategory), hasScreen, isGlobal, isInitial, isAvailable, isConditional,
  isLooped, fields? } ] }`.
- Seules les transitions accessibles a l'utilisateur dans l'etat courant sont renvoyees.
- **401**, **404** (issue absente ou non visible).
- Permission : *Browse Projects* ; liste vide sans *Transition Issues*.

Fixture : `crates/jira-client/tests/fixtures/issues/transitions.json`

## POST /rest/api/3/issue/{issueIdOrKey}/transitions

Corps minimal : `{ "transition": { "id": "21" } }` ; optionnels `fields`, `update`
(champs de l'ecran de transition, ex. resolution), `historyMetadata`, `properties`.
- **204** sans corps.
- **400** (transition invalide pour l'etat courant, champ d'ecran requis manquant ex.
  `resolution` quand `hasScreen: true`), **401**, **404**, **409** (conflit, a
  verifier), **422** (limite atteinte, a verifier).
- Permission : *Browse Projects* + *Transition Issues*.
- **Piege** : les ids de transition dependent du workflow et de l'etat courant :
  toujours les relire (GET) juste avant ; ne jamais les mettre en cache entre issues.

Fixture : `crates/jira-client/tests/fixtures/issues/transition_request.json`

Source : https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-issues/#api-rest-api-3-issue-issueidorkey-transitions-get

---

## GET /rest/api/3/issue/createmeta/{projectIdOrKey}/issuetypes

(L'ancien `GET /rest/api/3/issue/createmeta` global est deprecie au profit de ces deux
endpoints pagines.)

Query : `startAt` (defaut 0), `maxResults` (defaut 50, **max 200**).
- **200** : `{ maxResults, startAt, total, issueTypes: [ { id, name, subtask,
  hierarchyLevel, description, iconUrl, untranslatedName? } ] }`.
  **Divergence** : la cle est `issueTypes` (pas `values` comme les autres PageBean).
- Codes : 200, 400, 401.
- Permission : *Create Issues* dans le projet.

Fixture : `crates/jira-client/tests/fixtures/issues/createmeta_issuetypes.json`

### GET /rest/api/3/issue/createmeta/{projectIdOrKey}/issuetypes/{issueTypeId}

Champs de l'ecran de creation pour un type. Memes params de pagination.
- **200** : `{ maxResults, startAt, total, fields: [ { fieldId, key, name, required,
  hasDefaultValue, defaultValue?, operations[], schema, allowedValues?,
  autoCompleteUrl? } ] }`. Cle `fields` ici.
- Utiliser `required` pour valider le formulaire et `schema.custom` pour reperer
  story points / sprint presents sur l'ecran.

Fixture : `crates/jira-client/tests/fixtures/issues/createmeta_fields_story.json`

Source : https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-issues/#api-rest-api-3-issue-createmeta-projectidorkey-issuetypes-get

---

## ADF (Atlassian Document Format) minimal

En v3, `description` (et tout champ texte riche : commentaire, champ "paragraph")
**doit** etre un document ADF ; une string brute -> 400.

```json
{ "type": "doc", "version": 1,
  "content": [ { "type": "paragraph",
                 "content": [ { "type": "text", "text": "Une ligne." } ] } ] }
```
Regles :
- racine `type: "doc"`, `version: 1`, `content` = blocs (`paragraph`, `heading`,
  `bulletList`, `codeBlock`, ...).
- noeud `text` : `text` **non vide** (une chaine vide est invalide) ; pour une ligne
  vide, utiliser un `paragraph` sans `content`.
- Conversion TUI -> ADF : un paragraphe par bloc separe par ligne vide ; `hardBreak`
  pour les retours simples. Lecture ADF -> texte : concatener recursivement les noeuds
  `text`, saut de ligne entre blocs ; ignorer les noeuds inconnus (mentions, medias)
  en affichant un placeholder.
- Description vide : envoyer `null` plutot qu'un doc vide.

Fixture : `crates/jira-client/tests/fixtures/issues/adf_paragraph.json`

Source : https://developer.atlassian.com/cloud/jira/platform/apis/document/structure/

## Ajouts phase 2b

Sources lues (2026-11-11) : spec OpenAPI Jira Cloud platform v3
(https://developer.atlassian.com/cloud/jira/platform/swagger-v3.v3.json : operations
`getTransitions`, `doTransition` ; schemas `IssueTransition`, `FieldMetadata`,
`JsonTypeBean`).

### GET /rest/api/3/issue/{issueIdOrKey}/transitions?expand=transitions.fields

Requete exacte :
```
GET /rest/api/3/issue/DEMO-5/transitions?expand=transitions.fields
Accept: application/json
```
`expand=transitions.fields` : "returns information about the fields in the transition
screen for each transition. Fields hidden from the screen are not returned."
(formulation de la spec, resumee). Sans cet expand, `fields` est absent.

Schema `IssueTransition` (cles exactes de la spec, ordre alphabetique) : `expand`,
`fields`, `hasScreen`, `id`, `isAvailable`, `isConditional`, `isGlobal`, `isInitial`,
`looped`, `name`, `to`. Tous `readOnly` sauf `id`.
- `hasScreen` : "Whether there is a screen associated with the issue transition."
- `fields` : "Details of the fields associated with the issue transition screen. Use
  this information to populate `fields` and `update` in a transition request."
  -> **map** `id de champ -> FieldMetadata` (objet, pas tableau ; peut etre `{}`).
- **Divergence** : la spec nomme la propriete `looped`, la fixture existante
  `transitions.json` (et les reponses couramment observees) utilisent `isLooped`.
  Ne pas deserialiser ce champ (inutile a la TUI) ou accepter les deux
  (`#[serde(alias)]`).

`FieldMetadata` (valeur de chaque entree de `fields`) ; requis par la spec : `key`,
`name`, `operations`, `required`, `schema` :

| Cle | Type | Notes |
|---|---|---|
| `required` | bool | "Whether the field is required." |
| `schema` | `JsonTypeBean` | `{ type, items?, system?, custom?, customId?, configuration? }` ; `system` pour un champ systeme (ex. `"resolution"`), `custom` + `customId` pour un champ custom |
| `name` | string | libelle (localise) |
| `key` | string | id du champ (ex. `resolution`, `customfield_10016`) |
| `operations` | string[] | ex. `["set"]`, `["set","add"]` |
| `allowedValues` | array | optionnel ; objets dont la forme depend du type (resolution : `{ self, id, name, description? }`) |
| `hasDefaultValue` | bool | optionnel |
| `defaultValue` | any | optionnel |
| `autoCompleteUrl` | string | optionnel (utilisateurs, labels...) |
| `configuration` | object | optionnel |

**Detection d'une transition "a ecran"** (rien de plus n'est documente) :
1. `hasScreen == true` : un ecran est associe. Il peut ne contenir que des champs
   optionnels (transition applicable sans `fields`), ou des champs requis.
2. `fields[*].required == true` : au moins un champ doit etre fourni dans le POST.
   L'exemple officiel montre des `fields` avec `hasScreen: false` : ne pas se fier
   au seul `hasScreen`. Le contrat (`has_screen = hasScreen || any(required)`) est
   la bonne approximation.
3. **Angle mort** : les *validateurs* de workflow (ex. "Field Required Validator",
   conditions de sous-taches) ne sont **pas** exposes par cet endpoint ; une transition
   `hasScreen: false` sans champ requis peut quand meme echouer en 400. Toujours
   afficher le 400 de la transition.
4. `isAvailable == false` n'apparait qu'avec `includeUnavailableTransitions=true` ;
   `isConditional` indique seulement la presence de conditions.

Remarques sur l'exemple officiel : il utilise des `statusCategory.key` `in-flight` /
`completed` qui ne correspondent pas aux cles reelles (`new`, `indeterminate`, `done`)
et un champ `summary` de type multiselect : **exemple non representatif**, s'en tenir
au schema. Codes : 200, 401, 404.

Fixture : `crates/jira-client/tests/fixtures/issues/p2b_transitions_with_fields.json`
(transition sans ecran avec `fields: {}`, transition "Done" avec `resolution` requise
+ story points optionnels, transition "Review" a ecran sans champ requis).

### POST /rest/api/3/issue/{issueIdOrKey}/transitions avec champs d'ecran

```
POST /rest/api/3/issue/DEMO-5/transitions
Content-Type: application/json
```
```json
{ "transition": { "id": "31" }, "fields": { "resolution": { "id": "10000" } } }
```
- Valeur d'un champ = forme d'ecriture standard (resolution : `{ "id": ... }` ou
  `{ "name": "Done" }` ; l'exemple officiel utilise `{"name":"Fixed"}`). Preferer
  `id` pris dans `allowedValues`.
- N'envoyer que des champs presents dans `fields` de la transition.
- **204** "Returned if the request is successful."
- **400**, liste officielle : "no transition is specified", "the user does not have
  permission to transition the issue", "a field that isn't included on the transition
  screen is defined in `fields` or `update`", "a field is specified in both `fields`
  and `update`", "the request is invalid for any other reasons". **Permission de
  transition manquante = 400**, pas 403.
- **401**, **404** (issue absente ou non visible), **409** "conflicting update",
  **413** (limite par issue depassee : commentaires, worklogs, pieces jointes, liens),
  **422** "configuration problem".

Corps du 400 "champ requis manquant" : **non documente**. Forme attendue
`ErrorCollection` avec la cle du champ dans `errors` (fixture representative :
`{"errors":{"resolution":"Resolution is required."}}` ; le texte n'est pas garanti).
Cas voisin, documente dans les reponses de creation/edition : champ absent de l'ecran
-> `errors.<champ>` = "Field '<champ>' cannot be set. It is not on the appropriate
screen, or unknown.". Transition invalide pour l'etat courant -> message dans
`errorMessages` (texte representatif). Logique client : uniquement sur le code 400 +
presence de cles dans `errors`, jamais sur le texte.

Fixtures :
- `crates/jira-client/tests/fixtures/issues/p2b_transition_request_with_resolution.json`
- `crates/jira-client/tests/fixtures/issues/p2b_transition_400_required_field.json` (representatif)
- `crates/jira-client/tests/fixtures/issues/p2b_transition_400_not_on_screen.json` (representatif)
- `crates/jira-client/tests/fixtures/issues/p2b_transition_400_invalid_transition.json` (representatif)

Sources :
https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-issues/#api-rest-api-3-issue-issueidorkey-transitions-get ,
https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-issues/#api-rest-api-3-issue-issueidorkey-transitions-post
