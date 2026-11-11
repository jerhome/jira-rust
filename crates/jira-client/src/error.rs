use std::time::Duration;

pub type Result<T> = std::result::Result<T, Error>;

/// Erreurs du client Jira. Aucun message ne contient le token.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("URL Jira invalide : {0}")]
    InvalidUrl(String),

    #[error(
        "connexion bloquée vers {host} : mode hors-ligne actif ({env}). \
         Seules les adresses locales (wiremock) sont autorisées."
    )]
    OfflineGuard { host: String, env: &'static str },

    #[error("authentification refusée (401) : vérifier l'e-mail et le token API")]
    Unauthorized,

    #[error("accès interdit (403) : {0}")]
    Forbidden(String),

    #[error("ressource introuvable (404) : {0}")]
    NotFound(String),

    #[error("limite de requêtes Jira atteinte (429){}", retry_after.map(|d| format!(", réessayer dans {} s", d.as_secs())).unwrap_or_default())]
    RateLimited { retry_after: Option<Duration> },

    #[error("erreur Jira {status} : {message}")]
    Api { status: u16, message: String },

    #[error("erreur réseau : {0}")]
    Transport(String),

    #[error("réponse Jira illisible : {0}")]
    Decode(String),

    #[error("fonction non encore implémentée : {0}")]
    NotImplemented(&'static str),

    #[error("requête invalide : {0}")]
    Invalid(String),

    /// Écriture refusée localement : le client est en lecture seule (aucune requête envoyée).
    #[error("mode lecture seule : écriture refusée ({0})")]
    ReadOnly(String),
}

impl Error {
    /// `true` si l'erreur peut provenir d'un droit manquant : la TUI peut alors afficher un indice
    /// (« vérifier vos permissions Jira »).
    ///
    /// Jira ne renvoie pas toujours 403 pour un défaut de permission : 400 sur `PUT /version/{id}` et
    /// `POST /issue/{key}/transitions`, 404 sur `POST /version/{id}/removeAndSwap` (et la plupart des
    /// 404 disent « does not exist or you do not have permission »). Règle : toujours vrai pour
    /// 401/403 ; pour 400/404, vrai si le message évoque une permission (« permission »,
    /// « not have permission », « droits », insensible à la casse). Heuristique : un 404 Jira
    /// générique (« ... or you do not have permission to see it ») renvoie aussi `true`.
    pub fn maybe_permission(&self) -> bool {
        fn mentions_permission(message: &str) -> bool {
            let m = message.to_lowercase();
            ["permission", "not have permission", "droits"]
                .iter()
                .any(|k| m.contains(k))
        }
        match self {
            Self::Unauthorized | Self::Forbidden(_) => true,
            Self::NotFound(message) => mentions_permission(message),
            Self::Api {
                status: 400 | 404,
                message,
            } => mentions_permission(message),
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maybe_permission_rules() {
        assert!(Error::Unauthorized.maybe_permission());
        assert!(Error::Forbidden(String::new()).maybe_permission());
        assert!(
            Error::Api {
                status: 400,
                message: "You do not have PERMISSION to edit versions.".into()
            }
            .maybe_permission()
        );
        assert!(
            Error::Api {
                status: 400,
                message: "Vous n'avez pas les Droits nécessaires".into()
            }
            .maybe_permission()
        );
        assert!(
            Error::NotFound("Version does not exist or you do not have permission".into())
                .maybe_permission()
        );
        assert!(
            !Error::Api {
                status: 400,
                message: "name : A version with this name already exists".into()
            }
            .maybe_permission()
        );
        assert!(
            !Error::NotFound("Could not find version for id '10199'".into()).maybe_permission()
        );
        // Statut hors 400/404 : jamais, même si le message en parle.
        assert!(
            !Error::Api {
                status: 500,
                message: "permission".into()
            }
            .maybe_permission()
        );
        assert!(!Error::RateLimited { retry_after: None }.maybe_permission());
        assert!(!Error::Invalid("permission".into()).maybe_permission());
    }
}
