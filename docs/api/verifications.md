# Vérifications de suppositions sur Jira Cloud (documentation publique uniquement)

Date de la vérification : 2026-11-11. Aucune instance Jira réelle n'a été appelée.

Méthode et limites : les pages `developer.atlassian.com` sont rendues en JavaScript et le
fetch les renvoie tronquées ; plusieurs citations viennent donc de la spec OpenAPI publiée
(`https://developer.atlassian.com/cloud/jira/software/swagger.v3.json`), de pages
`support.atlassian.com`, de tickets `jira.atlassian.com` (comportement produit, pas contrat API)
ou de miroirs tiers (signalés). Un verdict « NON DOCUMENTÉ » veut dire : pas trouvé dans la doc
officielle lisible, avec l'éventuelle preuve indirecte indiquée. Niveaux de preuve :
[OFFICIEL] doc Atlassian, [TICKET] jira.atlassian.com, [COMMUNAUTÉ] forum, [MIROIR] copie tierce.

---

## 1. JQL : `openSprints()`, `futureSprints()`, backlog, `subTaskIssueTypes()`

Verdict : **CONFIRMÉ** (openSprints = actifs seulement ; futureSprints existe) ; **CONFIRMÉ avec réserve**
(JQL du backlog) ; **NON DOCUMENTÉ** précisément pour `subTaskIssueTypes()` (casse).

- `openSprints()` : « issues assigned to a sprint that was started but has not yet been completed »
  (sprint démarré, non clos = actif). `futureSprints()` : sprints planifiés non démarrés.
  Une réponse communautaire précise : si l'issue est dans un sprint du backlog, « you would need
  to include the JQL clause futureSprints() as well ». [COMMUNAUTÉ + doc JQL functions]
  https://support.atlassian.com/jira-software-cloud/docs/advanced-search-reference-jql-functions/
  (page non lisible via le fetch ; contenu recoupé par
  https://community.atlassian.com/forums/Jira-questions/How-can-JQL-return-current-sprint-and-future-sprints/qaq-p/2976777 ).
- `futureSprints()` existe sur Cloud : oui (même page de référence des fonctions JQL).
- Backlog d'un board = issues sans sprint actif ni futur. La JQL du code
  `sprint is EMPTY OR (sprint not in openSprints() AND sprint not in futureSprints())` est la
  forme correcte : `openSprints()`/`futureSprints()` ne ramènent que les issues ayant un sprint, d'où
  le `sprint is EMPTY` (un utilisateur : « only pulls back issues that have a sprint, so issues
  with no sprint are missed »). Une issue d'un sprint clos + d'un sprint futur reste exclue (correct).
  Une issue uniquement dans un sprint clos est dans le backlog (cohérent avec l'UI).
- `issuetype not in subTaskIssueTypes()` : la fonction existe (« produces all the sub-task issue
  types », Javadoc `AllSubIssueTypesFunction`). La doc la cite sous la graphie `subtaskIssueTypes()`
  (t minuscule). La casse des noms de fonctions JQL n'est pas affirmée par les pages lues :
  NON DOCUMENTÉ ici (en pratique JQL est insensible à la casse, non vérifié sur doc).
  https://docs.atlassian.com/jira/latest/com/atlassian/jira/plugin/jql/function/AllSubIssueTypesFunction.html
- Divergence à noter : le backlog UI d'un board filtre sur la **dernière colonne** du board
  (voir Q5), pas sur `statusCategory`. `statusCategory != Done` est une approximation.

Conséquence : JQL actuelle valide. Préférer `subtaskIssueTypes()` (graphie de la doc) par prudence.
Prévoir `GET /rest/agile/1.0/board/{id}/backlog` comme alternative exacte (filtre board + colonnes).

## 2. Projets team-managed : type de board, paramètre `type`, sprints

Verdict : **CONFIRMÉ** (`simple`) ; **NON DOCUMENTÉ** (format multi-valeurs) ; **CONFIRMÉ avec réserve**
(sprints dépendants de la fonction « Sprints » du board).

- Type : le ticket Atlassian JSWCLOUD-20591 / JRACLOUD-85696 : « The REST API endpoints to get
  boards always return "simple" as the board type for Team Managed projects ». Le type réel varie
  selon les fonctions activées (Kanban sans sprint ni backlog, « Kanplan » backlog sans sprint,
  Scrum avec les deux). Atlassian suggère `GET /rest/agile/1.0/board/{boardId}/features`. [TICKET]
  https://jira.atlassian.com/browse/JSWCLOUD-20591
- Paramètre `type` de `GET /rest/agile/1.0/board` : « Filters results to boards of the specified
  types. Valid values: scrum, kanban, simple. » [OFFICIEL, spec OpenAPI Software] Le format pour
  plusieurs valeurs (CSV ? paramètre répété ?) n'est **pas** précisé. Le pluriel « types » suggère
  le multi-valeurs ; `scrum,simple` (CSV) est ce que fait le code, non confirmé par la doc.
- Sprints sur board `simple` : pas de doc officielle sur la réponse. JSWCLOUD-18867 (Won't Fix) montre
  qu'un board next-gen sans la fonction Sprints peut ne pas renvoyer `estimation` ; un 400/404 sur
  `/board/{id}/sprint` quand les sprints sont désactivés est plausible mais le message exact n'est
  pas documenté (NON DOCUMENTÉ). Activer les sprints par API (`PUT /board/{id}/features`) : 500
  rapporté en 2026 dans un cas [COMMUNAUTÉ].
  https://jira.atlassian.com/browse/JSWCLOUD-18867

Conséquence : le code (`scrum_boards`, `sprints`) est cohérent mais repose sur du non documenté :
(a) si la liste `type=scrum,simple` revient vide/400, repli sur deux requêtes (`scrum`, puis `simple`)
ou sur aucun filtre `type` + filtrage client ; (b) pour un board `simple`, appeler `/board/{id}/features`
plutôt que deviner via 400/404.

## 3. Champ story points

Verdict : **CONFIRMÉ** (deux champs distincts, coexistence possible) ; **NON DOCUMENTÉ** (`jsw-story-points`) ;
**CONFIRMÉ** (estimation du board via `/configuration`, avec réserve team-managed).

- Atlassian : « the custom field that Jira uses to store estimates in company-managed spaces
  (Story points) is different to the custom field used in team-managed spaces (Story point estimate) ».
  https://support.atlassian.com/jira-software-cloud/docs/migrate-between-team-managed-and-company-managed-projects/
- Un même site peut donc avoir les deux (un projet de chaque type) ; ce sont deux custom fields
  distincts, non interchangeables ; unification seulement envisagée (JSWCLOUD-17173). Le nom
  `jsw-story-points` (type de schéma `com.pyxis.greenhopper.jira:jsw-story-points`) n'a été confirmé
  par aucune source lue : ne pas s'appuyer dessus comme discriminant sans test sur `/field`.
- Estimation d'un board : `GET /rest/agile/1.0/board/{boardId}/configuration` →
  `estimation.type` = `field` et `estimation.field.fieldId` (ex. `customfield_10016`, `displayName`
  « Story point estimate »). Exemple tiré de JSWCLOUD-18867 [TICKET], pas d'exemple complet dans la
  doc officielle lue. La doc de `GET /issue/{key}/estimation` dit : « The field used for estimation
  on the given board can be obtained from [board configuration resource] ».
- Piège : sur board team-managed sans la fonction Sprints, `estimation` peut être absent de la
  configuration (JSWCLOUD-18867, Won't Fix). Piège : `GET /issue/{key}/estimation` exige `boardId`
  en pratique (doc muette) [COMMUNAUTÉ] https://community.developer.atlassian.com/t/how-to-get-issue-details-story-point-estimate-by-api/79195

Conséquence : `points_candidates` / `points_field_for_write` (http.rs) : ordre de priorité
1) `estimation.field.fieldId` du board, 2) repli par nom dans `/field` (« Story Points » puis
« Story point estimate »), en tolérant `estimation` absent. Lire les deux champs à l'affichage.

## 4. Déplacer des sous-tâches vers un sprint / le backlog

Verdict : **NON DOCUMENTÉ** côté API ; comportement produit **CONFIRMÉ par ticket**.

- API : `POST /sprint/{id}/issue` : « The maximum number of issues that can be moved in one
  operation is 50 » ; `POST /backlog/issue` : limite 50 par requête. [OFFICIEL] Rien sur les sous-tâches.
- Produit : JRACLOUD-98079 (« Allow subtasks to be independently assigned to sprints ») : les sous-tâches
  ne peuvent pas être assignées indépendamment, elles héritent du sprint du parent ; erreur UI
  « Subtasks can't be moved from their parent on the backlog ». [TICKET, statut « Gathering Interest »]
  https://jira.atlassian.com/browse/JRACLOUD-98079
- Doc produit (company-managed) : à la clôture, « all sub-tasks must be marked as Done ».
  https://support.atlassian.com/jira-software-cloud/docs/complete-a-sprint/

Conséquence : ne jamais envoyer de sous-tâches à `move_to_sprint`/`move_to_backlog` (déplacer le
parent). Le backlog exclut déjà les sous-tâches (`issuetype not in subtaskIssueTypes()`). Traiter un
400 sur ces endpoints avec un message explicite. Le suivi parent → sous-tâches est
« automatique » côté produit, mais non garanti par la doc de l'API.

## 5. Clôture d'un sprint par API

Verdict : **NON DOCUMENTÉ** (sort des issues non terminées via REST) ; **CONFIRMÉ** pour la définition
UI de « terminé » (dernière colonne).

- API : « A sprint can be completed by updating the state to 'closed' » ; le sprint doit être
  `active`, `completeDate` posé à l'heure de la requête. Rien sur les issues. [OFFICIEL]
- Un utilisateur rapporte que la clôture par REST renvoie les issues non terminées **au backlog**
  sans option de destination [COMMUNAUTÉ] https://community.atlassian.com/t5/Jira-Software-questions/JiraAgile-API-When-completing-a-Sprint-how-to-move-open-issues/qaq-p/710378 .
  `incompleteIssuesDestinationId` n'apparaît que dans un ticket Data Center (JSWSERVER-26129), pas
  dans la spec Cloud lue : ne pas l'utiliser.
- UI (company-managed) : « Work items are considered as complete once they have been moved to the
  last column on the board » ; destinations : backlog, sprint futur ou nouveau sprint ; issues
  d'autres boards (sprint partagé) → backlog. Comportement team-managed non décrit (page limitée à
  company-managed). https://support.atlassian.com/jira-software-cloud/docs/complete-a-sprint/
- Les issues terminées restent dans le sprint clos (rapport/vélocité), non détaillé dans l'API.

Conséquence (`close_sprint`, http.rs) : pour contrôler la destination, **d'abord** lire les issues
ouvertes (`sprint = X AND statusCategory != Done`, en sachant que le critère produit est la dernière
colonne), les déplacer vers le sprint cible (`move_to_sprint`) ou le backlog, **puis** clore. Prévenir
l'utilisateur que sans cela le sort est « probablement le backlog » (non garanti).

## 6. `moveUnfixedIssuesTo` à la livraison d'une version

Verdict : **NON DOCUMENTÉ** pour la définition d'« unfixed » ; indice communautaire : résolution vide.

- Doc (formulation reprise par miroirs) : URL `self` de la version « to which all unfixed issues
  are moved when a version is released » (inutile à la création, optionnel à la mise à jour). [OFFICIEL
  pour l'existence du champ, définition absente] https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-project-versions/
- Définition de « non résolue » : l'UI de release compte les issues **sans résolution** (champ
  Resolution vide), pas la catégorie de statut ; Atlassian aurait aligné l'avertissement sur la
  résolution. Des issues « Done » sans résolution (workflow qui ne pose pas de résolution) sont donc
  déplacées aussi [COMMUNAUTÉ]
  https://community.atlassian.com/t5/Jira-questions/Problem-when-releasing-a-version/qaq-p/2603178
- « ignore unresolved » laisse les issues sur la version livrée.

Conséquence (`update_version`, http.rs ~l.1850) : le contrat est « issues sans résolution » ; ne pas
promettre « statut non Done ». Avant release, afficher le décompte via
`resolution is EMPTY AND fixVersion = X` pour prévenir l'utilisateur.

## 7. Plusieurs sprints actifs sur un board

Verdict : **CONFIRMÉ** (company-managed) ; **NON DOCUMENTÉ** (team-managed).

- « The Parallel sprints feature lets you enable multiple active sprints that are running in
  parallel with each other » (page « company-managed spaces »). Activation par un admin Jira,
  portée instance. Sans l'option, démarrer un 2e sprint : « There can only be one active sprint on
  each board ». https://support.atlassian.com/jira-software-cloud/docs/what-are-parallel-sprints/ ,
  https://support.atlassian.com/jira-software-cloud/docs/use-parallel-sprints/ ,
  https://support.atlassian.com/jira/kb/cannot-start-sprint-with-error-there-can-only-be-one-active-sprint/
- Effet API : `GET /board/{id}/sprint?state=active` renvoie logiquement tous les sprints actifs
  (déduction, pas de citation). Pour team-managed, une réponse communautaire de 2022 dit un seul actif.

Conséquence : ne pas supposer un sprint actif unique ; `current_sprint` (http.rs) et la TUI doivent
gérer N sprints actifs (choix/affichage de tous). Le démarrage peut échouer avec le message ci-dessus
(400) : l'afficher tel quel.

## 8. `reconcileIssues` de `POST /rest/api/3/search/jql`

Verdict : **CONFIRMÉ** (ids numériques, max 50) ; clés : **NON DOCUMENTÉ**.

- Page officielle : exemple `"reconcileIssues": [10000]` (tableau d'ids numériques) ; « API can accept
  a maximum of 50 reconcile issues » ; but : résultats cohérents juste après mise à jour ;
  « consistency is ensured only for the specified issues » ; une issue réconciliée n'est renvoyée
  que si elle correspond aussi à la JQL. https://developer.atlassian.com/cloud/jira/platform/search-and-reconcile/
- Variante GET (miroir docs.rs de la spec) : « Strong consistency issue ids to be reconciled with
  search results. Accepts max 50 ids » ; `maxResults` : « max 5000 » ; `nextPageToken` « not included
  in the response for the last page » (pas de `isLast`). La liste doit rester identique d'une page à
  l'autre (miroirs tiers). https://docs.rs/crate/jira_v3_openapi/1.5.3/source/docs/IssueSearchApi.md
- Un exemple Pipedream parle « d'ids ou clés » : non corroboré par Atlassian ; utiliser des ids.
- Le changelog Atlassian lu ne mentionne pas `reconcileIssues`.

Conséquence : le code (`RECONCILE_MAX = 50`, `reconcile_ids` en `u64`, passage dans
`search` ~l.654, même liste sur toutes les pages) est conforme. Garder l'id numérique (map clé → id).
Rappel : la garantie ne couvre que les issues listées (une issue créée mais absente de la liste peut
manquer d'une JQL).

## 9. `GET /board/{id}/sprint` : sprints d'autres boards, `originBoardId`

Verdict : **CONFIRMÉ** (sprints partagés possibles) ; `originBoardId` **CONFIRMÉ présent** dans les
exemples ; filtre serveur **NON DOCUMENTÉ**.

- Doc : « Returns all sprints from a board, for a given board ID. This only includes sprints that the
  user has permission to view. » Paramètres : `state` (`future`, `active`, `closed`, CSV), pagination.
  [OFFICIEL]
- KB Atlassian : un sprint peut apparaître sur plusieurs boards (filtres qui se recoupent) ; son
  board d'origine se lit via `originBoardId` de `GET /sprint/{id}`.
  https://support.atlassian.com/jira/kb/sprints-appearing-on-multiple-boards-in-jira-data-center-server-or-cloud
- `originBoardId` figure dans les réponses d'exemple (`id, self, state, name, startDate, endDate,
  completeDate, originBoardId, goal`) et est requis à la création. Aucun paramètre ne filtre les
  sprints par board d'origine ; Jira Align compare `originBoardId` au board visé. Présence garantie
  dans toutes les réponses de liste : non explicitée, la traiter comme optionnelle.
- Clôture d'un sprint partagé : les issues d'autres boards vont au backlog (voir Q5).

Conséquence : `SprintDto` ne lit pas `originBoardId` ; `sprints()` fait `sprint_board.insert(s.id,
board_id)` : pour un sprint partagé, le board mémorisé est le dernier board qui l'a listé, ce qui
peut fausser le catalogue de champs. Lire `originBoardId` (Option) et le préférer ; optionnellement
filtrer/marquer les sprints dont `originBoardId != board_id`.

---

## Tableau récapitulatif

| # | Question | Verdict | Source | Risque pour le client |
|---|----------|---------|--------|-----------------------|
| 1a | `openSprints()` = actifs seulement | CONFIRMÉ | doc JQL + communauté | faible |
| 1b | `futureSprints()` sur Cloud | CONFIRMÉ | doc JQL | faible |
| 1c | JQL backlog actuelle | CONFIRMÉ (réserve : Done = colonne) | doc JQL | divergence avec backlog UI |
| 1d | `subTaskIssueTypes()` valide | CONFIRMÉ (existence) / casse NON DOCUMENTÉ | Javadoc | utiliser `subtaskIssueTypes()` |
| 2a | Board team-managed = `simple` | CONFIRMÉ | JSWCLOUD-20591 | type variable selon fonctions |
| 2b | `type=` multi-valeurs | NON DOCUMENTÉ | spec Software | repli 2 requêtes |
| 2c | Sprints sur board `simple` | NON DOCUMENTÉ (dépend de la fonction Sprints) | JSWCLOUD-18867 | 400/404 tolérés |
| 3a | Champs story points distincts | CONFIRMÉ | doc migration | lire les deux |
| 3b | `jsw-story-points` | NON DOCUMENTÉ | aucune | ne pas s'y fier |
| 3c | `estimation.field.fieldId` | CONFIRMÉ (exemple ticket) | JSWCLOUD-18867 | absent si sprints off |
| 4 | Sous-tâches via sprint/backlog | NON DOCUMENTÉ API ; héritage parent CONFIRMÉ (ticket) | JRACLOUD-98079 | filtrer les sous-tâches |
| 5a | Issues ouvertes à la clôture (API) | NON DOCUMENTÉ (backlog selon communauté) | forum | déplacer avant de clore |
| 5b | « Terminé » | dernière colonne (UI, company-managed) | support.atlassian.com | pas statusCategory |
| 6 | `moveUnfixedIssuesTo` | NON DOCUMENTÉ (résolution vide selon communauté) | forum | Done sans résolution déplacées |
| 7 | Sprints actifs parallèles | CONFIRMÉ (company-managed, option admin) | support.atlassian.com | N sprints actifs |
| 8 | `reconcileIssues` | CONFIRMÉ : ids numériques, max 50 | developer.atlassian.com | conforme |
| 9a | Sprints d'autres boards | CONFIRMÉ | KB Atlassian | board mémorisé erroné |
| 9b | `originBoardId` dans la réponse | CONFIRMÉ (exemples) | spec Software | à lire dans `SprintDto` |

## Actions recommandées pour le code

Fichier : `/home/jerome/claude/jira-rust/crates/jira-client/src/http.rs` (lecture seule, aucune modification faite).

1. `backlog_issues` (~l.1480) : conserver le `sprint is EMPTY OR (not in openSprints() AND not in
   futureSprints())` ; remplacer `subTaskIssueTypes()` par `subtaskIssueTypes()` (graphie de la doc) ;
   documenter que `statusCategory != Done` approxime la « dernière colonne » ; envisager
   `GET /board/{id}/backlog` si un écart apparaît. Le commentaire de module (l.9-13) est exact.
2. `scrum_boards` (~l.1401) : `type=scrum,simple` n'est pas garanti par la doc. Prévoir un repli
   (deux requêtes ou pas de filtre `type`) si la réponse est vide ou en 400.
3. `sprints` (~l.1432) : pour un board `simple`, consulter `/board/{id}/features` avant de dépendre
   de la tolérance 400/404 ; lire `originBoardId` (`SprintDto`) et ne pas écraser `sprint_board`
   d'un sprint avec le board qui le liste si `originBoardId` est connu et différent.
4. `board_config` / `points_candidates` / `points_field_for_write` (~l.555-595) : tolérer
   `estimation` absent (team-managed sans sprints) ; repli par nom dans `/field` ; ne pas dépendre de
   `jsw-story-points`.
5. `move_to_sprint` / `move_to_backlog` / `post_in_batches` (~l.704-1515) : lots de 50 corrects ;
   refuser ou remplacer les sous-tâches par leur parent côté appelant ; message clair sur 400.
6. `close_sprint` (~l.1588) : ne pas compter sur un déplacement des issues ouvertes ; offrir (ou
   documenter) « déplacer les issues non terminées puis clore ». Ne pas utiliser
   `incompleteIssuesDestinationId` (Data Center).
7. `update_version` (~l.1850) : ne documenter `moveUnfixedIssuesTo` que comme « issues sans
   résolution » ; avertir que des issues Done sans résolution seront aussi déplacées.
8. `current_sprint` (~l.1167) et la couche TUI : gérer plusieurs sprints actifs (parallel sprints) ;
   afficher l'erreur « only one active sprint » au démarrage.
9. `search` (~l.616-660) : rien à changer pour `reconcileIssues` (ids `u64`, max 50, liste identique
   sur chaque page, pagination par `nextPageToken`).
