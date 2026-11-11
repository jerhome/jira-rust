//! Garde-fou contre toute connexion involontaire à une instance Jira réelle.
//!
//! Mode hors-ligne = seules les adresses locales (localhost / loopback, ex. wiremock) sont autorisées.
//! Il est actif :
//! - si `JIRA_RUST_OFFLINE=1` (positionné pour toutes les commandes cargo via `.cargo/config.toml`) ;
//! - par défaut dans les builds debug, sauf `JIRA_RUST_OFFLINE=0` explicite.
//!
//! Le binaire release lancé directement n'est pas bridé (usage normal par l'utilisateur).

use url::{Host, Url};

use crate::{Error, Result};

pub const OFFLINE_ENV: &str = "JIRA_RUST_OFFLINE";

pub(crate) fn check(url: &Url) -> Result<()> {
    let env = std::env::var(OFFLINE_ENV).ok();
    check_with(url, offline_enabled(env.as_deref(), cfg!(debug_assertions)))
}

/// Fail-closed : seule la valeur exacte `0` désactive le mode hors-ligne ; toute autre valeur l'active.
fn offline_enabled(env: Option<&str>, debug_build: bool) -> bool {
    match env.map(str::trim) {
        Some("0") => false,
        Some(_) => true,
        None => debug_build,
    }
}

fn check_with(url: &Url, offline: bool) -> Result<()> {
    if !offline || is_loopback(url) {
        return Ok(());
    }
    Err(Error::OfflineGuard {
        host: url.host_str().unwrap_or("?").to_owned(),
        env: OFFLINE_ENV,
    })
}

pub(crate) fn is_loopback(url: &Url) -> bool {
    match url.host() {
        Some(Host::Domain(d)) => d.eq_ignore_ascii_case("localhost"),
        Some(Host::Ipv4(ip)) => ip.is_loopback(),
        Some(Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    #[test]
    fn offline_blocks_remote_hosts() {
        assert!(check_with(&u("https://example.invalid/"), true).is_err());
        assert!(check_with(&u("https://demo.example.com/"), true).is_err());
    }

    #[test]
    fn offline_allows_loopback() {
        assert!(check_with(&u("http://127.0.0.1:8080/"), true).is_ok());
        assert!(check_with(&u("http://localhost:1234/"), true).is_ok());
        assert!(check_with(&u("http://[::1]:1234/"), true).is_ok());
    }

    #[test]
    fn online_allows_everything() {
        assert!(check_with(&u("https://demo.example.com/"), false).is_ok());
    }

    #[test]
    fn offline_flag_resolution() {
        assert!(offline_enabled(Some("1"), false));
        assert!(!offline_enabled(Some("0"), true));
        assert!(offline_enabled(None, true));
        assert!(!offline_enabled(None, false));
        assert!(offline_enabled(Some("oui"), false));
        assert!(offline_enabled(Some("true"), false));
        assert!(offline_enabled(Some(""), false));
    }
}
