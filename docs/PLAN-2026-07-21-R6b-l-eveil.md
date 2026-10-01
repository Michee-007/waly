# PLAN R6b — « L'Éveil » : dire « Waly » réveille la présence (2026-07-21)

> Brief : RFC 2026-07-20 §I4 (roadmap v3) et §I5. Pilier « Présence » de
> l'identité « l'intelligence qui reste » : une présence vit au bord de
> l'écran, sans clic. Critère de sortie RFC : **« Waly » à 3 m → l'éclipse
> s'éveille < 500 ms, faux positifs < 1/h, budget RAM < 300 Mo.**

## Ce qu'on construit

Dire « Waly » à voix haute, n'importe quand, réveille la présence : l'éclipse
frémit (état « Éveil », verdict design de Michée à graver) et la conversation
vocale démarre directement — sans clic, 100 % local, sans jamais persister le
tampon audio. L'écoute au repos doit coûter quasi rien (CPU) et rester dans le
budget signé.

**Moteur choisi : la pile openWakeWord en ONNX** — trois petits modèles
(3,6 Mo au total) via l'`onnxruntime.dll` signée Microsoft déjà en place
(même voie SAC-safe que Silero VAD, piège n°3 : DLL, jamais d'exe) :

1. `melspectrogram.onnx` — audio 16 kHz → mel 32 bandes (entrée dynamique
   `[batch, samples]`, hop 10 ms) ;
2. `embedding_model.onnx` — le speech-embedding Google, fenêtres de 76 trames
   mel `[N, 76, 32, 1]` → vecteur 96-d (un pas tous les 80 ms) ;
3. le classifieur « Waly » — `[1, 16, 96]` (≈ 1,28 s de contexte) → score
   sigmoïde. C'est LUI qu'on entraîne ; les deux premiers sont gelés.

microWakeWord (alternative) est écarté d'entrée : modèles TFLite (il faudrait
embarquer un runtime TensorFlow Lite — un runtime de plus dans le budget,
et pas de DLL signée Microsoft) ; openWakeWord est nativement ONNX.

**Le corpus d'entraînement vient de Pocket TTS (R-V)** : des milliers de
« Waly » synthétiques (voix fabien + développeuse + voix du bundle, graines
variées `WALY_POCKET_SEED`, vitesses variées) + négatifs adverses (mots
proches : vallée, Wi-Fi, allez, wallon, voilà… ; phrases FR ; bruit).
L'entraînement du classifieur est du DEV-TIME (Python WSL, jamais shippé) ;
seul l'ONNX exporté ship.

## GATES au banc AVANT le code (culture du dépôt — `lab/eveil-banc/`)

| Gate | Question | Verdict attendu |
|---|---|---|
| **1a** | Le pipeline melspec→embedding→classifieur tourne-t-il sous SAC côté Windows via ort load-dynamic, et en combien de temps par pas de 80 ms ? | exe debug passe SAC, pipeline vérifié sur le modèle témoin `hey_jarvis` (WAV synthétique), latence par pas mesurée |
| **1b** | Un classifieur « Waly » entraîné sur du 100 % synthétique Pocket atteint-il une qualité utilisable ? | FRR/FAR mesurés sur jeu tenu (voix/graines jamais vues à l'entraînement) ; seuil choisi ; test micro réel |
| **2** | Coût résident de l'écoute ? | CPU % au repos quasi nul (mesuré sur 60 s), RAM du processus < 300 Mo (attendu ~dizaines de Mo), AUCUNE écriture disque du tampon (anneau mémoire, vérifié par revue + Process Monitor si doute) |
| **3** | Qui écoute au repos, et l'éveil déclenche quoi ? | décision documentée (waly-voice résident allégé vs listener dédié), avec les mesures qui la fondent |

## Architecture d'éveil pressentie (à confirmer par GATE 3)

- **waly-voice résident en « veille »** plutôt qu'un listener dédié : il a déjà
  cpal, le ring 16 kHz, Silero, l'AEC et la cascade. En veille, SEULS
  capture + melspec + embedding + classifieur tournent (Parakeet/Pocket/FLM ni
  chargés ni chauffés) ; au réveil, chargement de la cascade complète +
  signal au desktop (éclipse « Éveil ») + tour vocal direct.
- Gating par Silero (déjà résident, ~1 Mo) : le wake word ne calcule
  embedding+classifieur QUE quand il y a de la parole → CPU au silence ≈ VAD
  seul. À mesurer au gate 2.
- Le tampon audio d'écoute est un anneau en RAM, JAMAIS persisté — rien ne
  touche le disque tant que « Waly » n'a pas été dit (et même après : seul le
  flux de la conversation engagée suit le chemin normal de la cascade, déjà
  zéro-persistance).
- Coexistence : en appel/écran, waly-voice écoute déjà en continu → le wake
  word en veille ne tourne que HORS session vocale active (pas de double
  micro).

## Chantiers (après gates verts)

0. `wake.rs` dans waly-voice (pipeline streaming 80 ms, seuil, hystérésis,
   patience — N pas consécutifs au-dessus du seuil) ;
1. mode veille de waly-voice (`waly-voice wake` : capture + wake seul, spawn
   à l'ouverture de session Windows ou par le desktop) ;
2. déclenchement : signal au desktop (éclipse « Éveil ») + entrée directe en
   conversation vocale (réutilise la session « Fil principal ») ;
3. état « Éveil » de l'éclipse (frémissement d'écoute — verdict Michée sur
   planche A/B/C, monochrome gravé) ;
4. docs (JOURNAL, CLAUDE.md, ce plan tenu à jour).

## Critères de sortie (RFC, mesurés)

- « Waly » à 3 m → éveil (premier signe visible/audible) **< 500 ms** ;
- faux positifs **< 1/h** en usage réel (bureau, musique de fond) ;
- budget RAM du résident de veille **< 300 Mo** (attendu : ~30-60 Mo) ;
- le tampon audio n'est **JAMAIS** persisté (revue de code + vérif terrain) ;
- verdict design de Michée sur l'état « Éveil » (planche A/B/C).

## Risques identifiés

- **Qualité du 100 % synthétique** : openWakeWord lui-même est entraîné sur
  du synthétique (c'est sa méthode canonique), mais Pocket french_24l a moins
  de diversité de locuteurs qu'un TTS multi-speaker anglais. Parades :
  variations (graines, vitesse, pitch par rééchantillonnage), augmentation
  (bruit, RIR/réverb simple, gain), et si FRR trop haut → compléter avec
  Piper (deux timbres de plus) et quelques enregistrements réels de Michée
  en jeu de VALIDATION (jamais d'entraînement, méthode : le modèle doit
  généraliser).
- **« Waly » = 2 syllabes courtes** : plus dur qu'« Alexa »/« Hey Jarvis ».
  Si FAR ingérable, fallback assumé : « Hé Waly » (à trancher avec Michée
  SEULEMENT si la mesure l'impose).
- **AEC en veille** : si Waly parle (rappels vocaux au repos), sa propre voix
  ne doit pas le réveiller — la garde de corrélation `aec.rs` existe déjà,
  à brancher au chantier 1.
