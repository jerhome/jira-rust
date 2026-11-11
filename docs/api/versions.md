# Versions (fixVersions)

Objet `Version` : `id` (**string**), `self`, `name`, `description?`, `archived`,
`released`, `startDate?` / `releaseDate?` (format `YYYY-MM-DD`), `userStartDate?` /
`userReleaseDate?` (format localise d'affichage, ex. `01/Dec/26` : **ne pas parser**),
`overdue?` (seulement si non released), `projectId` (**int**),
`issuesStatusForFixVersion?` (avec `expand=issuesstatus`).
Champ d'ecriture seule : `moveUnfixedIssuesTo` (URI d'une autre version).

---

## GET /rest/api/3/project/{projectIdOrKey}/version (pagine)

(Ne pas confondre avec `GET /rest/api/3/project/{projectIdOrKey}/versions`, non pagine.)

| Param query | Type | Defaut | Notes |
|---|---|---|---|
| `startAt` | int64 | 0 | |
| `maxResults` | int32 | 50 | |
| `orderBy` | string | | `description`, `name`, `releaseDate`, `sequence`, `startDate`, prefixe `+`/`-` ; versions sans date en dernier |
| `query` | string | | filtre insensible a la casse sur `name`/`description` |
| `status` | string | | CSV de `released`, `unreleased`, `archived` |
| `expand` | string | | CSV de `issuesstatus`, `operations`, `driver`, `approvers` |

- **200** : `PageBeanVersion` : `self`, `nextPage?`, `maxResults`, `startAt`, `total`,
  `isLast`, `values[Version]`.
- **404** : projet absent ou non visible. **401**.
- Permission : *Browse Projects* (accessible anonymement si le projet l'est).
- Pagination : offset, boucle tant que `isLast == false`.
- Vue TUI "versions" : `status=unreleased&orderBy=sequence` (ordre manuel du projet),
  ou `-releaseDate`.

Fixtures : `crates/jira-client/tests/fixtures/versions/project_versions_page.json`,
`crates/jira-client/tests/fixtures/versions/project_versions_last.json`

Source : https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-project-versions/#api-rest-api-3-project-projectidorkey-version-get

---

## POST /rest/api/3/version (creation)

Corps :
```json
{ "name": "1.2.0", "projectId": 10000, "description": "...",
  "startDate": "2027-01-16", "releaseDate": "2027-02-28",
  "released": false, "archived": false }
```
- Requis : `name` + `projectId` (int). (`project` = cle projet existe mais est marque
  deprecie au profit de `projectId` - **a verifier**, le fragment de spec lu ne
  precisait pas les champs requis.)
- **201** : `Version` cree.
- **400** (nom deja utilise dans le projet, dates invalides...), **401**, **404**
  (projet introuvable ou permission manquante), **422** (limite de versions atteinte).
- Permission : *Administer Jira* (globale) **ou** *Administer Projects* sur le projet.
  -> Un utilisateur "simple" ne pourra pas creer/publier de version : griser l'action
  si 403/404 et afficher le message.

Fixtures : `crates/jira-client/tests/fixtures/versions/version_create_request.json`,
`crates/jira-client/tests/fixtures/versions/version_created.json`

Source : https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-project-versions/#api-rest-api-3-version-post

---

## PUT /rest/api/3/version/{id} (edition / release)

Mise a jour partielle en pratique : n'envoyer que les champs a modifier.

Publier (release) :
```json
{ "released": true, "releaseDate": "2026-12-01",
  "moveUnfixedIssuesTo": "https://<site>/rest/api/3/version/10101" }
```
- `released: true` + `releaseDate` (sinon la date existante est conservee ; si aucune,
  Jira peut laisser vide - **non documente**, toujours envoyer la date du jour).
- `moveUnfixedIssuesTo` : **URI `self` d'une version** (pas un id) vers laquelle
  deplacer les issues non resolues. Champ "write-only". Optionnel : sans lui, les
  issues non terminees restent attachees a la version publiee.
  **Incertitude** : le fragment de spec lu ne detaillait pas ce champ ; description
  issue du schema `Version` connu. L'alternative documentee pour deplacer les issues
  est de les editer (`update.fixVersions`) avant la release.
- Depublier : `{ "released": false }`. Archiver : `{ "archived": true }`.
- **200** : `Version` mise a jour. **400** (requete invalide ou permission
  insuffisante, selon la doc), **401**, **404** (version introuvable).
- Permission : *Administer Jira* ou *Administer Projects*.
- Pour la TUI, avant release : compter les issues non `done` de la version via
  `search/jql` (`fixVersion = 10100 AND statusCategory != Done`) et proposer une
  version cible.

Fixtures : `crates/jira-client/tests/fixtures/versions/version_release_request.json`,
`crates/jira-client/tests/fixtures/versions/version_released.json`

Source : https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-project-versions/#api-rest-api-3-version-id-put

Hors perimetre mais utile plus tard : `DELETE /rest/api/3/version/{id}` est deprecie au
profit de `POST /rest/api/3/version/{id}/removeAndSwap`.

## Ajouts phase 2b

Sources lues (2026-11-11) : spec OpenAPI Jira Cloud platform v3
(https://developer.atlassian.com/cloud/jira/platform/swagger-v3.v3.json : schemas
`Version`, `DeleteAndReplaceVersionBean`, `CustomFieldReplacement` ; operations
`updateVersion`, `deleteAndReplaceVersion`, `deleteVersion`).

### Schema `Version` : precisions (lecture / ecriture)

Citations de la spec (descriptions des proprietes) :
- `startDate` (string, `format: date`) : "Expressed in ISO 8601 format (yyyy-mm-dd).
  Optional when creating or updating a version."
- `releaseDate` (string, `format: date`) : idem.
- `archived` (bool) : "Optional when creating or updating a version."
- `released` (bool) : "If the version is released a request to release again is
  ignored. Not applicable when creating a version. Optional when updating a version."
  -> **Correction de la section creation ci-dessus** : `released` n'est pas applicable
  a la creation (ne pas l'envoyer dans `POST /version`).
- `moveUnfixedIssuesTo` (string, `format: uri`) : "The URL of the self link to the
  version to which all unfixed issues are moved when a version is released. Not
  applicable when creating a version. Optional when updating a version."
  -> **Leve l'incertitude** de la section PUT : c'est bien l'URL `self` de la version
  cible (pas un id).
- `name` : "Optional when updating a version. The maximum length is 255 characters."
- `description` : "The maximum size is 16,384 bytes."
- `projectId` (int64) : "Not applicable when updating a version." (ne pas l'envoyer
  en PUT).
- `id`, `self`, `overdue` : `readOnly`.

### PUT /rest/api/3/version/{id} : startDate, archived, released, moveUnfixedIssuesTo

Requete exacte :
```
PUT /rest/api/3/version/10101
Content-Type: application/json
```
```json
{ "name": "1.1.0", "description": "Correctifs et versions",
  "startDate": "2026-12-01", "releaseDate": "2026-12-15",
  "released": true, "archived": false,
  "moveUnfixedIssuesTo": "https://jira.example.test/rest/api/3/version/10102" }
```
Pas de parametre query.

Format exact de `moveUnfixedIssuesTo` :
- La spec dit "The URL of the self link" : la valeur canonique est donc **le champ
  `self` de la version cible tel que renvoye par Jira** (ex.
  `https://jira.example.test/rest/api/3/version/10102`).
- L'exemple officiel de PUT montre un `self` de la forme
  `https://your-domain.atlassian.net/rest/api/~ver~/version/10000` : le segment de
  version d'API (`2` ou `3`) n'est pas fige par la doc ; non documente si Jira accepte
  les deux. Jira extrait vraisemblablement l'id en fin d'URL, mais **non documente**.
- **Recommandation** : reutiliser tel quel le `self` lu dans la liste des versions
  (`GET /project/{key}/version`) plutot que de reconstruire l'URL. Le contrat actuel
  reconstruit l'URL a partir de l'URL de base du client
  (`<base>/rest/api/3/version/<id>`) : identique au `self` tant que la base est
  l'URL du site ; **divergent si la base est la passerelle
  `https://api.atlassian.com/ex/jira/{cloudId}`** (OAuth 2.0 / jetons a portee), cas
  ou l'acceptation par Jira n'est pas documentee.
- Semantique : applique "when a version is released". Envoyer avec `released: true`
  dans la **meme** requete, sur une version non encore publiee. Si la version est deja
  publiee, "a request to release again is ignored" : le deplacement n'aura
  probablement pas lieu (**non documente** explicitement). Le champ n'apparait pas dans
  la reponse (ecriture seule).
- Contrainte cible (meme projet, differente de la version modifiee) : ecrite pour
  removeAndSwap, **pas** pour `moveUnfixedIssuesTo` ; la verifier quand meme cote
  client (le contrat le fait pour "differente").

`startDate` / `releaseDate` :
- Format `yyyy-mm-dd` uniquement (pas de datetime). Ne pas envoyer `userStartDate` /
  `userReleaseDate` (format localise d'affichage).
- Vider une date (`"startDate": null`) : **non documente**. Le contrat envoie `null`
  pour `Some(None)` : plausible mais **a verifier** (risque : ignore silencieusement).
- Contrainte `startDate <= releaseDate` : **non documentee** ; valider cote client, et
  s'attendre a un 400 sinon (corps non specifie).

`archived` : `{ "archived": true }` suffit ; une version archivee reste
`released`/`unreleased` comme avant. Interdiction de modifier une version archivee :
**non documentee**.

Reponses (spec) :
- **200** "Returned if the request is successful." -> `Version` complete.
- **400** "Returned if: the request is invalid. the user does not have the required
  permissions." (**permission insuffisante = 400 ici, pas 403**).
- **401** "Returned if the authentication credentials are incorrect."
- **404** "Returned if the version is not found."

Permission : *Administer Jira* (globale) ou *Administer Projects* sur le projet de la
version. Scope OAuth : `manage:jira-project` (granulaires :
`write:project-version:jira`, `read:project-version:jira`).

Fixtures :
- `crates/jira-client/tests/fixtures/versions/p2b_version_update_request.json` (tous les champs utiles)
- `crates/jira-client/tests/fixtures/versions/p2b_version_update_response.json` (200)
- `crates/jira-client/tests/fixtures/versions/p2b_version_archive_request.json`
- `crates/jira-client/tests/fixtures/versions/p2b_version_archived_response.json` (200)
- `crates/jira-client/tests/fixtures/versions/p2b_version_404.json` (404, message representatif)

Source : https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-project-versions/#api-rest-api-3-version-id-put

### POST /rest/api/3/version/{id}/removeAndSwap (suppression avec reaffectation)

Requete exacte :
```
POST /rest/api/3/version/10100/removeAndSwap
Content-Type: application/json
```
Pas de parametre query. Corps `DeleteAndReplaceVersionBean`
(`additionalProperties: false`, **aucun champ requis**) :

| Champ | Type | Description (spec) |
|---|---|---|
| `moveFixIssuesTo` | int64 | "The ID of the version to update `fixVersion` to when the field contains the deleted version." |
| `moveAffectedIssuesTo` | int64 | "The ID of the version to update `affectedVersion` to when the field contains the deleted version." |
| `customFieldReplacementList` | `CustomFieldReplacement[]` | "An array of custom field IDs (`customFieldId`) and version IDs (`moveTo`) to update when the fields contain the deleted version." |

`CustomFieldReplacement` : `{ "customFieldId": int64, "moveTo": int64 }`
(`customFieldId` = id **numerique** du champ picker de version, sans prefixe
`customfield_`).

```json
{ "moveFixIssuesTo": 10101, "moveAffectedIssuesTo": 10101,
  "customFieldReplacementList": [ { "customFieldId": 10050, "moveTo": 10101 } ] }
```
- **Ids numeriques (int64) dans le corps**, alors que `Version.id` est une string :
  convertir (le contrat le fait via `numeric_id`).
- Corps vide `{}` accepte par le schema : "If alternatives are not provided,
  occurrences of `fixVersion`, `affectedVersion`, and any version picker custom field,
  that contain the deleted version, are cleared."
  **Attention** : sans `moveAffectedIssuesTo`, les *Affects versions* sont aussi
  retirees (le contrat n'expose que `moveFixIssuesTo` : comportement conforme a sa doc
  "la reference est simplement retiree", mais cela touche aussi affectedVersion et les
  pickers custom).
- "Any replacement version must be in the same project as the version being deleted
  and cannot be the version being deleted."
- Pas d'exemple de requete dans la spec officielle (fixtures construites depuis le
  schema).

Reponses (spec) :
- **204** "Returned if the version is deleted." (corps vide).
- **400** "Returned if the request is invalid." (cible d'un autre projet, cible = version
  supprimee... : corps exact non documente).
- **401** "Returned if the authentication credentials are incorrect or missing."
- **404** "Returned if: the version to delete is not found. the user does not have the
  required permissions." (**permission insuffisante = 404 ici**).
- **Asynchronisme** : la spec documente un 204 synchrone ; pas de tache asynchrone
  (contrairement a d'autres suppressions Jira qui renvoient 303 + tache).

Permission : *Administer Jira* ou *Administer Projects*. Scope : `manage:jira-project`
(granulaires : `delete:project-version:jira`, `write:project-version:jira`).

### DELETE /rest/api/3/version/{id} (DEPRECIE)

`deprecated: true` dans la spec : "Deprecated, use Delete and replace version that
supports swapping version values in custom fields, in addition to the swapping for
`fixVersion` and `affectedVersion` provided in this resource."
Parametres **query** (string) `moveFixIssuesTo`, `moveAffectedIssuesTo` (meme
semantique, pas de remplacement des pickers custom). Reponses 204, 400, 401, 404.
**Ne pas utiliser** ; aucune date de retrait trouvee.

Fixtures :
- `crates/jira-client/tests/fixtures/versions/p2b_version_remove_and_swap_request.json` (complet)
- `crates/jira-client/tests/fixtures/versions/p2b_version_remove_and_swap_fix_only_request.json` (= corps du contrat avec cible)
- `crates/jira-client/tests/fixtures/versions/p2b_version_remove_and_swap_empty_request.json` (`{}` = contrat sans cible)
- `crates/jira-client/tests/fixtures/versions/p2b_version_remove_and_swap_400.json` (400, message representatif)
- `crates/jira-client/tests/fixtures/versions/p2b_version_404.json` (404, message representatif)

Sources :
https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-project-versions/#api-rest-api-3-version-id-removeandswap-post ,
https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-project-versions/#api-rest-api-3-version-id-delete
