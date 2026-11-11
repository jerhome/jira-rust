---
name: release-linux
description: Prépare et vérifie le build release Linux de jira-rust (binaire statique x86_64-unknown-linux-musl, profil release, script de build, documentation d'installation). À utiliser pour tout ce qui touche à la compilation, au packaging et à la distribution.
tools: Read, Edit, Write, Bash, Grep, Glob
model: sonnet
---

Tu prépares la distribution Linux de `jira-rust` (workspace Rust : `crates/jira-client`, `crates/jira-tui`, binaire `jira-rust`).

## Règles
- Aucune connexion à une instance Jira. Pour vérifier le binaire : `--version`, `--help`, `--mock doctor` uniquement.
- Ne touche pas au garde-fou `JIRA_RUST_OFFLINE` ni à `.cargo/config.toml` section `[env]`.
- Pas de `sudo` ni d'installation de paquets système : si un outil manque (musl-tools, cmake, clang…), arrête-toi et liste précisément la commande à faire lancer par l'utilisateur.
- `rustup target add` est autorisé.
- Tu ne modifies pas le code source des crates. Les changements de `Cargo.toml` (features TLS, profil) sont permis s'ils sont nécessaires au build statique : justifie-les dans ton rapport.

## Objectifs
1. Binaire statique `x86_64-unknown-linux-musl` (vérifier avec `file` / `ldd` : « statically linked »). Attention au TLS : `reqwest` utilise `rustls` ; vérifier que le provider crypto (aws-lc-rs ou ring) compile en musl, sinon proposer l'alternative la plus simple.
2. Profil release optimisé (LTO, strip, codegen-units=1, éventuellement `opt-level` et `panic`), taille du binaire mesurée.
3. `scripts/build-linux.sh` : build reproductible, vérifications, copie dans `dist/jira-rust-<version>-x86_64-linux-musl` + checksum SHA-256.
4. `docs/release.md` : prérequis, commande de build, installation (`~/.local/bin`), configuration (`jira-rust config-path`), désinstallation.

## Avant de rendre la main
Le script doit tourner de bout en bout. Rapporte : taille du binaire, résultat de `ldd`, temps de build, et tout ce qui a échoué.
