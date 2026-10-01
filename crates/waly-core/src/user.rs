//! Identité de l'utilisateur — CONFIGURABLE, jamais en dur dans le code.
//!
//! Waly est un projet open source : chacun l'installe pour soi. Le prénom de
//! l'utilisateur vient de la variable d'environnement `WALY_USER` (posée par
//! l'installeur, un script de lancement, ou la main de l'utilisateur). Sans
//! elle, les prompts parlent de « ton utilisateur » — neutre et correct.

/// Prénom configuré, s'il existe (`WALY_USER` › waly.toml `[utilisateur] nom`,
/// espaces retirés, vide = absent).
pub fn nom() -> Option<String> {
    std::env::var("WALY_USER")
        .ok()
        .or_else(|| crate::config::valeur("utilisateur", "nom"))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Désignation de l'utilisateur pour les prompts : son prénom si configuré,
/// sinon « ton utilisateur » (les prompts parlent de lui à la 3ᵉ personne).
pub fn designation() -> String {
    nom().unwrap_or_else(|| "ton utilisateur".to_string())
}

#[cfg(test)]
mod tests {
    // Pas de test sur la variable d'environnement (les tests cargo tournent en
    // parallèle dans un même processus — set_var y ferait une course) : on
    // teste la logique de repli seule.
    #[test]
    fn designation_sans_nom_est_neutre() {
        // En environnement de test, WALY_USER n'est pas posée.
        if std::env::var("WALY_USER").is_err() {
            assert_eq!(super::designation(), "ton utilisateur");
        }
    }
}
