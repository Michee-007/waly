# PLAN R-V « La Voix » — 2026-07-20

> Session flagship du mandat produit (RFC 2026-07-20 §I6.1). R-V est le GATE
> de toute exposition publique : le retour gravé du 10/07 (« voix moche et
> lente » vs Copilot) interdit de montrer quoi que ce soit avant que la voix
> soit belle ET rapide. Critères de sortie (roadmap v3) : voix jugée « belle »
> par Michée en A/B nommé vs Piper Pierre ; perçu ≤ 1,5 s ; budget 6,5 Go tenu.

## Contexte gravé (ne pas re-litiger sans Michée)

- **Piper upmc-PIERRE** = la voix gravée R1.5 (stable, 90 ms/clause) — c'est
  le champion à battre, il reste le DÉFAUT tant que Michée n'a pas tranché.
- **Pocket TTS french_24l** (Kyutai) = le seul TTS FR local de qualité
  supérieure ; toujours une **préversion non distillée** (vérifié 2026-07-20 :
  la distillation FR « more painful than anticipated », données en cause).
  Verdict R1.5 contre lui : identité vocale tirée au sort à CHAQUE prise.
- `pocket.rs` (pipeline natif complet sur ort, 04/07) était EN VEILLE —
  ce chantier le réveille ; le bundle et les voix avaient été supprimés au
  ménage R1.5 (re-téléchargés ce jour, HF `KevinAHM/pocket-tts-onnx` +
  `kyutai/tts-voices`).

## Chantier 1 — Pocket TTS streaming (LIVRÉ 2026-07-20)

`pocket.rs` : `synth_stream()` + `stream_sentence()` :
- **décodage mimi ENTRELACÉ** avec la boucle LM (chunks 2→4→8→12 trames :
  premier son tôt, efficacité ensuite) ;
- **porte de silence en flux** (`SilenceGate`, testée) : tête rognée à 120 ms
  de marge, pauses intra-phrase préservées à l'échantillon près, queue
  abandonnée au-delà de la marge — l'équivalent streaming de `trim_silence` ;
- **re-prise muette AVANT émission** : ~2,5 s de trames décodées sans voix =
  prise avortée silencieusement, 3 essais (l'ancienne borne de plausibilité
  min/max n'est pas rattrapable une fois l'audio émis — assumé, le plafond de
  trames borne la divagation) ;
- **sonde d'interruption par trame** (~80 ms) : chunk vide au callback, même
  contrat que le streaming LLM — le barge-in coupe une clause EN COURS de
  synthèse ;
- **graine fixe optionnelle** (`WALY_POCKET_SEED`) : prises déterministes
  (graine ^ FNV du texte ^ essai) — la parade à l'identité instable de la
  préversion : on CHOISIT une bonne prise au lieu de la subir. Vérifié au
  banc : mêmes durées à graine égale.
- `synth()` bloquant = collecte du stream (bench, warmup).

## Chantier 2 — mémoire (LIVRÉ 2026-07-20)

Vécu : **2,7 Go privés** au premier banc (documenté ~800 Mo en R1.5). Cause :
les ARÈNES ort gardent le pic pour toujours — la passe de conditionnement de
voix (~130 trames de référence à travers le LM 24 couches) + l'encodeur mimi.
Fixes dans `PocketNative` :
- `lm_main` en session **sans arène** (`with_memory_pattern(false)` +
  allocateur Device) — activations libérées après chaque run, surcoût RTF
  NUL (mesuré : RTF s'améliore même, 0,79-0,86) ;
- **encodeur mimi éphémère** : chargé dans `set_voice`, jeté après.
Résultat : **pic 1,2 Go WS / 1,76 Go commit** (2 passes de synthèse).
C'est ~1,1 Go de plus que Piper (~100 Mo) — à la place de Piper, jamais en
plus. Tribunal RAM complet en fin de plan.

## Chantier 3 — cascade (LIVRÉ 2026-07-20, terrain en attente)

`waly-voice.rs` :
- `Tts::synth_stream` uniforme (Piper = un chunk, Pocket = vrai flux) ;
- `speak()` streame vers le lecteur avec **tampon d'avance 350 ms**
  (`WALY_POCKET_LEAD_MS`) : le RTF oscille autour de 0,9-1,1 → pousser le
  premier chunk nu exposait à l'underrun en fin de phrase (trou au milieu
  d'un mot). Piper : zéro tampon, comportement inchangé ;
- la sonde d'interruption partagée (RefCell) vit dans le callback LLM ET la
  synthèse ; AEC, pouls, prosodie (pauses avant/après) inchangés ;
- prosodie : `speed` reste SANS EFFET sur Pocket (modèle de langage audio,
  pas de length_scale) — l'expressivité vient du modèle + la ponctuation ;
  les pauses pre/post s'appliquent comme avant ; `ClauseSplitter::
  sentences_only()` reste la règle Pocket (fragments = hallucinations) ;
- lanceur : `start-waly-voice.ps1 -Tts pocket [-TtsRef x.wav] [-TtsSeed 42]`.
- **71 tests verts** en natif Windows (67 existants + 4 SilenceGate).

## Chantier 4 — voix signature + A/B nommé — **VERDICT GRAVÉ (Michée, 2026-07-20)**

> « Je préfère C et B, le reste est soit trop robotique soit on n'entend
> rien. B si la personne veut une voix masculine et C si elle veut une voix
> féminine. »

**Gravé en conséquence** : Pocket french_24l = LA voix de Waly. **B fabien =
masculine (défaut)**, **C développeuse = féminine** (`WALY_TTS_SPEAKER=0` /
lanceur `-Voix femme`), **graine 42 par défaut dans le binaire** (l'identité
jugée à l'A/B ; `WALY_POCKET_SEED=0` = tirage libre). Pierre (A, « trop
robotique ») relégué moteur de secours explicite `WALY_TTS=piper` ; D et les
voix non retenues supprimées. Le compagnon d'appel/écran hérite du défaut
(spawn sans env). ⚠ Arbitrage accepté : la beauté coûte +0,4-1,9 s de part
TTS — « lente » reste un front ouvert (leviers au § Restes).

Kit d'écoute `engines/voice-bench/rv-*.wav`, mêmes textes (courte + longue),
graine 42 :
- **A = Pierre** (Piper upmc sid 1, la voix actuelle) ;
- **B = Fabien** (Pocket, clone `voices-fr/fabien.wav` — le choix R1.5 parmi
  36 candidats) ;
- **C = Développeuse** (Pocket, voix féminine Unmute) ;
- **D = Michée** (Pocket, clone `ma-voix.wav`) — TÉMOIN : prises quasi muettes
  et tronquées (2,9 s au lieu de ~5), cohérent avec le verdict R1.5 « micro
  trop bruité pour le clonage ». Ne sera pas la signature.
Protocole : écouter A vs B (puis C), plusieurs graines si B convainc
(`waly-voice pocket ../voices-fr/fabien.wav out.wav "texte"` +
`WALY_POCKET_SEED=n`) — graver la voix ET sa graine.

## Chantier 5 — banc Kyutai stt-1b-en_fr-candle (tribunal RAM)

`lab/stt1b-banc/` (crate AUTONOME hors workspace — candle/moshi n'entrent pas
dans l'arbre produit sans verdict) : charge le 1B (f32 CPU), mimi, décodage
tokenizer maison (pieces.json, zéro C++), transcrit les WAV réels, trace les
**têtes VAD sémantiques** (horizons 0,5/1/2/3 s — le candidat remplaçant
d'`endpoint.rs`), mesure RTF + RAM (pic, FFI psapi interne).
Profil dev opt-level 3 (piège 3 : release non signé bloqué par SAC).
Mesures → `engines/README.md`, verdict au JOURNAL.

## Mesures (2026-07-20, machine de référence, debug windows-gnu)

| Mesure | Valeur |
|---|---|
| Premier chunk Pocket (clause ~5 s, fabien, graine 42) | **0,26-0,86 s** (selon l'air mort échantillonné par la prise) |
| RTF Pocket (4 threads, fp32 flow+décodeur, 5 pas) | **0,79-1,09** (sans arène : 0,79-0,86) |
| 6 threads | PIRE (1,35-1,40 — cœurs Zen 5c hétérogènes) : rester à 4 |
| RAM Pocket (pic, modèles+voix+2 synthèses) | **1,2 Go WS / 1,76 Go commit** (était 2,7 Go avant fixes arène) |
| Piper Pierre (référence) | 0,21-0,42 s/clause, ~100 Mo |
| Tests waly-voice | 71 verts (natif Windows) |
| E2E `text` (FLM qwen3-it:4b, one-shot, préfill froid) — part TTS du premier son | **Pocket 0,82-1,90 s vs Piper 0,03-0,09 s** |
| Perçu estimé en session chaude (TTFT ~1,2-1,4 s) | Pocket ≈ 2,3-3,5 s ; Piper ≈ 1,4-2,0 s |
| Banc stt-1b-en_fr (candle, f32 CPU) | 4,2 Go (pic 5,7), RTF 3,0-3,4, ultra-court pas mieux → **disqualifié** |

## Restes / risques

1. **Perçu ≤ 1,5 s avec Pocket : improbable en l'état.** Chaîne : TTFT FLM
   ~1,2 s (plancher serveur) + première PHRASE entière (~15 tok ≈ 0,9 s à
   17 tok/s, `sentences_only`) + TTS 0,3-0,9 s + tampon 0,35 s ≈ **2,7-3,3 s**
   vs Piper ~1,4-2 s (clause à la virgule + 90 ms). Leviers si la qualité B
   l'emporte : couper à la virgule pour la PREMIÈRE clause (à re-tester —
   les fragments hallucinaient en 07/2026), TTFT upstream FLM, ou assumer
   le perçu Piper et garder Pocket pour un mode « qualité ».
2. Identité par prise : la graine fixe stabilise MAIS chaque phrase différente
   reste un tirage — le verdict d'écoute de Michée dira si c'est vivable.
3. E2E cascade réel (talk/appel) : à jouer (FLM éteint pendant la session,
   RAM machine occupée par les builds).
4. FLM v0.9.45 dispo : re-banc GATE A avant adoption (hors R-V).
