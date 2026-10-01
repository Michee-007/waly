//! Chemins de Waly — UNE source de vérité, par plateforme (portage Linux,
//! `docs/PLAN-2026-09-11-R-L-portage-linux.md`, chantier 0).
//!
//! Racine : `WALY_HOME` › Windows `C:\waly` (INCHANGÉ — machine de
//! référence) › Linux/macOS `$XDG_DATA_HOME/waly` › `$HOME/.local/share/waly`.
//! Sous la racine, la même arborescence partout :
//! `data/` (base, waly.toml, journaux), `engines/` (`models/`, `sherpa/lib/`,
//! `voices-fr/`), `bin/` (exécutables). Les bibliothèques natives prennent le
//! nom de la plateforme (`onnxruntime.dll` / `libonnxruntime.so` / `.dylib`).

use std::path::PathBuf;

fn var(nom: &str) -> Option<String> {
    std::env::var(nom).ok().filter(|v| !v.trim().is_empty())
}

/// Racine de CE processus.
pub fn racine() -> PathBuf {
    racine_pour(std::env::consts::OS, var("WALY_HOME"), var("XDG_DATA_HOME"), var("HOME"))
}

/// Règle PURE (testable sur toute plateforme).
pub fn racine_pour(
    os: &str,
    waly_home: Option<String>,
    xdg_data_home: Option<String>,
    home: Option<String>,
) -> PathBuf {
    if let Some(h) = waly_home {
        return PathBuf::from(h);
    }
    if os == "windows" {
        return PathBuf::from(r"C:\waly");
    }
    if let Some(x) = xdg_data_home {
        return PathBuf::from(x).join("waly");
    }
    PathBuf::from(home.unwrap_or_else(|| ".".into())).join(".local").join("share").join("waly")
}

/// `base` + chemin relatif écrit avec `/` (ou `\`), séparateurs natifs.
fn sous(base: PathBuf, rel: &str) -> PathBuf {
    rel.split(['/', '\\']).filter(|c| !c.is_empty()).fold(base, |p, c| p.join(c))
}

fn texte(p: PathBuf) -> String {
    p.to_string_lossy().into_owned()
}

pub fn data() -> PathBuf {
    racine().join("data")
}

pub fn engines() -> PathBuf {
    racine().join("engines")
}

pub fn modeles() -> PathBuf {
    engines().join("models")
}

pub fn bin() -> PathBuf {
    racine().join("bin")
}

/// Fichier sous `data/` (ex. `waly.db`, `waly.toml`, `waly-voice.log`).
pub fn data_fichier(rel: &str) -> String {
    texte(sous(data(), rel))
}

/// Fichier sous `engines/` (ex. `voices-fr/fabien.wav`).
pub fn engines_fichier(rel: &str) -> String {
    texte(sous(engines(), rel))
}

/// Fichier ou dossier de modèle sous `engines/models/`.
pub fn modele(rel: &str) -> String {
    texte(sous(modeles(), rel))
}

/// Nom de bibliothèque native de la plateforme (`onnxruntime` →
/// `onnxruntime.dll` / `libonnxruntime.so` / `libonnxruntime.dylib`).
pub fn nom_lib(base: &str) -> String {
    format!("{}{base}{}", std::env::consts::DLL_PREFIX, std::env::consts::DLL_SUFFIX)
}

/// Exécutable Waly sous `bin/` (`waly-voice` → `waly-voice.exe` sur Windows).
pub fn exe(nom: &str) -> String {
    texte(bin().join(format!("{nom}{}", std::env::consts::EXE_SUFFIX)))
}

/// Bibliothèque de la distribution sherpa-onnx (`engines/sherpa/lib/`).
pub fn lib_sherpa(base: &str) -> String {
    texte(engines().join("sherpa").join("lib").join(nom_lib(base)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn racine_par_plateforme() {
        let s = |v: &str| Some(v.to_string());
        // Machine de référence : inchangé.
        assert_eq!(racine_pour("windows", None, None, s("C:\\Users\\x")), PathBuf::from(r"C:\waly"));
        // WALY_HOME prime partout.
        assert_eq!(racine_pour("windows", s("D:\\w"), None, None), PathBuf::from("D:\\w"));
        assert_eq!(racine_pour("linux", s("/opt/waly"), s("/x"), s("/home/u")), PathBuf::from("/opt/waly"));
        // Linux / macOS : XDG puis ~/.local/share.
        assert_eq!(racine_pour("linux", None, s("/x/data"), s("/home/u")), PathBuf::from("/x/data").join("waly"));
        assert_eq!(
            racine_pour("macos", None, None, s("/Users/u")),
            PathBuf::from("/Users/u").join(".local").join("share").join("waly")
        );
    }

    #[test]
    fn sous_chemins_separateurs_natifs() {
        let p = sous(PathBuf::from("base"), "yunet/face.onnx");
        assert_eq!(p, PathBuf::from("base").join("yunet").join("face.onnx"));
        assert_eq!(sous(PathBuf::from("b"), "a\\b//c"), PathBuf::from("b").join("a").join("b").join("c"));
    }

    #[test]
    fn noms_natifs_de_la_plateforme() {
        #[cfg(windows)]
        {
            assert_eq!(nom_lib("onnxruntime"), "onnxruntime.dll");
            assert!(exe("waly-voice").ends_with("waly-voice.exe"));
        }
        #[cfg(target_os = "linux")]
        {
            assert_eq!(nom_lib("onnxruntime"), "libonnxruntime.so");
            assert!(exe("waly-voice").ends_with("waly-voice"));
        }
        assert!(lib_sherpa("sherpa-onnx-c-api").contains("sherpa-onnx-c-api"));
    }
}
