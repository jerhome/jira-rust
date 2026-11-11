# Guide d'utilisation de jira-rust

`jira-rust` est une interface terminal (TUI) pour gérer le backlog, le sprint actif et les versions d'un projet Scrum sur Jira Cloud. Ce guide décrit le comportement actuel du programme. L'aide intégrée (touche `?`) reste la référence rapide des raccourcis.

Sommaire : [Lancer](#lancer-le-programme) · [Configuration](#configuration) · [Les trois onglets](#les-trois-onglets) · [Raccourcis](#raccourcis) · [Écritures](#flux-décriture) · [Lecture seule](#mode-lecture-seule) · [Erreurs](#messages-derreur-courants) · [Logs](#logs) · [Limites](#limites-connues)

## Lancer le programme

```
jira-rust [--mock] [--config FICHIER] [--read-only] [doctor | config-path]
```

| Option / commande | Effet |
|---|---|
| (aucune) | lance la TUI |
| `--mock` | données fictives en mémoire, aucun accès réseau ni fichier de configuration requis |
| `--config FICHIER` | utilise ce fichier de configuration au lieu de `~/.config/jira-rust/config.toml` |
| `--read-only` | lecture seule : aucune écriture n'est envoyée à Jira |
| `doctor` | vérifie la configuration et la connexion (appel de `/myself`) ; toujours en lecture seule |
| `config-path` | affiche le chemin du fichier de configuration et un exemple de contenu |

Les options sont globales : `jira-rust doctor --config ./test.toml` fonctionne.

Au démarrage, si un seul projet est accessible, puis un seul board Scrum, ils sont choisis automatiquement. Sinon un sélecteur propose d'abord les projets, puis les boards Scrum du projet choisi.

## Configuration

### Fichier

Chemin par défaut : `~/.config/jira-rust/config.toml` (sous Linux ; `jira-rust config-path` donne le chemin exact sur votre machine).

```toml
base_url = "https://<votre-site>.atlassian.net"
email    = "prenom.nom@entreprise.com"
# api_token = "..."   # facultatif : voir ci-dessous
```

- `base_url` : obligatoire. Doit commencer par `https://` (`http://` n'est accepté que pour `localhost`, `127.0.0.1` et `::1`). Une barre oblique finale est ignorée.
- `email` : obligatoire, l'adresse de votre compte Atlassian.
- `api_token` : facultatif (voir ci-dessous).
- Toute clé inconnue dans le fichier est refusée (message « configuration invalide »).

### Token API

Créez un token sur <https://id.atlassian.com/manage-profile/security/api-tokens>. Deux façons de le fournir :

1. **Variable d'environnement `JIRA_API_TOKEN`** (recommandée, prioritaire) :
   `export JIRA_API_TOKEN=...` (ou `set -x JIRA_API_TOKEN ...` sous fish).
2. **Clé `api_token`** dans le fichier. Le fichier doit alors être privé : `chmod 600 ~/.config/jira-rust/config.toml`. S'il est lisible par d'autres utilisateurs, le programme refuse de démarrer, même si `JIRA_API_TOKEN` est définie.

Si aucun token n'est trouvé, le programme s'arrête avec un message qui le dit. Le token n'est jamais affiché ni journalisé.

### Vérifier

```
jira-rust doctor
```

Affiche le fichier utilisé, l'instance, le compte, l'origine du token (variable ou fichier) puis le résultat de la connexion (« OK — connecté en tant que … » ou « ÉCHEC » avec l'erreur). Sans écriture.

## Les trois onglets

L'en-tête montre les onglets (`1 Backlog`, `2 Sprint actif`, `3 Versions`) et, à droite, `PROJET · board`, suivi de `[LECTURE SEULE]` le cas échéant. La ligne du bas (barre de raccourcis) rappelle les touches principales de l'onglet courant (le détail complet est dans `?`) ; la ligne de statut affiche les messages (vert : information, rouge : erreur), précédés d'un spinner « Chargement… » tant qu'une requête est en cours.

Barre de raccourcis par écran :

| Écran | Contenu de la barre |
|---|---|
| Backlog | `j/k` · `Entrée` détail/replier · `t` transition · `m` déplacer · `K/J` rang · `c` créer · `e` éditer · `s` démarrer sprint · `?` aide |
| Sprint actif | `h/l` colonne · `j/k` · `Entrée` détail · `t` transition · `m` déplacer · `K/J` rang · `x` clôturer · `s` démarrer · `?` aide |
| Versions | `j/k` · `h/l` liste/issues · `Entrée` détail · `c` créer · `e` éditer · `r` livrer · `t` transition · `?` aide |
| Sélecteur | `j/k` naviguer · `Entrée` choisir · `Esc` retour · `R` recharger · `?` aide · `q` quitter |

### 1 — Backlog

Une liste unique : les **sprints futurs** (en-tête repliable avec nombre d'issues et de points), puis la section **Backlog** (issues hors sprint). Le titre affiche le total de points. Seules les issues non terminées du backlog sont listées ; les sous-tâches ne sont pas affichées (elles suivent leur parent). Chaque ligne indique clé, type, statut, résumé, points.

### 2 — Sprint actif

En haut : nom, dates (avec « J-n » ou « dépassé de n j »), objectif, jauge de points terminés. En dessous, trois colonnes selon la catégorie de statut : **À faire**, **En cours**, **Terminé**, avec nombre d'issues et de points. S'il n'y a pas de sprint actif, un message l'indique (et propose `s` si un sprint futur existe). Si plusieurs sprints sont actifs en parallèle sur le board, seul le premier (dans l'ordre renvoyé par Jira) est affiché ; un avertissement « ⚠ N autre(s) sprint(s) actif(s) en parallèle, non affiché(s) : clôture désactivée » s'ajoute à la ligne des dates. Seuls les sprints dont le board d'origine est le board courant sont pris en compte : les sprints d'autres boards (que Jira liste quand les filtres se recoupent) sont ignorés, et s'il n'y a pas de sprint actif le message ajoute « N sprint(s) d'un autre board ignoré(s) ».

### 3 — Versions

À gauche : les versions du projet, « Non livrées » puis « Livrées » (✓), avec leur date de livraison. À droite : description de la version sélectionnée et ses issues (nombre, terminées, points). Les issues sont chargées à la sélection d'une version.

## Raccourcis

Les raccourcis sont tirés du code ; `?` affiche la même liste dans l'application. Les touches de Vim (`h j k l g G`) coexistent avec les flèches, Début et Fin.

### Global (écran principal)

| Touche | Action |
|---|---|
| `1` / `2` / `3` | onglet Backlog / Sprint actif / Versions |
| `Tab` / `Maj+Tab` | onglet suivant / précédent |
| `j` `k`, `↓` `↑` | descendre / monter le curseur |
| `g` `G`, `Début` `Fin` | premier / dernier élément |
| `h` `l`, `←` `→` | Sprint : colonne précédente / suivante ; Versions : focus liste / issues |
| `Entrée` | détail de l'issue sélectionnée ; Backlog : sur un en-tête de sprint, replie/déplie ; Versions : depuis la liste, passe aux issues |
| `Espace` | Backlog : replie/déplie le sprint courant |
| `p` | changer de projet / board |
| `R` | rafraîchir les données |
| `Esc` | efface le message de statut |
| `?` | aide |
| `q`, `Ctrl+C` | quitter (différé si une écriture est en cours, voir [Flux d'écriture](#flux-décriture)) |

Pendant le chargement initial, les autres touches (dont tous les raccourcis d'écriture) répondent « Chargement en cours… ».

L'aide (`?`) liste ces raccourcis par thème (Général, Issues, Sprints, Versions, Formulaires, confirmation). Elle se ferme avec `Esc`, `?`, `q` ou `Entrée` ; elle est aussi disponible dans le sélecteur.

### Issues (onglets Backlog, Sprint, et issues de Versions)

| Touche | Action |
|---|---|
| `t` | transition de statut (liste des transitions disponibles) |
| `m` | déplacer vers un sprint (actif ou futur) ou vers le backlog |
| `K` / `J` | monter / descendre dans le rang |
| `c` | créer une issue (en Versions : créer une version) |
| `e` | éditer l'issue (en Versions, liste focalisée : éditer la version) |

`K` et `J` s'appliquent aux voisins dans la liste affichée : dans l'onglet Sprint, il s'agit de la colonne courante. Dans l'onglet Versions, ils répondent « Le rang se modifie dans les onglets Backlog et Sprint ». Sur la première (resp. dernière) issue, un message indique « déjà en tête » (resp. « en dernier »).

### Sprints

| Touche | Action |
|---|---|
| `s` | démarrer un sprint futur |
| `x` | clôturer le sprint actif |

### Versions (onglet 3)

| Touche | Action |
|---|---|
| `c` | créer une version |
| `e` | éditer la version sélectionnée (liste focalisée) ou l'issue (issues focalisées) |
| `r` | livrer la version sélectionnée |

### Sélecteur projet / board

| Touche | Action |
|---|---|
| `j` `k`, `↓` `↑` | naviguer |
| `Entrée` | choisir |
| `Esc` | revenir à la liste des projets, ou fermer le sélecteur si un board est déjà ouvert |
| `R` | recharger |
| `?` / `q` | aide / quitter |

### Fenêtres de choix (transitions, déplacement, destination, sprint à démarrer)

| Touche | Action |
|---|---|
| `j` `k`, `↓` `↑`, `g` `G` | naviguer |
| `Entrée` | choisir (ouvre ensuite la confirmation) |
| `Esc`, `q` | annuler |

Fenêtre de détail d'une issue : `Entrée`, `Esc` ou `q` ferment ; `t`, `m` et `e` lancent la transition, le déplacement ou l'édition.

### Formulaires

| Touche | Action |
|---|---|
| `Tab`, `↓` | champ suivant |
| `Maj+Tab`, `↑` | champ précédent |
| `←` `→`, `h` `l`, `Espace` | changer la valeur d'un champ à choix (type, version) |
| `Entrée`, `Ctrl+S` | valider (ouvre la confirmation) ; dans « Description », `Entrée` insère un saut de ligne, seul `Ctrl+S` valide |
| `Esc` | annuler (sans confirmation, la saisie est perdue) |
| `←` `→`, `Début`, `Fin`, `Retour arrière`, `Suppr` | édition d'un champ texte |
| `Ctrl+U` | vide le champ texte |

Le texte collé (collage du terminal, « bracketed paste », activé au démarrage et désactivé à la sortie) arrive en un seul bloc et n'est jamais rejoué comme des frappes : un saut de ligne collé ne valide donc rien. Comportement selon le champ actif :

| Champ actif | Effet du collage |
|---|---|
| Texte sur une ligne (Résumé, Story points, dates, Nom, Objectif…) | inséré ; sauts de ligne et tabulations deviennent des espaces |
| « Description » (multiligne) | inséré ; les fins de ligne (`\r\n`, `\r`) sont normalisées en saut de ligne, les tabulations deviennent des espaces |
| Champ à choix (type, version) ou en lecture seule | ignoré |

Dans les deux cas, les autres caractères de contrôle sont supprimés. Hors formulaire, le collage est ignoré (listes, détail, aide, écran principal). Pendant une fenêtre de confirmation, il est ignoré mais repousse l'échéance de validation (voir ci-dessous).

Si le formulaire contient des modifications non enregistrées (y compris quand il est masqué derrière une confirmation d'écriture) et que vous faites `Ctrl+C`, une fenêtre « Quitter ? » (« Le formulaire contient des modifications non enregistrées. ») demande confirmation : `Entrée`, `o` ou `y` quittent sans enregistrer, `Esc` ou `n` reviennent à l'écran précédent ; toute autre touche est ignorée. Un second `Ctrl+C` dans cette fenêtre quitte aussi. Si vous revenez sur une confirmation d'écriture, elle doit être relue avant de pouvoir être validée (nouveau délai). Sans modification (ou hors formulaire), `Ctrl+C` quitte directement, comme `q` ; `Esc` annule un formulaire sans rien demander.

### Confirmation d'écriture

| Touche | Action |
|---|---|
| `Entrée`, `o`, `y` | confirmer et envoyer |
| `Esc`, `n`, `q` | annuler (retour à la fenêtre précédente, formulaire ou liste) |

Pour éviter les validations accidentelles, une confirmation n'est validable que si elle a réellement été affichée à l'écran, puis laissée affichée au moins 0,4 s (`CONFIRM_DELAY` = 400 ms) sans frappe ni collage. Les touches déjà en attente au moment de l'ouverture ne peuvent donc pas la valider. Une validation (`Entrée`, `o`, `y`) arrivée trop tôt est ignorée, journalisée, repousse l'échéance et affiche « Validation ignorée (trop rapide) : relisez puis confirmez » ; il suffit d'attendre un instant puis de recommencer. Toute autre touche (et tout collage) repousse aussi l'échéance ; une rafale de frappes ne finit jamais par valider. L'annulation (`Esc`, `n`, `q`) est toujours immédiate.

## Flux d'écriture

Toute écriture suit le même parcours : action (touche) → éventuellement choix ou formulaire → **fenêtre de confirmation qui résume ce qui sera envoyé** → `Entrée` → envoi → message de succès ou d'échec → rafraîchissement automatique des données (même en cas d'échec, une opération composée pouvant être partielle). Le curseur se replace sur l'élément concerné.

Les écritures sont **sérialisées** : une seule est en vol à la fois ; si vous en confirmez une autre pendant ce temps, elle est mise en file (message « … : en file d'attente (une écriture est en cours) ») et envoyée dans l'ordre à la fin de la précédente (chaque écriture déclenche son propre rafraîchissement). Si vous quittez (`q` ou `Ctrl+C`) pendant une écriture en cours ou en file, la sortie est différée avec le message « Écriture en cours (N) : patienter, sortie automatique à la fin » ; le programme se ferme une fois la file vidée si la dernière écriture a réussi. Si la dernière écriture échoue, la sortie est annulée et vous restez pour voir l'erreur (le message d'échec est affiché ; Jira a pu appliquer une partie d'une opération composée, d'où le rafraîchissement).

### Rang

`K` (monter) / `J` (descendre) : place l'issue avant son voisin précédent ou après son voisin suivant, dans la liste affichée. Confirmation : « Placer X avant/après Y ».

### Déplacement

`m` : propose les sprints actifs et futurs (hors sprint actuel de l'issue, sans doublon) et « Backlog » si l'issue est dans un sprint. Confirmation : « X → Sprint » ou « X → Backlog ». S'il n'y a ni sprint actif ni futur, un message l'indique.

### Transition

`t` : charge les transitions disponibles dans l'état courant. Celles qui ouvrent un écran ou exigent des champs sont marquées « (écran Jira requis : peut échouer) » : jira-rust ne sait pas remplir ces écrans (voir [400](#400-sur-une-transition)). Confirmation : « clé : transition », « Nouveau statut : … ».

### Création et édition d'issue

`c` ouvre le formulaire « Nouvelle issue » : **Résumé** (obligatoire), **Type** (types hors sous-tâches), **Story points** (nombre positif ; virgule ou point), **Version** (une fixVersion, ou aucune), **Description** (multiligne).

`e` ouvre « Modifier X » : mêmes champs, mais le **type est en lecture seule**. Seuls les champs réellement modifiés sont envoyés ; sans modification, le formulaire répond « aucune modification ». La description n'est modifiable que si elle peut être reconvertie sans perte ; sinon elle est affichée en lecture seule et jamais envoyée. Si l'issue a plusieurs fixVersions, le champ Version est en lecture seule : il affiche leurs noms séparés par des virgules suivis de « (plusieurs versions : modifiables dans Jira uniquement) », ne peut pas être changé et n'est jamais envoyé (les fixVersions existantes restent intactes). Quand la version d'une issue à une seule fixVersion est modifiée, la confirmation affiche « version → » suivi du nom choisi (ou « (aucune) » si elle est retirée), pas de l'identifiant.

### Démarrer un sprint

`s` : refusé si un sprint est déjà actif (« clôturez-le d'abord ») ou s'il n'y a pas de sprint futur. Dans le Backlog, le sprint sous le curseur est utilisé ; sinon, s'il n'y a qu'un sprint futur il est pris d'office, et sinon une liste « Sprint à démarrer » s'affiche. Formulaire : **Sprint** (lecture seule), **Début** (aujourd'hui par défaut), **Fin** (début + 14 jours), **Objectif**. Dates au format `AAAA-MM-JJ`, la fin doit être après le début.

### Clôturer le sprint

`x` : refusé s'il n'y a pas de sprint actif (« Aucun sprint actif »), ou si plusieurs sprints sont actifs en parallèle sur le board (« Sprints actifs en parallèle (A, B) : clôture désactivée, à faire dans Jira »). Seuls les sprints du board courant comptent ; ceux d'autres boards sont ignorés.

- Si toutes les issues sont terminées : confirmation directe.
- Sinon une fenêtre « Clôturer … » indique « N issue(s) terminée(s), M non terminée(s) » et demande la **destination des issues non terminées** : le **Backlog** (présélectionné) ou l'un des **sprints futurs** du board. Puis confirmation, qui rappelle la destination et les clés des issues déplacées.

Exécution : jira-rust déplace d'abord explicitement les issues non terminées vers la destination, puis clôture le sprint. Juste avant d'écrire (même quand toutes les issues semblaient terminées), il relit les issues du sprint : si l'ensemble des issues non terminées a changé depuis votre confirmation (issue terminée, ajoutée ou déplacée entre-temps, par vous ou un collègue), l'opération est abandonnée sans rien écrire (ni déplacement ni clôture) avec un message de la forme « clôture de … abandonnée, rien n'a été modifié : les issues non terminées ont changé depuis la confirmation (en plus : … ; en moins : …). Vérifiez puis recommencez (x) » ; recommencez alors avec `x`. Les sous-tâches ne comptent ni parmi les issues non terminées à déplacer ni parmi les issues terminées affichées : elles suivent leur parent. Si le déplacement échoue, le sprint n'est pas clôturé.

### Versions

- **Créer** (`c` en onglet Versions) : **Nom**, **Début**, **Livraison**, **Description** (dates `AAAA-MM-JJ`, facultatives).
- **Éditer** (`e`, liste focalisée) : **Nom**, **Livraison**, **Description** ; seuls les champs modifiés sont envoyés.
- **Livrer** (`r`) : refusé si la version est déjà livrée, ou si ses issues ne sont pas (encore) chargées (message « R pour réessayer avant de livrer » en cas d'échec). Si toutes les issues sont terminées, confirmation directe. Sinon, la fenêtre « Livrer … » affiche « ≈ N issue(s) non terminée(s) d'après leur statut : clés » et demande la **destination des issues non terminées** : « Aucune (restent dans la version) » par défaut (présélectionnée), ou l'une des autres versions non livrées. Date de livraison : si la version a déjà une date prévue, elle est conservée (rien n'est envoyé pour la date) ; sinon la date du jour est appliquée et la confirmation affiche « Date de livraison : AAAA-MM-JJ ». Le « ≈ N » (repris dans la confirmation : « Jira déplacera les issues non résolues (≈ N d'après leur statut) vers … » ou « … restent dans … ») est une estimation faite par la TUI d'après la catégorie de statut ; c'est Jira qui déplace réellement les issues non résolues, selon sa propre définition.

## Mode lecture seule

`jira-rust --read-only` (indicateur `[LECTURE SEULE]` dans l'en-tête). Vous pouvez tout consulter et ouvrir formulaires et confirmations ; à la validation (une fois le délai de confirmation écoulé), le message « Mode lecture seule : « … » non envoyé à Jira » s'affiche en rouge et rien n'est envoyé. Avec un vrai Jira, le blocage est aussi appliqué dans le client HTTP, qui n'envoie que des lectures (requêtes GET et la recherche JQL, qui est une lecture). `doctor` est toujours en lecture seule. Avec `--mock`, seul le blocage de la TUI s'applique (les données sont en mémoire, sans risque).

Recommandation : commencez toujours une nouvelle configuration par `--read-only`.

## Messages d'erreur courants

Les erreurs s'affichent en rouge dans la ligne de statut, par exemple « Échec — Transition : … ». Les détails complets sont dans le [journal](#logs).

### 401

« authentification refusée (401) : vérifier l'e-mail et le token API — relancer `jira-rust doctor` ». Causes : token erroné, expiré ou révoqué ; e-mail différent de celui du compte ; mauvais `base_url`. Que faire : créer un nouveau token, vérifier `email` et `base_url`, relancer `jira-rust doctor`. Rappel : `JIRA_API_TOKEN` l'emporte sur `api_token` du fichier ; une variable périmée dans votre shell est une cause fréquente.

### 403 / « droits Jira insuffisants ? »

La TUI complète les erreurs du client (fonction `error_text`) :

- 401 : « … — relancer `jira-rust doctor` » (voir plus haut).
- « accès interdit (403) … — droits Jira insuffisants pour cette action » : Jira refuse l'action pour ce compte (certain).
- « … — droits Jira insuffisants ? » (avec point d'interrogation) : Jira a répondu 400 ou 404 avec un message évoquant une permission ; c'est une hypothèse, Jira signalant parfois un droit manquant ainsi (modification de version, transition, etc.).
- Toute autre erreur est affichée telle quelle.

Que faire : vérifier avec un administrateur Jira que votre compte peut modifier ce projet (gérer les sprints, les versions, transitionner ou éditer les issues, selon l'action). En attendant, utilisez `--read-only`.

### 429

« limite de requêtes Jira atteinte (429), réessayer dans N s ». Jira limite le débit. Pour les lectures, le client réessaie automatiquement (jusqu'à 4 tentatives, en respectant `Retry-After`) ; il n'y a **jamais** de nouvel essai automatique sur une écriture. Que faire : patienter le délai indiqué puis refaire l'action (pour une écriture, vérifier d'abord son résultat avec `R`).

### 400 sur une transition

Une transition marquée « (écran Jira requis : peut échouer) » dans la liste des transitions exige des champs (résolution, commentaire, etc.) que jira-rust ne peut pas renseigner. Jira répond alors 400 (ex. « résolution requise »). Que faire : faire cette transition dans Jira, ou utiliser une transition sans écran. L'issue n'est pas modifiée.

### Autres

| Message | Signification / action |
|---|---|
| « configuration introuvable : … » | créer le fichier (voir `jira-rust config-path`) ou passer `--config` |
| « … contient un token mais est lisible par d'autres utilisateurs » | `chmod 600` sur le fichier |
| « base_url doit commencer par https:// » | corriger `base_url` |
| « connexion bloquée … mode hors-ligne actif » | le garde-fou `JIRA_RUST_OFFLINE` est positionné (voir [README](../README.md#développeurs)) : normal dans l'environnement de développement de ce dépôt |
| « erreur réseau : … » | connexion, proxy ou DNS ; vérifier avec `jira-rust doctor` |
| « Aucun board Scrum pour X » | le projet n'a pas de board Scrum (Kanban non géré) |
| « Aucun projet accessible » | le compte ne voit aucun projet |
| « Sprints actifs en parallèle (…) : clôture désactivée, à faire dans Jira » | clôturer dans Jira |
| « clôture de … abandonnée, rien n'a été modifié : … » | les issues non terminées ont changé : vérifier avec `R`, puis refaire `x` |
| « Validation ignorée (trop rapide) : relisez puis confirmez » | attendre un instant et valider à nouveau |
| « Mode lecture seule : « … » non envoyé à Jira » | relancer sans `--read-only` pour écrire |
| « Échec — … » | l'écriture a échoué ; les données sont rafraîchies, vérifier l'état réel (une opération composée peut être partielle) |
| « Chargement du board : … » | échec de lecture ; `R` pour réessayer |

## Logs

Les logs ne sont jamais écrits dans le terminal (il appartient à la TUI) mais dans un fichier :

```
~/.local/share/jira-rust/jira-rust.log
```

(dossier en `0700`). Le fichier est ajouté en continu, sans rotation : supprimez-le de temps en temps. On y trouve le board choisi, chaque écriture confirmée ou refusée (type et clés, jamais le token), les erreurs affichées, etc.

Le niveau se règle avec `JIRA_RUST_LOG` (défaut : `info`) :

```
JIRA_RUST_LOG=debug jira-rust --read-only
```

Les bibliothèques HTTP/TLS restent plafonnées à `warn`. Le token n'apparaît jamais dans les logs.

## Limites connues

- **Pas de sous-tâches dans le backlog** : elles sont exclues des listes Backlog (elles suivent leur parent) et ne comptent pas dans les issues à déplacer à la clôture d'un sprint. On ne peut pas créer de sous-tâche (le type est exclu du formulaire de création).
- **Une seule fixVersion éditable** par issue ; si l'issue en a plusieurs, le champ Version est en lecture seule (modifiables dans Jira uniquement).
- **Pas de création, suppression ni renommage de sprint** dans l'interface (on ne peut que démarrer un sprint futur existant et clôturer le sprint actif). Pas non plus de suppression de version.
- Pas de modification des dates ou de l'objectif d'un sprint déjà créé, hors formulaire de démarrage.
- **Sprints parallèles non gérés** : si plusieurs sprints du board sont actifs en même temps, seul le premier est affiché (avec un avertissement), et la clôture est désactivée (à faire dans Jira). Le démarrage d'un sprint est aussi refusé tant qu'un sprint est actif.
- **Sprints d'autres boards ignorés** : seuls les sprints dont le board d'origine est le board choisi sont affichés et modifiables (Jira en liste parfois d'autres) ; un sprint dont le board d'origine est inconnu est ignoré par prudence.
- **Projets team-managed (gérés par l'équipe) : support en cours, non finalisé.** Le client Jira liste désormais aussi les boards de type `simple` (board des projets team-managed) en plus des boards `scrum`, et tolère l'absence de sprints sur un board `simple` (liste vide). Mais la prise en charge n'est ni terminée ni validée de bout en bout dans la TUI (le sélecteur dit encore « Aucun board Scrum » ; le backlog, les sprints et les versions d'un tel projet n'ont pas été vérifiés). Le support fiable reste celui des projets **company-managed** avec un board Scrum.
- **Pas de board Kanban**, **Jira Cloud uniquement** (ni Server ni Data Center).
- **Backlog approximatif** : calculé à partir du filtre du board ; il peut différer légèrement du backlog affiché par Jira (issues dont le statut est mappé sur la dernière colonne, épopées incluses, etc.).
- La description d'une issue n'est éditable que si elle est convertible sans perte (sinon lecture seule). Pas de commentaires, d'assignation, de priorité ni d'autres champs.
- Les transitions à écran (champs requis) ne sont pas supportées (marquées « écran Jira requis : peut échouer »).
- Le rang ne se change que d'un cran (`K`/`J`), pas vers une position arbitraire.
- Les écritures sont envoyées par lots de 50 (non atomiques au-delà, sans objet avec la TUI actuelle qui n'opère que sur quelques issues à la fois, sauf à la clôture d'un sprint). La clôture d'un sprint est composée (déplacement des issues puis clôture) et donc non atomique : en cas d'échec entre les deux, des issues ont pu être déplacées sans que le sprint soit clôturé.
- Les écritures sont strictement séquentielles (une seule à la fois, les autres en file) ; il n'y a pas d'annulation d'une écriture déjà confirmée.
- Le comptage des issues non terminées à la livraison d'une version (« ≈ N ») est une estimation d'après la catégorie de statut ; Jira décide réellement des issues déplacées.
- Les changements faits par d'autres utilisateurs n'apparaissent qu'après `R` ou après l'une de vos écritures.

## Pour aller plus loin

- [docs/release.md](release.md) : build et installation
- [docs/fake-server.md](fake-server.md) : faux serveur Jira local pour les développeurs
- [docs/api/](api/README.md) : contrats des endpoints Jira
