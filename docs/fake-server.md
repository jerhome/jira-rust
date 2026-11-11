# jira-fake : faux serveur Jira Cloud local

`crates/jira-fake` fournit le binaire `jira-fake` : un Jira Cloud simulé **en mémoire**
qui écoute **uniquement sur 127.0.0.1**. Il sert à la phase 3 (intégration) :
lancer le vrai `jira-rust` (client HTTP `JiraHttp`, pas `--mock`) contre un serveur
qui parle le format HTTP de Jira Cloud (REST v3 + Agile 1.0), sans jamais toucher
une instance réelle.

Les formats suivent `docs/api/*.md` et les fixtures de
`crates/jira-client/tests/fixtures/`. Le jeu de données reprend celui de `MockJira`
(projets DEMO/OPS, board scrum, sprints, versions, 13 issues) sans dépendre de la
crate `jira-client`. L'état est perdu à l'arrêt du serveur.

## Lancement

```sh
cargo run -p jira-fake -- --port 8089
```

Chaque requête est journalisée sur stderr (`méthode chemin -> statut (durée)`) ;
l'en-tête `Authorization` n'est jamais journalisé.

### Options

| Option | Défaut | Effet |
|---|---|---|
| `--port <N>` | `8089` | port d'écoute sur 127.0.0.1 (`0` = éphémère). Aucune option ne permet d'écouter ailleurs. |
| `--latency-ms <N>` | `0` | délai ajouté à chaque requête |
| `--fail-rate <F>` | `0.0` | fraction (0.0 à 1.0) des requêtes authentifiées qui renvoient **429** avec `Retry-After`, pour tester les retries |
| `--retry-after <S>` | `1` | valeur de `Retry-After` (secondes) des 429 |
| `--forbid-writes` | non | **403** sur toutes les écritures (POST/PUT/DELETE), sauf `POST /rest/api/3/search/jql` qui est une lecture |
| `-q`, `--quiet` | non | pas de log des requêtes |

Exemple pour éprouver la politique de retry du client :

```sh
cargo run -p jira-fake -- --port 8089 --fail-rate 0.3 --latency-ms 150
```

## Configurer jira-rust pour l'utiliser

Utiliser un **fichier de configuration séparé**, passé avec `--config`. Ne jamais
modifier `~/.config/jira-rust/config.toml`.

`/tmp/jira-fake.toml` (par exemple) :

```toml
base_url = "http://127.0.0.1:8089"   # http accepté uniquement en loopback
email = "alice@example.com"
```

Le token est fourni par l'environnement (n'importe quelle valeur : le faux serveur
exige seulement un en-tête `Authorization: Basic …` non vide) :

```sh
JIRA_API_TOKEN=factice cargo run -p jira-tui -- --config /tmp/jira-fake.toml doctor
JIRA_API_TOKEN=factice cargo run -p jira-tui -- --config /tmp/jira-fake.toml
```

(Si `api_token` est mis dans le fichier, celui-ci doit être en mode 600.)
Le garde-fou hors-ligne de `jira-client` (`JIRA_RUST_OFFLINE=1`) laisse passer
127.0.0.1 : aucune exception n'est nécessaire.

## Endpoints servis

| Méthode | Chemin | Notes |
|---|---|---|
| GET | `/rest/api/3/myself` | Alice Martin |
| GET | `/rest/api/3/field` | Sprint `customfield_10020` (`gh-sprint`), Story point estimate `customfield_10016` (`jsw-story-points`), Story Points legacy `customfield_10028`, Rank `customfield_10019`, leurre `customfield_10050` |
| GET | `/rest/api/3/project/search` | offset (`startAt`, `maxResults` ≤ 50, `isLast`, `nextPage`) ; `query`, `typeKey`, `keys` |
| GET | `/rest/agile/1.0/board` | `projectKeyOrId`, `type` ; boards 1 (DEMO, filtre 10001) et 2 (OPS, filtre 10002) |
| GET | `/rest/agile/1.0/board/{id}/configuration` | `filter.id`, `estimation.field.fieldId`, `ranking.rankCustomFieldId` |
| GET | `/rest/agile/1.0/board/{id}/sprint` | `state` CSV (valeur inconnue → 400), offset + `isLast` |
| POST | `/rest/agile/1.0/sprint` | création (201, état `future`) ; nom ≤ 30 caractères |
| GET / POST | `/rest/agile/1.0/sprint/{id}` | POST = mise à jour partielle ; champ inconnu → 400 ; `future→active` (dates requises, un seul sprint actif par board), `active→closed` (`completeDate` = maintenant) ; autre transition → 400 ; sprint clôturé : seuls `name`/`goal` |
| POST | `/rest/agile/1.0/sprint/{id}/issue` | ≤ 50 issues ; sprint clôturé → 400 ; `rankBeforeIssue`/`rankAfterIssue` optionnels ; 204 |
| POST | `/rest/agile/1.0/backlog/issue` | ≤ 50 ; retire le sprint ouvert (l'historique des sprints clôturés est conservé) ; 204 |
| PUT | `/rest/agile/1.0/issue/rank` | exactement un de `rankBeforeIssue`/`rankAfterIssue` ; 204, ou **207** `{entries:[…]}` si une issue est inconnue ou est l'ancre |
| POST (et GET) | `/rest/api/3/search/jql` | pagination `nextPageToken`/`isLast` (`maxResults` défaut 50, plafond 100) ; `fields` respecté (défaut : `id` seulement ; `*all`, `*navigable`, `-champ`) ; description en ADF ; jeton invalide → 400 |
| GET | `/rest/api/3/issue/createmeta/{projet}/issuetypes` | clé `issueTypes` (Story 10001, Bug 10002, Tâche 10003) |
| GET | `/rest/api/3/issue/createmeta/{projet}/issuetypes/{typeId}` | clé `fields` |
| POST | `/rest/api/3/issue` | 201 `{id,key,self}` ; erreurs par champ (`summary`, description non ADF, champ hors écran…) |
| GET / PUT | `/rest/api/3/issue/{clé}` | PUT : `fields` + `update.fixVersions` (`set`/`add`/`remove`) ; 204 (200 si `returnIssue=true`) |
| GET / POST | `/rest/api/3/issue/{clé}/transitions` | `expand=transitions.fields` ; ids `{statut}1` (11, 31, 41, 51) |
| GET | `/rest/api/3/project/{clé}/version` | offset + `isLast` ; `status`, `query`, `orderBy` (`name`, `description`, `releaseDate`, `startDate`, `sequence`, ±) |
| POST | `/rest/api/3/version` | 201 ; `projectId` (ou `project`) ; nom dupliqué → 400 `errors.name` |
| GET / PUT | `/rest/api/3/version/{id}` | PUT partiel ; `moveUnfixedIssuesTo` = URI `self` d'une version : déplace les issues non terminées |
| POST | `/rest/api/3/version/{id}/removeAndSwap` | `moveFixIssuesTo` optionnel ; 204 |

Comportements transverses :

- sans `Authorization: Basic …` → **401** au format Jira ;
- erreurs toujours au format `{ "errorMessages": [...], "errors": {...} }` (y compris
  404 sur un chemin non simulé) ;
- ids : issues/projets/versions en chaîne, boards/sprints/`projectId` de version en entier,
  comme Jira.

## Jeu de données

- Statuts : `1` À faire (new), `3` En cours, `4` En revue (indeterminate), `5` Terminé (done).
- Sprints du board 1 : `9` DEMO Sprint 3 (closed), `10` DEMO Sprint 4 (active),
  `11` DEMO Sprint 5 (future).
- Versions DEMO : `100` 1.0.0 (publiée), `101` 1.1.0, `102` 2.0.0.
- Issues DEMO-1 à DEMO-12 (DEMO-1 à 5 dans le sprint 10, DEMO-6/7 dans le sprint 11,
  DEMO-10 passée par le sprint 9 clôturé), OPS-1.
- Transition **« À faire » → « Terminé »** : `hasScreen: true` et champ `resolution`
  requis ; sans `fields.resolution` dans le POST → **400** (`errors.resolution`).
  Depuis « En cours »/« En revue », la transition vers « Terminé » passe sans écran.

## Limites

- **JQL** : seules ces formes sont reconnues (casse et espaces libres, `ORDER BY Rank ASC`
  facultatif) ; toute autre requête → **400** avec un `errorMessages` explicite :
  - `sprint = {id} ORDER BY Rank ASC`
  - `filter = {fid} AND (sprint is EMPTY OR sprint not in openSprints()) AND statusCategory != Done ORDER BY Rank ASC`
    (avec ou sans `AND issuetype not in subTaskIssueTypes()`, forme émise par `jira-rust`)
  - `project = "{clé}" AND fixVersion = {vid} ORDER BY Rank ASC`
- Pas de lecture via `/rest/agile/1.0/board/{id}/backlog`, `/rest/agile/1.0/sprint/{id}/issue`
  (GET) ni `/rest/software/1.0/...` : `jira-rust` lit les issues uniquement par `search/jql`.
- Pas de cohérence éventuelle simulée (`reconcileIssues` ignoré) ; les écritures sont
  visibles immédiatement.
- Authentification non vérifiée au-delà de la présence d'un `Basic` non vide ; pas de
  permissions fines (seul `--forbid-writes` simule un 403 global).
- Le rang est global (ordre d'un vecteur) ; la valeur `customfield_10019` est un
  pseudo-LexoRank dérivé de la position.
- La clôture d'un sprint ne déplace pas les issues : celles non terminées n'ont plus de
  sprint ouvert et réapparaissent donc dans la JQL du backlog (comportement Jira).
- Pas d'en-têtes `X-RateLimit-*` hors des 429 simulés.

## Tests

```sh
cargo test -p jira-fake
```

Les tests d'intégration (`crates/jira-fake/tests/api.rs`) démarrent le serveur sur un
port éphémère de 127.0.0.1 et vérifient chaque endpoint avec reqwest (clés de premier
niveau comparées aux fixtures, pagination, JQL non supportée, 401, 403, 429, 207,
transition refusée).
