# jira-rust

Une TUI (terminal) en Rust, basée sur [ratatui](https://ratatui.rs), pour gérer le **backlog**, le **sprint actif** et les **versions** d'un projet **Scrum** sur **Jira Cloud**.

```
 1 Backlog  2 Sprint actif  3 Versions                              DEMO · Tableau DEMO [LECTURE SEULE]
┌ Backlog — sprints futurs + backlog · 34 pts ──────────────────────────────────────────────────┐
│ ▼ Sprint 6  3 issue(s) · 13 pts                                                                │
│ ›   DEMO-12  Story  À faire   Page de connexion                                    5           │
│     DEMO-14  Bug    À faire   Erreur 500 à l'export                                3           │
│ ▼ Backlog  8 issue(s) · 21 pts                                                                 │
└────────────────────────────────────────────────────────────────────────────────────────────────┘
 j/k · Entrée détail/replier · t transition · m déplacer · K/J rang · c créer · e éditer · s démarrer sprint · ? aide
```

(Schéma indicatif : l'affichage réel diffère dans le détail.)

## Périmètre

- Projets **Scrum** sur **Jira Cloud** (REST API v3 + Jira Software Agile 1.0), authentification par e-mail + token API.
- **Backlog** : sprints futurs et issues hors sprint, changement de rang, déplacement, transitions, création et édition d'issues.
- **Sprint actif** : colonnes À faire / En cours / Terminé, démarrage et clôture de sprint.
- **Versions** : liste, issues par version, création, édition, livraison.

Hors périmètre : Kanban, Jira Server/Data Center, création ou suppression de sprint ou de version, sprints actifs en parallèle (un seul est géré). Projets team-managed : support en cours, non finalisé. Détails dans [docs/usage.md](docs/usage.md#limites-connues).

## Installation

Binaire Linux statique x86_64 : voir [docs/release.md](docs/release.md) (build avec `scripts/build-linux.sh`, puis installation dans `~/.local/bin`).

## Démarrage rapide

1. Créer un token API : <https://id.atlassian.com/manage-profile/security/api-tokens>.
2. Écrire la configuration. `jira-rust config-path` affiche le chemin (par défaut `~/.config/jira-rust/config.toml`) et un exemple :

   ```toml
   base_url = "https://<votre-site>.atlassian.net"
   email    = "prenom.nom@entreprise.com"
   ```

   Le token se donne de préférence par la variable d'environnement `JIRA_API_TOKEN` (sinon `api_token = "..."` dans le fichier, qui doit alors être en `chmod 600`).
3. Vérifier : `jira-rust doctor` (lit `/myself`, n'écrit rien).
4. **Commencer en lecture seule** : `jira-rust --read-only`. Aucune écriture n'est envoyée à Jira.
5. Une fois à l'aise : `jira-rust`.

Pour essayer sans Jira ni réseau : `jira-rust --mock` (données fictives en mémoire). `?` ouvre l'aide des raccourcis, `q` quitte.

## Sécurité

- **Chaque écriture est confirmée** dans une fenêtre qui détaille ce qui sera envoyé (Entrée/o/y pour valider, Esc/n/q pour annuler). La confirmation n'est validable qu'une fois affichée et laissée 0,4 s sans frappe : une validation trop rapide (frappe répétée, collage) est ignorée.
- **Collage sans risque** : le texte collé dans un formulaire est inséré tel quel et ne valide jamais rien.
- **Clôture de sprint sûre** : le sprint est relu juste avant la clôture ; si ses issues non terminées ont changé, l'opération est abandonnée sans rien écrire.
- **Mode lecture seule** (`--read-only`) : les écritures sont refusées avant tout envoi (dans le client HTTP, et aussi à la confirmation dans la TUI).
- Le **token n'est jamais affiché ni journalisé**. Un fichier de configuration contenant un token lisible par d'autres utilisateurs est refusé.
- `http://` n'est accepté que pour `localhost`, `127.0.0.1` et `::1`.

## Documentation

- [docs/usage.md](docs/usage.md) : guide d'utilisation complet (configuration, raccourcis, écritures, erreurs, logs, limites).
- [docs/release.md](docs/release.md) : build et installation.
- [docs/fake-server.md](docs/fake-server.md) : faux serveur Jira local (développeurs).
- [docs/api/](docs/api/README.md) : contrats des endpoints Jira utilisés.

## Développeurs

Workspace Cargo :

| Crate | Rôle |
|---|---|
| `crates/jira-client` | client REST Jira Cloud, modèles, trait `JiraApi`, `MockJira` |
| `crates/jira-tui` | l'application (binaire `jira-rust`) |
| `crates/jira-fake` | faux serveur Jira local sur 127.0.0.1 pour les tests d'intégration |

```sh
cargo test --workspace
```

`.cargo/config.toml` positionne `JIRA_RUST_OFFLINE=1` pour toute commande `cargo` : le client refuse alors tout hôte non local, seul le loopback (wiremock, `jira-fake`) est joignable. Ne contournez ce garde-fou que volontairement, pour un test réel autorisé.
