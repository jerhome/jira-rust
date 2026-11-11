//! Configuration : `~/.config/jira-rust/config.toml` + variable `JIRA_API_TOKEN`.
//!
//! Le token peut être dans le fichier (qui doit alors être en 0600) ou dans `JIRA_API_TOKEN`
//! (prioritaire). Il n'est jamais affiché.

use std::path::{Path, PathBuf};

use color_eyre::eyre::{Result, WrapErr, bail, eyre};
use jira_client::SecretString;
use serde::Deserialize;

pub const TOKEN_ENV: &str = "JIRA_API_TOKEN";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileConfig {
    base_url: String,
    email: String,
    api_token: Option<String>,
}

#[derive(Debug)]
pub struct Config {
    pub path: PathBuf,
    pub base_url: String,
    pub email: String,
    pub token: SecretString,
    pub token_source: TokenSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenSource {
    Env,
    File,
}

pub fn default_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "jira-rust").map(|d| d.config_dir().join("config.toml"))
}

fn resolve_path(explicit: Option<&Path>) -> Result<PathBuf> {
    explicit
        .map(Path::to_path_buf)
        .or_else(default_path)
        .ok_or_else(|| eyre!("impossible de déterminer le dossier de configuration"))
}

pub fn load(explicit: Option<&Path>) -> Result<Config> {
    let path = resolve_path(explicit)?;
    let text = std::fs::read_to_string(&path).wrap_err_with(|| {
        format!(
            "configuration introuvable : {} (voir `jira-rust config-path`)",
            path.display()
        )
    })?;
    let env_token = std::env::var(TOKEN_ENV).ok();
    parse(&path, &text, env_token, file_mode(&path))
}

fn parse(path: &Path, text: &str, env_token: Option<String>, mode: Option<u32>) -> Result<Config> {
    let file: FileConfig = toml::from_str(text).map_err(|e| {
        eyre!(
            "configuration invalide ({}) : {}",
            path.display(),
            e.message()
        )
    })?;

    let base_url = file.base_url.trim().trim_end_matches('/').to_owned();
    if !base_url.starts_with("https://") && !is_local_http(&base_url) {
        bail!(
            "base_url doit commencer par https:// (http:// n'est accepté que pour localhost) : {base_url}"
        );
    }
    let email = file.email.trim().to_owned();
    if email.is_empty() {
        bail!("email manquant dans {}", path.display());
    }

    let file_token = file.api_token.filter(|t| !t.trim().is_empty());
    // Un fichier contenant un token doit être privé, même si JIRA_API_TOKEN est prioritaire.
    if file_token.is_some() {
        match mode {
            Some(mode) if mode & 0o077 != 0 => bail!(
                "{} contient un token mais est lisible par d'autres utilisateurs (mode {:o}). \
                 Corriger avec : chmod 600 {}",
                path.display(),
                mode & 0o777,
                path.display()
            ),
            None if cfg!(unix) => bail!(
                "impossible de vérifier les permissions de {}",
                path.display()
            ),
            _ => {}
        }
    }

    let (token, token_source) = match (env_token.filter(|t| !t.trim().is_empty()), file_token) {
        (Some(t), _) => (SecretString::new(t.trim()), TokenSource::Env),
        (None, Some(t)) => (SecretString::new(t.trim()), TokenSource::File),
        _ => bail!(
            "token API manquant : définir {TOKEN_ENV} ou api_token dans {}",
            path.display()
        ),
    };

    Ok(Config {
        path: path.to_path_buf(),
        base_url,
        email,
        token,
        token_source,
    })
}

fn is_local_http(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("http://") else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let host = match host.strip_prefix('[') {
        Some(v6) => v6.split_once(']').map_or(v6, |(h, _)| h),
        None => host.split(':').next().unwrap_or(host),
    };
    matches!(
        host.to_ascii_lowercase().as_str(),
        "localhost" | "127.0.0.1" | "::1"
    )
}

#[cfg(unix)]
fn file_mode(path: &Path) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).ok().map(|m| m.permissions().mode())
}

#[cfg(not(unix))]
fn file_mode(_path: &Path) -> Option<u32> {
    None
}

pub fn print_help(explicit: Option<&Path>) {
    match resolve_path(explicit) {
        Ok(p) => println!("Fichier de configuration : {}", p.display()),
        Err(e) => println!("{e}"),
    }
    println!(
        r#"
Exemple de contenu :

    base_url = "https://<votre-site>.atlassian.net"
    email    = "prenom.nom@entreprise.com"
    # api_token = "..."   # ou variable d'environnement {TOKEN_ENV} (recommandé)

Si le token est dans le fichier : chmod 600 <fichier>
Token API : https://id.atlassian.com/manage-profile/security/api-tokens"#
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = "base_url = \"https://demo.example.com/\"\nemail = \"a@b.c\"\n";

    fn p() -> &'static Path {
        Path::new("/tmp/config.toml")
    }

    #[test]
    fn env_token_wins_and_url_is_trimmed() {
        let text = format!("{BASE}api_token = \"file\"\n");
        let c = parse(p(), &text, Some("env".into()), Some(0o600)).unwrap();
        assert_eq!(c.token.expose(), "env");
        assert_eq!(c.token_source, TokenSource::Env);
        assert_eq!(c.base_url, "https://demo.example.com");
    }

    #[test]
    fn file_token_requires_private_permissions() {
        let text = format!("{BASE}api_token = \"file\"\n");
        let err = parse(p(), &text, None, Some(0o100644)).unwrap_err();
        assert!(err.to_string().contains("chmod 600"));
        assert!(!err.to_string().contains("file\""));
        let c = parse(p(), &text, None, Some(0o100600)).unwrap();
        assert_eq!(c.token_source, TokenSource::File);
    }

    #[test]
    fn file_token_checked_even_when_env_token_wins() {
        let text = format!("{BASE}api_token = \"file\"\n");
        assert!(parse(p(), &text, Some("env".into()), Some(0o100644)).is_err());
        assert_eq!(
            parse(p(), &text, Some("env".into()), None).is_err(),
            cfg!(unix)
        );
    }

    #[test]
    fn plain_http_only_for_localhost() {
        let t = || Some("t".to_string());
        let cfg = |u: &str| format!("base_url = \"{u}\"\nemail = \"a@b.c\"\n");
        assert!(parse(p(), &cfg("http://demo.example.com"), t(), None).is_err());
        assert!(parse(p(), &cfg("http://localhost.example.com"), t(), None).is_err());
        assert!(parse(p(), &cfg("http://127.0.0.1:8080"), t(), None).is_ok());
        assert!(parse(p(), &cfg("http://[::1]:8080/jira"), t(), None).is_ok());
        assert!(parse(p(), &cfg("http://localhost"), t(), None).is_ok());
    }

    #[test]
    fn missing_token_is_an_error() {
        assert!(parse(p(), BASE, None, Some(0o600)).is_err());
        assert!(parse(p(), BASE, Some("  ".into()), Some(0o600)).is_err());
    }

    #[test]
    fn invalid_content() {
        assert!(parse(p(), "email = \"a@b.c\"", Some("t".into()), None).is_err());
        assert!(
            parse(
                p(),
                "base_url = \"demo\"\nemail = \"a\"",
                Some("t".into()),
                None
            )
            .is_err()
        );
        assert!(parse(p(), &format!("{BASE}inconnu = 1"), Some("t".into()), None).is_err());
    }

    #[test]
    fn debug_does_not_leak_token() {
        let c = parse(p(), BASE, Some("tres-secret".into()), None).unwrap();
        assert!(!format!("{c:?}").contains("tres-secret"));
    }
}
