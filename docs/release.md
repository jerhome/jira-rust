# Release Linux

`jira-rust` est distribué sous forme d'un binaire unique, **statique**
(`x86_64-unknown-linux-musl`), sans dépendance à la glibc ni à OpenSSL : il
tourne sur n'importe quelle distribution Linux x86_64.

## Prérequis (machine de build)

| Outil | Rôle | Installation |
|---|---|---|
| `rustup` + toolchain stable | compilation | <https://rustup.rs> |
| cible musl | édition de liens statique | ajoutée automatiquement par le script (`rustup target add x86_64-unknown-linux-musl`) |
| `musl-gcc` | compile le code C d'`aws-lc-sys` (crypto TLS de rustls) | `sudo apt-get install musl-tools` (Debian/Ubuntu) |
| `file`, `sha256sum` | vérifications, checksum | présents sur la plupart des distributions |

`cmake` et `clang` ne sont **pas** nécessaires : `aws-lc-sys` se compile avec le
builder `cc` intégré.

Sans `musl-gcc`, le script s'arrête. L'option `--allow-gcc-fallback` utilise
alors le `gcc` de l'hôte (en-têtes glibc) : cela fonctionne sur Debian 12, mais
n'est pas recommandé pour une version publiée. Le script le signale par des
lignes `!!!` au début et à la fin de la sortie.

## Build

```sh
scripts/build-linux.sh
```

Le script :

0. option `--check-only` : ne compile rien et rejoue seulement les contrôles
   (étape 3) sur le binaire déjà présent dans `dist/` ;
1. vérifie les outils et ajoute la cible musl si besoin ;
2. compile en profil `release` (`opt-level = "s"`, LTO, `codegen-units = 1`,
   `strip`) avec `--locked`, chemins absolus neutralisés (`--remap-path-prefix`)
   et `SOURCE_DATE_EPOCH` fixé sur le dernier commit ;
3. contrôles bloquants : binaire statique (`file`, `ldd`), `--version` égal à
   la version de `Cargo.toml`, `--help` listant `--read-only` et `--mock`,
   `--mock doctor` réussi, aucune chaîne `/home` dans le binaire (aucun accès
   réseau) ;
4. produit `dist/jira-rust-<version>-x86_64-linux-musl` et son fichier
   `.sha256`.

Durée indicative : ~2 min 30 (build à froid), binaire d'environ 5,2 Mo.

## Installation

```sh
cd dist
sha256sum -c jira-rust-0.1.0-x86_64-linux-musl.sha256
install -Dm755 jira-rust-0.1.0-x86_64-linux-musl ~/.local/bin/jira-rust
jira-rust --version
```

Vérifier que `~/.local/bin` est dans le `PATH` (sinon l'ajouter dans le profil
du shell, par ex. `fish_add_path ~/.local/bin` ou
`export PATH="$HOME/.local/bin:$PATH"`).

## Configuration

```sh
jira-rust config-path
```

affiche l'emplacement du fichier (par défaut `~/.config/jira-rust/config.toml`)
et un exemple de contenu :

```toml
base_url = "https://<votre-site>.atlassian.net"
email    = "prenom.nom@entreprise.com"
# api_token = "..."   # ou variable d'environnement JIRA_API_TOKEN (recommandé)
```

Si le token est stocké dans le fichier : `chmod 600 ~/.config/jira-rust/config.toml`
(le programme refuse un fichier lisible par d'autres utilisateurs).

Essai sans Jira : `jira-rust --mock` (données fictives). Diagnostic réel :
`jira-rust doctor`.

## Premier lancement sur le Jira réel

Le garde-fou hors-ligne des builds de développement n'est **pas** actif dans
le binaire release : c'est l'usage normal, il parle au Jira configuré. Seul
`cargo run` reste bridé au loopback.

1. **Token dédié** : créer un token API réservé à cet outil sur
   <https://id.atlassian.com/manage-profile/security/api-tokens>
   (« Create API token », nom explicite, expiration courte).
2. **Stockage** : soit `export JIRA_API_TOKEN=...` dans la session, soit
   `api_token` dans `config.toml` avec `chmod 600`. Ne jamais coller le token
   dans un chat, un ticket ou un message de support.
3. `jira-rust config-path` : affiche où créer/éditer le fichier
   (`base_url`, `email`).
4. `jira-rust doctor` : diagnostic de la connexion et de l'authentification.
5. **Premier lancement en lecture seule** : `jira-rust --read-only`. N'utiliser
   le mode normal qu'après ces vérifications.
6. À vérifier à l'écran :
   - la liste des projets attendus ;
   - le board correct pour le projet choisi ;
   - le backlog, **sans doublons** (une même clé n'apparaît qu'une fois) ;
   - le sprint actif et ses tickets ;
   - les versions (releases) du projet ;
   - les story points cohérents avec Jira.
7. **Révoquer le token** (fin de test, fuite suspectée) : même page
   <https://id.atlassian.com/manage-profile/security/api-tokens>, bouton
   « Revoke » ; puis retirer `JIRA_API_TOKEN` / `api_token`.
8. **Journaux** : `~/.local/share/jira-rust/jira-rust.log`. En cas de souci,
   envoyer : la sortie de `jira-rust --version` et `jira-rust doctor`, les
   dernières lignes du journal, et la description de ce qui est affiché.
   Vérifier avant envoi qu'aucun token ni `Authorization` n'y figure, et ne
   jamais envoyer `config.toml`.

## Désinstallation

```sh
rm ~/.local/bin/jira-rust
rm -r ~/.config/jira-rust          # configuration (contient éventuellement le token)
rm -r ~/.local/share/jira-rust     # journaux, s'ils existent
```
