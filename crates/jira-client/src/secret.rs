use std::fmt;

/// Chaîne secrète (token API). Jamais affichée : `Debug` est masqué et `Display` n'est pas implémenté.
#[derive(Clone, PartialEq, Eq)]
pub struct SecretString(String);

impl SecretString {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Accès explicite à la valeur, réservé à la construction de l'en-tête d'authentification.
    pub fn expose(&self) -> &str {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.trim().is_empty()
    }
}

impl fmt::Debug for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretString(***)")
    }
}

impl From<String> for SecretString {
    fn from(value: String) -> Self {
        Self(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_is_masked() {
        let s = SecretString::new("super-secret");
        assert_eq!(format!("{s:?}"), "SecretString(***)");
        assert!(!format!("{s:?}").contains("super-secret"));
    }
}
