# Auth, utilisateur courant et decouverte des champs

Base URL : `https://<site>` (fourni par la config utilisateur, jamais code en dur).
Dans les fixtures, le site fictif est `https://jira.example.test`.

## Authentification

- **Basic auth** : `Authorization: Basic base64(email:api_token)`. Jeton API cree par
  l'utilisateur sur son compte Atlassian. Pas de mot de passe.
- Toujours envoyer `Accept: application/json` ; `Content-Type: application/json` pour
  les corps.
- **Incertitude** : Atlassian propose aussi des jetons API *scopes* (granulaires) qui,
  d'apres la doc securite, doivent etre utilises via la passerelle
  `https://api.atlassian.com/ex/jira/{cloudId}/rest/...` et non via l'URL du site.
  A verifier sur la page "Basic auth for REST APIs" avant de supporter ce mode ;
  prevoir dans la config une `base_url` libre pour couvrir les deux cas.
- OAuth 2.0 (3LO) hors perimetre : les scopes cites ci-dessous servent de reference
  de permissions uniquement.

Source : https://developer.atlassian.com/cloud/jira/platform/basic-auth-for-rest-apis/

---

## GET /rest/api/3/myself

Utilisateur authentifie. Sert de "ping" de validation des credentials au demarrage.

| Param query | Type | Notes |
|---|---|---|
| `expand` | string | `groups`, `applicationRoles` (inutile pour la TUI) |

- **200** : objet `User`. Champs utiles : `accountId` (identifiant stable, a utiliser
  partout, ex. JQL `assignee = "<accountId>"`), `displayName`, `emailAddress`,
  `timeZone`, `locale`, `active`, `avatarUrls`.
- **401** : credentials absents/invalides.
- Permission : acces a Jira.
- **Piege** : `emailAddress` peut etre absent selon les reglages de confidentialite du
  profil -> `Option<String>`. Ne jamais utiliser `name`/`key` (supprimes, GDPR).

Fixture : `crates/jira-client/tests/fixtures/auth/myself.json`

Source : https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-myself/#api-rest-api-3-myself-get

---

## GET /rest/api/3/field

Liste **non paginee** de tous les champs (systeme + custom) visibles par l'utilisateur.
(Il existe `GET /rest/api/3/field/search` pagine, mais il exige les droits admin pour
une partie de ses fonctions : inutile ici.)

- **200** : tableau de `FieldDetails` : `id`, `key`, `name`, `custom`, `orderable`,
  `navigable`, `searchable`, `clauseNames`, `schema`.
- `schema` : `{ type, items?, system? }` pour les champs systeme,
  `{ type, items?, custom, customId }` pour les custom fields.
- Permissions : aucune particuliere (les champs non visibles sont filtres).

### Identifier les champs custom necessaires a la TUI

Les ids `customfield_NNNNN` **varient d'une instance a l'autre**. Ne jamais les coder.

| Concept | Regle d'identification (`schema.custom`) | Valeur dans une issue |
|---|---|---|
| Sprint | `com.pyxis.greenhopper.jira:gh-sprint` (`type: array`, `items: json`) | `null` ou tableau d'objets `{id, name, state, boardId, goal, startDate?, endDate?, completeDate?}` |
| Rank | `com.pyxis.greenhopper.jira:gh-lexo-rank` (`type: any`) | string LexoRank opaque, ex. `"0|i0000f:"` |
| Story point estimate | `com.pyxis.greenhopper.jira:jsw-story-points` (`type: number`) | `null` ou nombre (f64) |
| Story Points (classique) | `com.atlassian.jira.plugin.system.customfieldtypes:float` **et** nom `Story Points` | `null` ou nombre (f64) |

**Divergence / correction de l'hypothese initiale** : le champ "Story point estimate"
n'est PAS de type `customfieldtypes:float` ; il a son propre type
`com.pyxis.greenhopper.jira:jsw-story-points`. Ce type n'est documente nulle part dans
la reference officielle : il est atteste par des echanges sur
community.developer.atlassian.com. Le champ legacy "Story Points" est un simple `float`,
donc indiscernable de n'importe quel autre champ numerique (cf. "Estimation interne"
dans la fixture) autrement que par son nom - nom qui peut etre traduit/renomme.

**Methode recommandee (fiable)** : lire le champ d'estimation configure sur le board via
`GET /rest/agile/1.0/board/{boardId}/configuration` -> `estimation.field.fieldId`
(et `ranking.rankCustomFieldId` pour le Rank). Voir `projects.md`.
Ordre de resolution conseille :
1. board configuration (`estimation.type == "field"`) ;
2. sinon `/field` : `schema.custom == ...:jsw-story-points` ;
3. sinon `/field` : `schema.custom == ...:float` et `name`/`untranslatedName` == `Story Points` ;
4. sinon : pas de story points (afficher `-`), surcharge possible dans la config.

Pour le Sprint, s'il existe plusieurs champs `gh-sprint` (rare, migrations), prendre
celui dont le nom est `Sprint`, sinon le premier, et le signaler en log.

Fixture : `crates/jira-client/tests/fixtures/auth/fields.json` (contient les deux
variantes de story points + un `float` leurre).

Source : https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-issue-fields/#api-rest-api-3-field-get
