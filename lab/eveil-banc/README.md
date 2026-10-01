# Banc R6b « L'Éveil » — wake word « Waly » (2026-07-21)

Plan de phase : `docs/PLAN-2026-07-21-R6b-l-eveil.md`. Trois gates AVANT tout
code produit. Moteur au banc : pile openWakeWord ONNX
(`engines/models/openwakeword/` — gitignoré, se retélécharge :
`melspectrogram.onnx`, `embedding_model.onnx`, `hey_jarvis_v0.1.onnx` depuis
la release v0.5.1 de github.com/dscripka/openWakeWord) via l'`onnxruntime.dll`
signée déjà en place — même voie SAC-safe que Silero (DLL, jamais d'exe).

## Boucle de dev

```bash
# build (WSL, cible windows-gnu, target dédié hors dépôt) :
CARGO_TARGET_DIR=~/waly-target-eveil cargo build --target x86_64-pc-windows-gnu
# ⚠ ort-sys DOIT être pinné =2.0.0-rc.9 (rc.10 = bindings incompatibles,
# 267 erreurs dans ort) — fait au Cargo.toml.
# l'exe : copié ici en eveil-banc.exe, exécuté côté Windows (SAC).
```

Sous-commandes : `gen` / `gen-extra` / `gen-pitch` (corpus TTS), `features`
(word = clip 2 s augmenté, slide = fenêtres glissantes ; K passes bruitées),
`bruit`, `pipeline`, `latence`, `evalue` (par fenêtre), `frr` / `far`
(séquentiels, patience 2 — les métriques MIROIR du runtime), `detecte`
(latence après fin du mot), `repos` (coût résident simulé).
Entraînement : `train/train_waly.py` (WSL, numpy pur + export ONNX,
standardisation cuite dans le graphe).

## VERDICTS (2026-07-21)

### GATE 1a — moteur sous SAC : ✅ VERT, impeccable

- Exe debug windows-gnu (dev opt-3) **passe SAC** ; les 3 modèles ONNX
  (3,6 Mo au total) tournent sur l'`onnxruntime.dll` signée (sessions
  1 thread — piège du pool qui spin-wait).
- Témoin officiel `hey_jarvis` : **9/9 positifs SAPI ≥ 0,997** (David/Zira/
  Hazel × 3 vitesses), **9/9 négatifs à 0,000** — l'échelle int16 du mel, la
  transformée `x/10+2`, le fenêtrage 76/8 et le contexte 16×96 sont EXACTS.
- Coût par pas de 80 ms : mel 88 µs + embedding ~1,4 ms + classifieur 18 µs
  = **1,5 ms ≈ 1,9 % d'un cœur** en continu.
- Trames mel mesurées : `frames = N/160 − 3` → chunk streaming = 1280 éch.
  + 480 de contexte = 8 trames exactes.

### GATE 1b — modèle « Waly » 100 % synthétique : 🟠 ORANGE (chemin clair)

Corpus : **355 prises « Waly » isolées + 293 porteuses** (« dis/hé/ok
Waly ») × Pocket 4 timbres (fabien, développeuse, clones Piper pierre/
jessica) à graines LIBRES + Piper direct + **8 pseudo-timbres par
pitch-shift des références de clonage** (réf rééchantillonnée ×0,88/×1,14
avant `set_voice` → autre locuteur pour l'encodeur mimi — l'astuce qui a
marché) ; négatifs = voisins phonétiques isolés (vallée, Willy, wallon…),
phrases pièges (« La vallée est magnifique… »), phrases neutres, bruit.
Split par FAMILLE de voix (développeuse + variantes = jamais vues).
4 itérations d'entraînement (v1→v4 ; v4 = MLP 1536-256-64-1, dropout 0,4,
bruit de features 0,25, négatifs durs ×3, sélection d'époque sur métrique
de DÉCISION à 0,8 — la val loss choisissait un modèle sous-appris).

Mesures v4 (`waly_v4.onnx`, seul conservé) :

| Métrique (séquentielle, patience 2) | seuil 0,85 | seuil 0,95 |
|---|---|---|
| FRR famille jamais vue (isolé / porteuses) | **18,0 % / 17,8 %** | 25,8 % |
| FRR voix d'entraînement (fabien) | 17,0 % | — |
| FAR phrases FR (échantillon 47 s) | ~1 alerte | ~1 alerte |
| FAR parole anglaise SAPI (moteur étranger) | ~6/min ✗ | **0** |
| Latence de détection après fin du mot | **−134 à +106 ms** ✓ | — |

**Verdict honnête : le 100 % synthétique Pocket/Piper NE suffit PAS seul**
pour « FRR confortable + FAR < 1/h » — le modèle sur-spécialise sur les
moteurs TTS d'entraînement (4 vrais timbres, même après pseudo-timbres :
l'écart train/val par fenêtre reste béant ; en séquentiel il se referme
mais à 18 % de FRR). **CE QUI EST PROUVÉ** : toute la chaîne
d'entraînement (corpus TTS → features Rust = le MÊME code que le runtime →
MLP numpy → ONNX → ort) marche de bout en bout et la latence est royale.
**Chemin de sortie (chantier R6b)** : l'ENROLLMENT PERSONNEL — 20-50
« Waly » réels de Michée à l'installation (validation aujourd'hui,
fine-tuning du classifieur demain, 100 % local, cohérent avec « l'appareil
personnel ») + garde-fous runtime (seuil ~0,9, patience 2, réfractaire
2 s, VAD co-signal, un faux éveil = un frémissement d'éclipse, pas un
enregistrement). Levier futur : 2ᵉ moteur TTS FR de diversité.

### GATE 2 — coût résident : ✅ VERT, très large

- Pipeline complet TOUJOURS ACTIF (sans porte VAD) : **1,4 % d'un cœur**
  (60 s de flux simulé). Avec porte Silero : 1,8-2,0 % — **la porte VAD
  coûte PLUS CHER que le pipeline qu'elle éteint** (Silero 2×0,7 ms/chunk) ;
  son intérêt éventuel = réduction du FAR, pas du CPU.
- **Pic WorkingSet : 46,3 Mo** (exe + ort + 3 modèles) ≪ 300 Mo.
- Zéro persistance : la boucle d'écoute n'écrit RIEN (revue : seuls
  `gen`/`features` écrivent, dev-time) ; l'audio vit dans un anneau RAM.

### GATE 3 — architecture d'éveil : DÉCISION

**waly-voice résident allégé (`WALY_VEILLE=1`), pas de listener dédié.**

- La veille = cpal + anneau 16 kHz + wake (mel/emb/clf) SEULS — ni
  Parakeet, ni Pocket, ni FLM (~50 Mo attendus, cf. GATE 2). waly-voice a
  déjà la capture, l'AEC (sa propre voix de rappel ne doit pas l'éveiller)
  et toute la cascade en aval : un mode, pas un binaire de plus (un
  listener dédié dupliquerait la pile audio et ajouterait un saut IPC).
- Spawn par le desktop au démarrage (même canal env que
  `WALY_ECRAN_MODE`) ; suspendue pendant ◐ Appel / ▣ Écran (waly-voice
  écoute déjà — pas de double micro).
- **L'éveil déclenche, dans l'ordre** : (1) signal instantané au desktop
  (loopback 52710, sceau-compatible) → éclipse « Éveil » **< 500 ms tenu**
  (détection −134/+106 ms + patience 160 ms + IPC ; budget large) ;
  (2) chargement de la cascade complète PENDANT que l'anneau continue de
  capturer (Parakeet 2,5 s — la phrase dite juste après « Waly » est
  rattrapée depuis l'anneau, jamais perdue) ; (3) conversation vocale
  directe sur le Fil principal (session 1), comme la voix d'appel.
- Faux éveil = frémissement + écoute quelques secondes puis retombée —
  aucun enregistrement, aucun tour LLM sans parole.

## Chantiers livrés (2026-07-21, après verdict « Souffle »)

- **Verdict design GRAVÉ : C « Souffle »** (planche réduite au choix,
  A/B supprimées, Artifact republié même URL).
- `wake.rs` dans waly-voice (WakeDetector streaming 1280+480 + WakeDecision
  pure testée : seuil/patience/réfractaire) ; modèle produit :
  `engines/models/openwakeword/waly_wake.onnx` (= v4 du banc).
- `waly-voice veille` : écoute légère seule ; à l'éveil → POST /eveil au
  desktop → cascade chargée PENDANT que le flux capture (rattrapage) →
  conversation directe (talk_loop) → retour en veille après
  `WALY_VEILLE_TIMEOUT` s (défaut 300). Seuil : `WALY_WAKE_SEUIL`
  (défaut 0,9).
- Desktop : veille spawnée au démarrage (WALY_VEILLE=0 désactive), tuée
  pendant Appel/Écran, relancée après, chien de garde 30 s au repos ;
  route POST /eveil + invoke `core_eveil` ; le « Souffle » anime la marque
  d'identité (titlebar) — pleine cadence seulement pendant la séquence.

## Enrollment (prises réelles de Michée — le chemin du FRR)

`run-enroll.ps1` (console, micro) : 30 prises « Waly » variées (distance,
volume, intonation) → FRR séquentiel du modèle courant sur TA voix →
si trop haut, features + re-entraînement avec les prises dans `--pos`,
export → `engines/models/openwakeword/waly_wake.onnx`. C'est là que le
critère RFC « 3 m » se mesure.
