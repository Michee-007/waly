//! Télécharger d'autres modèles (lot 3 des « bientôt », 2026-10-01).
//!
//! Waly ne télécharge RIEN lui-même : il est scellé (huis clos). Il DEMANDE
//! au moteur local (Ollama, loopback) de tirer le modèle — c'est le moteur,
//! hors du périmètre scellé, qui ouvre la sortie vers son registre. L'app le
//! dit et l'inscrit au journal du sceau (la sortie se VOIT).
//!
//! Le catalogue est court et vérifié à la source (ollama.com/library, tailles
//! relues le 2026-10-01) ; tout autre nom Ollama se tire par le champ libre.
//! Le verdict « tient bien / lent / trop gros » est une règle de mémoire
//! calée sur la machine de référence (4B fluide, 8B lent, 20B impossible).

use serde::Serialize;

use crate::llm::LlmClient;
use crate::materiel::Profil;

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Entree {
    pub nom: &'static str,
    pub taille_go: f32,
    pub outils: bool,
    pub vision: bool,
    pub reflexion: bool,
    pub note: &'static str,
}

pub const CATALOGUE: &[Entree] = &[
    Entree {
        nom: "qwen3:4b-instruct-2507-q4_K_M",
        taille_go: 2.5,
        outils: true,
        vision: false,
        reflexion: false,
        note: "Le cerveau de référence de Waly : outils, bon français, ne « pense » pas (rapide).",
    },
    Entree {
        nom: "llama3.2:3b",
        taille_go: 2.0,
        outils: true,
        vision: false,
        reflexion: false,
        note: "Le plus léger. Outils parfois écrits en texte (Waly les rattrape).",
    },
    Entree {
        nom: "gemma3:4b",
        taille_go: 3.3,
        outils: false,
        vision: true,
        reflexion: false,
        note: "Voit les images, sans outils : à déclarer comme modèle vision d'un cerveau qui ne voit pas.",
    },
    Entree {
        nom: "qwen3-vl:4b-instruct",
        taille_go: 3.3,
        outils: true,
        vision: true,
        reflexion: false,
        note: "Un seul cerveau qui parle, outille et voit.",
    },
    Entree {
        nom: "qwen3:8b",
        taille_go: 5.2,
        outils: true,
        vision: false,
        reflexion: true,
        note: "Plus juste que le 4B, raisonne nativement (réflexion approfondie).",
    },
    Entree {
        nom: "qwen3-vl:8b-instruct",
        taille_go: 6.1,
        outils: true,
        vision: true,
        reflexion: false,
        note: "Le 8B qui voit : pour une carte graphique dédiée.",
    },
    Entree {
        nom: "mistral-nemo:12b",
        taille_go: 7.1,
        outils: true,
        vision: false,
        reflexion: false,
        note: "Mistral 12B, très à l'aise en français.",
    },
    Entree {
        nom: "gemma3:12b",
        taille_go: 8.1,
        outils: false,
        vision: true,
        reflexion: false,
        note: "Vision plus fine, sans outils.",
    },
    Entree {
        nom: "gpt-oss:20b",
        taille_go: 14.0,
        outils: true,
        vision: false,
        reflexion: true,
        note: "20B qui raisonne : pour 24 Go de mémoire ou une carte de 16 Go.",
    },
    Entree {
        nom: "mistral-small3.2:24b",
        taille_go: 15.0,
        outils: true,
        vision: true,
        reflexion: false,
        note: "Mistral 24B qui voit : pour une carte de 16-24 Go.",
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    TientBien,
    Lent,
    TropGros,
}

/// Mémoire qu'un modèle occupe une fois chargé : ses poids + contexte et
/// calcul (mesuré : qwen3 4B, 2,5 Go sur disque → 3,2 Go chargés à 8k).
pub fn besoin_go(taille_go: f32) -> f32 {
    taille_go * 1.15 + 0.8
}

/// Verdict pour une machine. Carte dédiée : tout tient dans sa mémoire →
/// fluide ; sinon la RAM décide, et elle se partage avec le reste du système
/// (35 % = fluide, 60 % = lent, au-delà la machine pagine).
pub fn verdict(p: &Profil, taille_go: f32) -> Verdict {
    let besoin = besoin_go(taille_go);
    let vram = p.gpus.iter().filter(|g| !g.integre).map(|g| g.vram_go).fold(0.0, f32::max);
    if vram > 0.0 && besoin <= vram {
        return Verdict::TientBien;
    }
    if besoin <= p.ram_totale_go * 0.35 {
        Verdict::TientBien
    } else if besoin <= p.ram_totale_go * 0.6 {
        Verdict::Lent
    } else {
        Verdict::TropGros
    }
}

/// Piège connu d'un modèle sur CETTE machine (mesuré, pas supposé).
pub fn avertissement(p: &Profil, nom: &str) -> Option<&'static str> {
    let igpu_amd = p.gpus.iter().all(|g| g.integre)
        && p.gpus.iter().any(|g| {
            let n = g.nom.to_lowercase();
            n.contains("radeon") || n.starts_with("amd")
        });
    (igpu_amd && nom.starts_with("qwen3-vl"))
        .then_some("plante à l'initialisation de la vision sur carte intégrée AMD (mesuré, Ollama 0.33-0.34)")
}

/// Un nom de modèle Ollama plausible (`famille[:tag]`, éventuellement
/// `hote/espace/famille`) — rien d'autre ne part vers le moteur.
pub fn nom_valide(nom: &str) -> bool {
    !nom.is_empty()
        && nom.len() <= 120
        && nom.chars().all(|c| c.is_ascii_alphanumeric() || "._:/-".contains(c))
        && nom.chars().next().is_some_and(|c| c.is_ascii_alphanumeric())
        && !nom.contains("..")
}

/// Avancement d'un téléchargement (toutes couches cumulées).
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Progression {
    pub statut: String,
    pub total: u64,
    pub fait: u64,
}

/// Cumul des couches d'un `ollama pull` : chaque ligne porte l'avancement
/// d'UNE couche (`digest`, `total`, `completed`).
#[derive(Default)]
pub struct Cumul {
    couches: Vec<(String, u64, u64)>,
}

impl Cumul {
    /// Lit une ligne NDJSON de `/api/pull`. `Err` = le moteur a refusé
    /// (modèle inconnu, disque plein…) ; `Ok(None)` = ligne sans intérêt.
    pub fn ligne(&mut self, json: &str) -> Result<Option<Progression>, String> {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(json.trim()) else {
            return Ok(None);
        };
        if let Some(e) = v["error"].as_str() {
            return Err(e.to_string());
        }
        let Some(statut) = v["status"].as_str() else { return Ok(None) };
        if let (Some(d), Some(t)) = (v["digest"].as_str(), v["total"].as_u64()) {
            let c = v["completed"].as_u64().unwrap_or(0);
            match self.couches.iter_mut().find(|x| x.0 == d) {
                Some(x) => (x.1, x.2) = (t, c.max(x.2)),
                None => self.couches.push((d.to_string(), t, c)),
            }
        }
        Ok(Some(Progression {
            statut: statut.to_string(),
            total: self.couches.iter().map(|c| c.1).sum(),
            fait: self.couches.iter().map(|c| c.2).sum(),
        }))
    }
}

/// Demande au moteur local de télécharger `nom`. `on` reçoit l'avancement et
/// retourne `false` pour annuler (le moteur garde les couches déjà reçues :
/// une reprise repart d'où elle s'est arrêtée). `Ok(true)` = installé,
/// `Ok(false)` = annulé.
pub fn telecharger(
    moteur: &LlmClient,
    nom: &str,
    mut on: impl FnMut(&Progression) -> bool,
) -> Result<bool, String> {
    if !nom_valide(nom) {
        return Err("nom de modèle invalide".into());
    }
    let body = serde_json::json!({"model": nom, "stream": true}).to_string();
    let mut cumul = Cumul::default();
    let mut erreur = None;
    let mut reussi = false;
    let fini = moteur.flux_lignes("POST", "/api/pull", &body, 600, |ligne| {
        if ligne.is_empty() {
            // Sonde pendant les attentes : seule l'annulation compte.
            return on(&Progression::default());
        }
        match cumul.ligne(ligne) {
            Err(e) => {
                erreur = Some(e);
                false
            }
            Ok(Some(p)) => {
                reussi |= p.statut == "success";
                on(&p)
            }
            Ok(None) => true,
        }
    })?;
    match erreur {
        Some(e) => Err(e),
        None if reussi => Ok(true),
        None if !fini => Ok(false),
        None => Err("le moteur a fermé la connexion avant la fin".into()),
    }
}

/// Un modèle installé sur le moteur local.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Installe {
    pub nom: String,
    pub taille_go: f32,
    pub capacites: Vec<String>,
}

/// Modèles installés depuis la réponse de `/api/tags` (taille, capacités si
/// le moteur les donne — Ollama ≥ 0.34).
pub fn installes_depuis_tags(json: &str) -> Vec<Installe> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else { return Vec::new() };
    v["models"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|m| {
                    Some(Installe {
                        nom: m["name"].as_str()?.to_string(),
                        taille_go: ((m["size"].as_u64().unwrap_or(0) as f64 / 1e8).round() / 10.0) as f32,
                        capacites: m["capabilities"]
                            .as_array()
                            .map(|c| c.iter().filter_map(|x| x.as_str().map(String::from)).collect())
                            .unwrap_or_default(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::materiel::Gpu;

    fn machine(ram: f32, gpus: Vec<Gpu>) -> Profil {
        Profil { ram_totale_go: ram, ram_dispo_go: ram / 2.0, gpus, npu: None, sac_actif: false, os: "windows".into() }
    }

    #[test]
    fn verdicts_de_la_machine_de_reference() {
        // 15,3 Go, carte intégrée : le 4B est fluide, le 8B lent, le 20B hors de portée.
        let p = machine(15.3, vec![Gpu { nom: "AMD Radeon(TM) 840M Graphics".into(), vram_go: 0.5, integre: true }]);
        assert_eq!(verdict(&p, 2.5), Verdict::TientBien);
        assert_eq!(verdict(&p, 3.3), Verdict::TientBien);
        assert_eq!(verdict(&p, 5.2), Verdict::Lent);
        assert_eq!(verdict(&p, 14.0), Verdict::TropGros);
        assert!(avertissement(&p, "qwen3-vl:4b-instruct").is_some());
        assert!(avertissement(&p, "gemma3:4b").is_none());
    }

    #[test]
    fn une_carte_dediee_change_le_verdict() {
        let p = machine(16.0, vec![Gpu { nom: "NVIDIA GeForce RTX 4070".into(), vram_go: 12.0, integre: false }]);
        assert_eq!(verdict(&p, 6.1), Verdict::TientBien);
        assert_eq!(verdict(&p, 15.0), Verdict::TropGros);
        assert!(avertissement(&p, "qwen3-vl:8b-instruct").is_none());
    }

    #[test]
    fn noms_admis_et_refuses() {
        for n in ["qwen3:8b", "mistral-nemo:12b", "hf.co/bartowski/Qwen_Qwen3-4B-GGUF:Q4_K_M", "gemma3"] {
            assert!(nom_valide(n), "{n}");
        }
        for n in ["", "a b", "x;rm", "../x", "-x", "x\"y"] {
            assert!(!nom_valide(n), "{n}");
        }
        assert!(CATALOGUE.iter().all(|e| nom_valide(e.nom)));
    }

    #[test]
    fn cumul_des_couches_et_erreur_du_moteur() {
        let mut c = Cumul::default();
        assert_eq!(c.ligne(r#"{"status":"pulling manifest"}"#).unwrap().unwrap().total, 0);
        c.ligne(r#"{"status":"pulling aaa","digest":"sha256:aaa","total":1000,"completed":400}"#).unwrap();
        let p = c
            .ligne(r#"{"status":"pulling bbb","digest":"sha256:bbb","total":500,"completed":500}"#)
            .unwrap()
            .unwrap();
        assert_eq!((p.total, p.fait), (1500, 900));
        // Une couche ne recule jamais (reprise : `completed` peut repartir bas).
        let p = c.ligne(r#"{"status":"pulling aaa","digest":"sha256:aaa","total":1000,"completed":100}"#).unwrap().unwrap();
        assert_eq!(p.fait, 900);
        assert!(c.ligne(r#"{"error":"pull model manifest: file does not exist"}"#).is_err());
        assert_eq!(c.ligne("1a3").unwrap(), None); // taille de chunk HTTP
    }

    #[test]
    fn installes_lus_avec_taille_et_capacites() {
        let j = r#"{"models":[{"name":"qwen3:8b","size":5225388164,"capabilities":["completion","tools","thinking"]},{"name":"x","size":0}]}"#;
        let l = installes_depuis_tags(j);
        assert_eq!(l[0].taille_go, 5.2);
        assert_eq!(l[0].capacites, vec!["completion", "tools", "thinking"]);
        assert!(l[1].capacites.is_empty());
    }
}
