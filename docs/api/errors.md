# Erreurs, rate limiting, retries

## Format d'erreur (`ErrorCollection`)

Commun a l'API platform v3 et a l'API agile 1.0 :
```json
{ "errorMessages": ["message global"], "errors": { "champ": "message du champ" } }
```
- `errorMessages` : string[] (peut etre vide).
- `errors` : map champ -> message (peut etre vide ; cles = id de champ, ex.
  `customfield_10016`, `summary`).
- Optionnels possibles : `warningMessages` (recherche JQL), `status` (int).
- Les deux cles peuvent etre absentes : deserialiser avec `#[serde(default)]`.
- **Le corps peut ne pas etre du JSON** (401 genere par la passerelle, 429, 5xx
  proxy, page HTML de maintenance) : en cas d'echec de parsing, conserver le texte brut
  tronque dans l'erreur. **Incertitude** : la fixture `401.json` reprend le format
  JSON usuel, mais le corps exact d'un 401 n'est pas specifie dans la doc.
- Endpoints a succes partiel : `PUT /rest/agile/1.0/issue/rank` renvoie **207** avec
  `{ entries: [...] }` (voir `backlog.md`) : ce n'est pas une erreur HTTP mais doit
  etre traite comme tel par entree.

## Codes

| Code | Signification | Action TUI |
|---|---|---|
| 400 | requete invalide (JQL, champ hors ecran, transition invalide, regle de sprint) | afficher `errorMessages` + `errors` par champ |
| 401 | non authentifie (jeton invalide/revoque, email errone) | arreter, inviter a reconfigurer ; ne pas retenter |
| 403 | authentifie mais permission/licence manquante (ex. Manage Sprints, Administer Projects) | message clair, griser l'action |
| 404 | ressource absente **ou non visible** (Jira masque l'existence) | message "introuvable ou pas d'acces" |
| 409 | conflit d'edition (PUT issue) | relire puis retenter une fois |
| 422 | limite metier atteinte (ex. nb de versions) | afficher |
| 429 | rate limit | retry avec backoff (ci-dessous) |
| 5xx | transitoire (503 peut porter `Retry-After`) | retry idempotent avec backoff |

Source : https://developer.atlassian.com/cloud/jira/platform/rest/v3/intro/#status-codes

## 429 et Retry-After

En-tetes (doc rate limiting Jira Cloud) :
- `Retry-After` : **secondes** (entier) ; uniquement sur 429 (et certains 503).
  La doc ne mentionne pas le format date HTTP, mais le parser defensivement (entier,
  sinon date HTTP, sinon defaut).
- `X-RateLimit-Limit`, `X-RateLimit-Remaining` ; `X-RateLimit-Reset` (timestamp
  ISO 8601, sur 429) ; `X-RateLimit-NearLimit: true` quand < 20 % de capacite restante.
- `RateLimit-Reason` : `jira-quota-global-based`, `jira-quota-tenant-based`,
  `jira-burst-based`, `jira-per-issue-on-write`.
- `Beta-Retry-After` / `X-Beta-RateLimit-*` : informatifs (phase beta), **ne pas**
  declencher de retry dessus.

Strategie recommandee par la doc :
- delai = max(`Retry-After`, backoff exponentiel) ; base ~2 s (le pseudo-code de la
  doc part de 5 s - **divergence interne a la page**), doublement, plafond ~30 s,
  jitter multiplicatif ~0.7-1.3, **max 4 tentatives**.
- Ne retenter que les requetes **idempotentes** (GET, PUT, transitions apres relecture).
  Pour POST creation (issue, version) : ne pas retenter automatiquement (risque de
  doublon) sauf sur 429 (requete non traitee). **Divergence** : la section bonnes
  pratiques dit de ne retenter qu'en presence de `Retry-After`, le pseudo-code retente
  aussi les 429 sans cet en-tete.
- `jira-per-issue-on-write` : espacer les ecritures sur la meme issue (ex. rangs
  successifs rapides) ; `jira-burst-based` : ralentir l'endpoint ; quota : tout
  suspendre jusqu'a `X-RateLimit-Reset`.
- Pour la TUI : surveiller `X-RateLimit-NearLimit` pour ralentir le rafraichissement
  automatique.

Fixture : `crates/jira-client/tests/fixtures/errors/429_envelope.json` est une
**enveloppe de test** (`status`, `headers`, `body`), pas un corps Jira : a utiliser
avec le mock HTTP (status + en-tetes).

Source : https://developer.atlassian.com/cloud/jira/platform/rate-limiting/

## Fixtures

- `crates/jira-client/tests/fixtures/errors/400_field_errors.json` (erreurs par champ, creation)
- `crates/jira-client/tests/fixtures/errors/400_jql.json` (JQL invalide)
- `crates/jira-client/tests/fixtures/errors/401.json`
- `crates/jira-client/tests/fixtures/errors/403.json`
- `crates/jira-client/tests/fixtures/errors/404.json`
- `crates/jira-client/tests/fixtures/errors/429_envelope.json`

Les messages sont representatifs, pas des chaines garanties : ne jamais faire de
logique sur le texte des messages, uniquement sur le code HTTP et les cles de `errors`.

## Ajouts phase 2b

### Redirections (3xx)

Ce que dit la doc developer.atlassian.com (relue 2026-11-11) :
- La page rate limiting et la page d'intro REST v3 ne mentionnent **aucun** code 3xx
  ni changement de site. Aucune entree du changelog Jira Cloud platform trouvee sur
  le sujet.
- Seule mention explicite, dans *Get issue* / *Bulk fetch issues* : si l'identifiant
  ne correspond pas exactement (casse, issue deplacee), Jira cherche l'issue et
  renvoie ses details ; "a 302 or other redirect is **not** returned". Une issue
  deplacee (`DEMO-5` -> `OPS-12`) est donc servie en **200** sous sa nouvelle cle :
  comparer `key` de la reponse a la cle demandee.
- Des operations asynchrones de la plateforme (hors perimetre TUI : suppression de
  projet, certaines operations bulk) repondent **303 See Other** avec `Location` vers
  `/rest/api/3/task/{taskId}`. Aucun endpoint du perimetre TUI n'est documente comme
  renvoyant 3xx.
- Changement de nom de site (`old.atlassian.net` -> `new.atlassian.net`) : **non
  documente** sur developer.atlassian.com. Une recherche fait apparaitre un article KB
  support.atlassian.com (hors sources autorisees, non consulte) intitule "REST API
  calls failing with error 4xx response code after Cloud site is renamed" : indice
  que l'API n'est **pas** redirigee de facon fiable apres renommage (echecs 4xx plutot
  que 301). A traiter comme incertain.

Consequence pour le client (conforme a `http.rs` : `redirect::Policy::none()`,
3xx = erreur) : ne jamais suivre une redirection automatiquement (risque de fuite du
jeton vers un autre hote et de contournement de l'origine fixee) ; afficher l'en-tete
`Location` dans le message pour que l'utilisateur corrige l'URL de base.
Fixture : `crates/jira-client/tests/fixtures/errors/p2b_302_envelope.json` (enveloppe
de test, hote fictif `jira-renamed.example.test`).

Sources : https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-issues/#api-rest-api-3-issue-issueidorkey-get ,
https://developer.atlassian.com/cloud/jira/platform/rate-limiting/

### `Retry-After` : format exact

- Doc rate limiting : "Indicates how many seconds to wait before retrying." Exemple
  `Retry-After: 1847`. **Uniquement des secondes entieres** dans la doc ; le format
  HTTP-date (RFC 9110) n'est **jamais** mentionne. Le parseur defensif du client
  (entier, puis IMF-fixdate) reste correct.
- Present sur **429** ; "Some transient 5xx responses (such as 503) may also include a
  `Retry-After` header" : ce ne sont pas des reponses de rate limit, mais la meme
  logique de retry s'applique (requetes idempotentes uniquement).
- Valeurs grandes possibles (exemple officiel : 1847 s, ~31 min, cas d'un quota
  `jira-quota-*`) : le plafond du client (echec immediat au-dela) est pertinent.

### En-tetes `X-RateLimit-*` et beta

| En-tete | Quand | Format |
|---|---|---|
| `X-RateLimit-Limit` | reponses soumises a limite | entier (requetes/s pour la limite de debit) |
| `X-RateLimit-Remaining` | idem | entier |
| `X-RateLimit-Reset` | **429 uniquement** | ISO 8601, ex. `2025-10-08T15:00:00Z` (pas un epoch) |
| `X-RateLimit-NearLimit` | quand < 20 % restant | `true` ; "not used for request rate limiting" |
| `RateLimit-Reason` | **429 uniquement** | `jira-quota-global-based`, `jira-quota-tenant-based`, `jira-burst-based`, `jira-per-issue-on-write` |
| `Beta-Retry-After` | reponses qui *seraient* limitees une fois l'application active | secondes ; **informatif, pas de 429** |
| `X-Beta-RateLimit-Limit` / `-Remaining` / `-NearLimit` | variantes beta "legacy" | memes semantiques |
| `X-Beta-RateLimit-Reset` / `-Reason` | seulement si la reponse *serait* limitee | memes formats |

Regle client : un `Beta-Retry-After` ou `X-Beta-*` sur une reponse **2xx** ne doit
declencher ni erreur ni retry (au plus un ralentissement du rafraichissement).

Fixtures (enveloppes de test `status`/`headers`/`body`, pas des corps Jira) :
- `crates/jira-client/tests/fixtures/errors/p2b_429_envelope_full.json` (429, `Retry-After: 1847`, quota tenant)
- `crates/jira-client/tests/fixtures/errors/p2b_503_retry_after_envelope.json` (503 HTML + `Retry-After`)
- `crates/jira-client/tests/fixtures/errors/p2b_200_beta_headers_envelope.json` (200 avec en-tetes beta : ne pas retenter)
- `crates/jira-client/tests/fixtures/errors/p2b_302_envelope.json` (redirection a refuser)

Source : https://developer.atlassian.com/cloud/jira/platform/rate-limiting/

### Codes "permission" atypiques releves en phase 2b

Ne pas supposer 403 pour un defaut de permission :
- `PUT /rest/api/3/version/{id}` : permission insuffisante -> **400**.
- `POST /rest/api/3/version/{id}/removeAndSwap` : permission insuffisante -> **404**.
- `POST /rest/api/3/issue/{key}/transitions` : pas le droit de transitionner -> **400**.
- Agile (`/rest/agile/1.0/sprint...`) : 403 = "does not have a valid license" (et
  permission *Manage Sprints* manquante en pratique).
