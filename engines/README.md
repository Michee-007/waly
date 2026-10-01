# engines/ — moteurs d'inférence natifs Windows

Les binaires et modèles vivent dans `C:\waly\engines\` (jamais dans git).

## Moteur A — FastFlowLM (NPU XDNA 2) — prioritaire
- Install : zip portable GitHub `FastFlowLM/FastFlowLM` → `C:\waly\engines\flm\`
  (installé le 2026-07-03, v0.9.43 ; driver NPU vérifié 32.0.203.329).
- Lancer : `powershell -File start-flm.ps1` (qwen3:4b, port 52625, OpenAI-compat `/v1`).
- Texte : `qwen3:4b` · VLM (R4) : `qwen2.5vl-it:3b` · liste : `flm list`.
- Mesures publiées (même NPU, Ryzen AI 7 350) : qwen3:4b décodage 19,6 tok/s @1k,
  préfill 615 tok/s.
- **Nos mesures (2026-07-03, Ryzen AI 5 340, ctx-len 8192)** :
  - qwen3:4b : **préfill 617,9 tok/s** (3 063 tok en 4,96 s) · décodage 16,4-19,2 tok/s
    · tour court complet ~2 s. Vs base CPU qwen3:8b : préfill **×16**, décodage ×1,7-1,9.
  - qwen3:1.7b : décodage 39,3 tok/s, tour complet 1,6 s (candidat routage/résumé).
  - Cache FLM préexistant découvert : Whisper-V3-Turbo-NPU2 (STT sur NPU pour R1),
    Llama-3.2-1B, Qwen3-0.6B, Qwen3.5-2B.
- ⚠ Piège documenté : ctx-len défaut 32k → KV ~4,5 Go → échec 0xc01e0200 sur 16 Go.
  Toujours passer `--ctx-len` (8192 voix/chat). Un modèle à la fois sur le NPU.

## ⚠ Distribution : Smart App Control
SAC (activé sur la machine de réf.) bloque les exe non signés téléchargés :
llama-server.exe (release GitHub) est bloqué. flm.exe passe (signé).
→ Pour shipper Waly : signature de code obligatoire pour tous nos binaires/sidecars.
→ En dev : moteur B via Ollama Vulkan (signé) en attendant.

## Moteur B — Vulkan iGPU (fallback toutes machines)
- **En dev : via Ollama** (signé, passe Smart App Control). Variables utilisateur posées
  le 2026-07-03 : `OLLAMA_VULKAN=1`, **`OLLAMA_IGPU_ENABLE=1`** (sans elle Ollama
  écarte l'iGPU — c'était LA cause du 100 % CPU historique), `OLLAMA_KV_CACHE_TYPE=q8_0`,
  `OLLAMA_FLASH_ATTENTION=1`. ROCm : impossible (gfx1152 non supporté, confirmé au log).
- llama-server.exe (release GitHub, non signé) : **bloqué par Smart App Control** —
  réservé au jour où nos binaires seront signés. Binaires dans `C:\waly\engines\llama\`,
  modèle Qwen3-VL-4B GGUF Q4_K_M + mmproj dans `C:\waly\engines\models\`.
- **Mesures (2026-07-03, Ollama Vulkan, qwen3:4b, 100 % GPU confirmé)** :
  préfill long 119,6 tok/s (3 056 tok en 25,6 s) · décodage 12,4-14,8 tok/s ·
  tour court à chaud 3,9 s.

## Verdict R0 (mesuré sur la machine de référence)

| qwen3:4b | Préfill 3k tok | Décodage | Tour court chaud |
|---|---|---|---|
| CPU (base 8B : 38,8 / 9,9) | 77,5 tok/s | 13 tok/s | — |
| iGPU Vulkan (840M) | 119,6 tok/s (25,6 s) | 12,4-14,8 | 3,9 s |
| **NPU (FastFlowLM)** | **617,9 tok/s (4,96 s)** | **16,4-19,2** | **~2 s** |

Hiérarchie confirmée : NPU = moteur principal (préfill ×16 vs base), Vulkan = fallback
honorable (préfill ×3, libère le CPU), décodage partout ~12-19 tok/s (borne bande
passante, comme prédit par l'audit). Critère R0 « < 2 s/tour à chaud » atteint au
niveau moteur sur le NPU.

## STT R1 — Whisper-V3-Turbo sur NPU (mesuré 2026-07-03)
- Lancer : `powershell -File start-flm-asr.ps1` (mode ASR **standalone**, port 52625).
  Le LLM tourne dans un second processus : `start-flm.ps1 -Port 52626`.
- ⚠ **Piège v0.9.43** : `/v1/audio/transcriptions` ne marche QU'en standalone
  (`flm serve --asr 1` sans LLM). LLM co-chargé → réponse `null` HTTP 200 en 3 ms.
  (PR upstream #510 en cours sur le multipart ; réévaluer à la prochaine version.)
- Appel : `curl.exe -F "file=@x.wav;type=audio/wav" -F "model=whisper-v3:turbo"
  http://127.0.0.1:52625/v1/audio/transcriptions` (le champ `language` est ignoré en 0.9.43).
- **Mesures (WAV 16 kHz mono, à chaud)** :

| Énoncé | Durée audio | Latence transcription |
|---|---|---|
| FR court | 4,8 s | 2,57-2,83 s (méd. ~2,65 s) |
| FR long | 10,7 s | 2,56-2,64 s |
| EN | 9,4 s | 3,03 s |

  Latence **constante ≈ 2,6 s** quelle que soit la durée (fenêtre Whisper de 30 s
  encodée entière). Qualité : parfaite sur l'énoncé EN (voix SAPI native) ; les
  énoncés FR étaient lus par une voix SAPI *anglaise* (aucune voix FR sur la machine)
  → transcription FR dégradée non significative, à re-mesurer au vrai micro.
- **Cohabitation NPU (2 processus flm)** : chat seul 2,1 s / ASR seul 2,7 s ;
  simultanés : chat 3,9 s + ASR 4,3 s. Sérialisation ~additive, **aucun crash** —
  acceptable, le pipeline voix est séquentiel.
- **Duel sur voix réelle (fr-reel.wav, 6 s, mesuré 2026-07-03)** :

| STT | Latence finalisation | Qualité FR |
|---|---|---|
| Whisper-V3-Turbo NPU | 2,84-3,07 s | parfaite |
| **Parakeet-TDT 0.6B v3 int8, 4 threads CPU** (sherpa-onnx, WSL) | **0,20-0,25 s** (RTF 0,04) | parfaite, meilleure ponctuation |

- **Verdict R1 (tranché)** : **Parakeet v3 = STT primaire** (12× plus rapide, ~650 Mo
  int8, 25 langues dont FR). Whisper-NPU = re-transcription qualité optionnelle.
- **Intégration NATIVE validée (2026-07-03)** : `waly-voice.exe stt fr-reel.wav` →
  **0,18-0,21 s** (load 1,46 s), transcription parfaite. Architecture : notre exe
  Rust (debug) + FFI libloading sur `sherpa-onnx-c-api.dll` **v1.13.3** prébuilt
  (`C:\waly\engines\sherpa\lib\`, non signée : les DLL passent SAC, seuls les EXE
  sont filtrés). Modèle : `C:\waly\engines\models\parakeet-tdt-0.6b-v3-int8\`.
  ⚠ Les structs FFI de `crates/waly-voice/src/stt.rs` répliquent le c-api.h
  v1.13.3 champ par champ — re-vérifier À CHAQUE mise à jour de la DLL.
  La même DLL expose le TTS Piper (OfflineTts) : prochaine brique, même chemin.
- Note : une erreur 500 `invalid string position` sur `/v1/chat/completions` a été
  observée une fois sur le serveur co-chargé ASR+LLM (avec `max_tokens`) ; non
  reproduite en serveur LLM seul. À surveiller.
- **Énoncés ULTRA-COURTS (< 1 s) — chantier R2 instruit le 2026-07-07, sur audio
  PROPRE (Piper PIERRE, zéro bruit micro)** : la dérive est le MODÈLE, pas le
  micro. Parakeet : « Oui. » → « Yeah. », « Attends. » → (vide), « Hello mon
  pote. » → « Elle ou mon pote. » ; les 2+ s (« fr-reel ») restent parfaits.
  Pistes fermées avec mesures :
  - **Whisper-NPU en re-transcription : NON** — pas meilleur sur l'ultra-court
    (« Oui. » → « Mm-hmm. », « Parfait. » → « Paffee. », 2/6 exacts comme
    Parakeet) pour ~2,3 s de latence par appel.
  - **Padding de silence (1 s avant/après) : NON** — n'aide pas, parfois pire
    (« Attends. » paddé → « Python. »).
  - **Forçage de langue : IMPOSSIBLE** — l'API C sherpa n'expose `language` que
    pour Whisper/SenseVoice/Canary, pas pour le transducer TDT (auto-détection
    seule sur les 25 langues).
  Parade en place : garde `looks_offlang_hallucination` (waly-voice, généralisée
  en→pt/es/it/ro/de le 2026-07-07 — lettres étrangères au français í/ă/ã… =
  signal fort). Limite résiduelle ASSUMÉE : un mot isolé très court peut être
  avalé (rejeté par la garde ou vide) — répéter suffit ; suivre les prochains
  modèles STT pour lever ça.
- ⚠ **Piège ASR v0.9.43 (vécu 2026-07-07)** : après un envoi malformé (WAV
  22 kHz et/ou multipart sans `;type=audio/wav` — cause exacte non isolée, un
  flm résiduel tournait aussi au chargement), l'endpoint tombe dans un état
  **`null` PERMANENT** (HTTP 200 `null` instantané pour TOUTES les requêtes
  suivantes, même valides) → tuer/relancer le processus flm. Toujours envoyer
  du **16 kHz mono s16** avec `;type=audio/wav`. Un `flm serve` spawn un couple
  parent+worker : les tuer TOUS LES DEUX (le worker garde le port sinon).

## TTS R1 — Piper FR via sherpa-onnx (mesuré 2026-07-03, WSL, 2 threads CPU)

| Voix (medium, 22 050 Hz) | Clause courte (2,4 s audio) | Phrase longue (7 s) | RTF | Load |
|---|---|---|---|---|
| fr_FR-siwis | **0,090 s** | 0,258 s | 0,04 | 0,59 s |
| fr_FR-upmc | 0,095 s | 0,265 s | 0,04 | 0,53 s |

→ **Le TTS ne sera pas le goulot du budget ≤ 1 s** (~90 ms par clause, 2 cœurs).
WAV d'écoute : `voice-bench/tts-siwis-*.wav`, `tts-upmc-*.wav` (juger la qualité FR).
Alternative qualité si Piper déçoit : Pocket TTS (Kyutai, FR depuis avril 2026,
ports ONNX/Rust communautaires). Modèles : `~/tts-bench/` dans WSL
(vits-piper-*.tar.bz2 des releases sherpa-onnx).

### Pocket TTS — R-V STREAMING (état 2026-07-20)

- **Le pipeline natif est STREAMING** (`pocket.rs::synth_stream`, phase R-V) :
  décodage mimi entrelacé (chunks 2→4→8→12 trames), porte de silence en flux
  (tête/queue rognées à 120 ms, pauses intra-phrase préservées), re-prise
  muette AVANT émission (2,5 s décodées sans voix = prise avortée, 3 essais),
  sonde d'interruption par trame (barge-in coupe une clause en cours).
- **Bundle re-téléchargé** (supprimé au ménage R1.5) : HF
  `KevinAHM/pocket-tts-onnx` → `models/pocket-tts-fr-24l/` (LM int8 305 Mo +
  flow/décodeur/encodeur fp32, ~460 Mo). Tokenizer converti (WSL,
  `pip install sentencepiece --user --break-system-packages` puis script du
  README sherpa) — IDs identiques à la vérité terrain des tests. Voix :
  `voices-fr/` (fabien, degaulle, developpeuse, defaut-unmute, HF
  `kyutai/tts-voices`). ⚠ french_24l = TOUJOURS une préversion non distillée
  (vérifié 20/07 : distillation FR « more painful than anticipated »).
- **Mesures (debug windows-gnu, 4 threads, graine fixe)** : premier chunk
  **0,26-0,86 s** (selon l'air mort échantillonné), RTF **0,79-1,09** ;
  6 threads = PIRE (1,35-1,40, cœurs Zen 5c) → rester à 4. E2E cascade
  (`text`, FLM qwen3-it:4b) : TTS 0,82-1,90 s jusqu'au premier son vs
  0,03-0,09 s Piper → **Pocket coûte +0,8-1,8 s de perçu**, tampon d'avance
  350 ms compris (`WALY_POCKET_LEAD_MS` — RTF ~1 : sans tampon, trous en fin
  de phrase).
- ⚠ **PIÈGE ORT MAJEUR (vécu, −1,5 Go)** : les arènes ort gardent le PIC
  d'activations pour toujours — la passe de conditionnement de voix
  (~130 trames à travers le LM 24 couches) laissait **2,7 Go privés**.
  Fixes gravés dans `PocketNative` : `lm_main` en session SANS ARÈNE
  (`with_memory_pattern(false)` + allocateur Device — surcoût RTF nul,
  c'est même plus rapide) et encodeur mimi ÉPHÉMÈRE (chargé/jeté dans
  `set_voice`). Résultat : **pic 1,2 Go WS / 1,76 Go commit** — ~1,1 Go de
  plus que Piper, à sa place, jamais en plus.
- **Graine fixe `WALY_POCKET_SEED`** : prises déterministes (graine ^ FNV du
  texte ^ essai) — parade à l'identité par prise de la préversion ; vérifié
  au banc (durées identiques à graine égale). **Défaut du binaire : 42**
  (l'identité jugée à l'A/B) ; `0` = tirage libre.
- **VERDICT GRAVÉ (Michée, 2026-07-20)** : Pocket = LA voix de Waly, moteur
  PAR DÉFAUT (`WALY_TTS` absent). **fabien = masculine (défaut),
  développeuse = féminine** (`WALY_TTS_SPEAKER=0` / lanceur `-Voix femme`).
  Piper/Pierre (« trop robotique ») = secours : `WALY_TTS=piper` /
  `-Tts piper`. Échantillons de référence :
  `voice-bench/rv-{B-fabien,C-developpeuse}-*.wav` (graine 42).

### STT-1B Kyutai au tribunal RAM (banc R-V 2026-07-20) — DISQUALIFIÉ

`kyutai/stt-1b-en_fr-candle` via `lab/stt1b-banc/` (candle+moshi purs Rust,
profil dev opt-level 3 — l'exe 232 Mo passe SAC ; décodage tokenizer maison
`pieces.json`, modèles `models/stt-1b-en_fr-candle/` ~2,4 Go) :

| Mesure | stt-1b (f32 CPU) | Parakeet-TDT 0.6B int8 (titulaire) |
|---|---|---|
| RAM résidente | **4,2 Go (pic 5,7 Go)** | 0,65 Go |
| Vitesse | **RTF 3,0-3,4** (3× plus lent que le réel) | 0,2-0,25 s pour 6 s |
| FR long (fr-reel.wav) | parfait, ponctué | parfait |
| Ultra-court (« Oui. », « Attends. ») | **vide / « Pas de temps. »** — pas mieux | avalé/dérive (limite connue) |
| Fin de tour | VAD sémantique intégrée (têtes 0,5/1/2/3 s — belle idée) mais texte retardé de 2,5 s → flush ≈ delay × RTF ≈ **1,5-1,7 s** | endpoint.rs : 0,28-0,8 s |

**Verdict : disqualifié sur les trois tableaux** (RAM 6,5×, vitesse
inutilisable en streaming CPU, aucun gain sur la faiblesse connue de
Parakeet). Un GGUF q8 (~1,2 Go, ~2-3× plus rapide) resterait au mieux
marginal (RTF ~1). **Parakeet + endpoint.rs gardent leur place.** La VAD
sémantique reste l'idée à suivre si Kyutai sort un modèle ≤ 300M ou un
chemin NPU.

### Pocket TTS (état 2026-07-04, soir)
- **FRANÇAIS OPÉRATIONNEL EN NATIF** : pipeline maison `pocket.rs` sur ort
  load-dynamic (sherpa ne gère pas le schéma 2 des bundles récents, vérifié
  v1.13.3 + master). Bundle `models/pocket-tts-fr-24l/` (french_24l int8) :
  **RTF 0,60-0,65** (4 threads), clonage de voix depuis ~6 s de référence,
  BOS-avant-voix géré, tokenizer unigram maison validé contre SentencePiece.
  Activer : `start-waly-voice.ps1 -Tts pocket [-TtsRef voix.wav]` (ou
  `WALY_TTS=pocket`). ~800 Mo résidents, à la place de Piper.
  Latence v1 : 1re clause payée entière (~0,6 s/s d'audio) → streaming des
  chunks mimi = prochaine optimisation si la qualité est actée.
- Wrapper DLL `tts::PocketTts` conservé (archives sherpa schéma 1 : modèle
  anglais officiel OK, RTF 0,20). ⚠ `num_steps` y est utilisé TEL QUEL
  (pas de « 0 = défaut ») : toujours passer 5.
- Écoute : `voice-bench/tts3-pocket-fr-court.wav`, `tts3-pocket-fr-long.wav`
  (FR, voix Michée clonée) ; `tts3-pocket-en-test.wav` (bria),
  `tts3-pocket-en-clone-michee.wav` (EN).
- **Banque de voix FR de référence** : `engines/voices-fr/` (source :
  HF `kyutai/tts-voices`, public — `unmute-prod-website/` = voix du produit
  Unmute de Kyutai, `cml-tts/fr/` = LibriVox nettoyé). Utiliser :
  `start-waly-voice.ps1 -Tts pocket -TtsRef C:\waly\engines\voices-fr\<voix>.wav`.
  Échantillons comparatifs (même texte) : `voice-bench/voix-*.wav`.
  Licences VÉRIFIÉES (2026-09-10) : `fabien` (= `fabieng-enhanced-v2`) et
  `developpeuse` (= `developpeuse-3`) sont des enregistrements propres de
  Kyutai → **CC0**, redistribuables. ⚠ Dans ce même dossier : `ex04_*` =
  Expresso **CC-NC**, `p329_022` = VCTK CC BY. Inventaire complet des
  licences tierces : `THIRD_PARTY_NOTICES.md` (racine).

## VLM R4 — qwen3vl-it:4b sur NPU (mesuré 2026-07-07, GATES A+B du plan R4)

- Modèle : `qwen3vl-it:4b` (Qwen3-VL-4B-Instruct-NPU2, 3,9 Go dont encodeur
  vision `vision_weight.q4nx` 792 Mo). Téléchargé dans `~\.flm\models\`.
- Lancer : `start-flm.ps1 -Model qwen3vl-it:4b` (le `--ctx-len 8192` du script
  s'applique — piège 1 vaut aussi pour le VLM).
- **API vision = OpenAI-compat standard** : content-parts `image_url` en
  `data:image/png;base64,...`. Outils (`tools`) et image dans la MÊME requête :
  OK, aucune confusion mesurée.
- **Mesures (Ryzen AI 5 340, ctx 8192, pmode performance, à chaud)** :

| Requête | Prompt tok | TTFT | Total |
|---|---|---|---|
| Texte court (« Hi », 1 tok out) | 9 | 1,12 s | 1,19 s |
| Texte FR (38 tok out) | 35 | 1,07 s | 3,32 s |
| Image 640×480 + question (77-90 tok out) | 325 | 1,87-1,95 s | **6,8-7,6 s** |
| Photo 4K 1,6 Mo (49 tok out) | 848 | 3,15 s | 7,86 s |
| **Forme de prod** : système+outils+historique+image (28 tok out) | 1 083 | 2,91 s | **4,5-4,7 s** |
| Tool-calling FR (3 outils, `heure_actuelle`) | 302 | 1,49 s | 2,54 s |

  Décodage constant 17,1-17,8 tok/s (= qwen3-it:4b). Une image 640×480 ≈
  300 tok de vision ; le 4K ≈ 820 tok → **redimensionner les frames (~640 px)
  avant envoi**. Critère R4 « moment VLM < 8 s » : ✓ au banc.
- **Tool-calling natif : OK** (`finish_reason:"tool_calls"`, bon outil, args
  propres) et **calibré** (question directe → pas d'outil parasite). Qualité
  FR à confirmer à l'oreille, mais rien d'anormal au banc.
- **RAM : 5,96 Go privés résidents** (vs ~4,3 Go pour qwen3-it:4b — l'encodeur
  vision se paie). Budget 6,5 Go : le cerveau unique VLM + stack voix ≈ 7 Go →
  serré, à mesurer en conditions réelles au chantier 5 du plan R4.
- ⚠ **Hot-swap de modèle = MORT sur cette machine** : envoyer un `model:` autre
  que celui chargé déclenche un rechargement à la volée (comportement FLM) qui
  a tué le serveur en plein vol (OOM probable, exit 255, zombie tenant le
  verrou NPU à tuer ensuite). **Un serveur = un modèle épinglé ; valider le
  nom de modèle côté client avant d'envoyer.** Corollaire depuis la bascule
  R4 : les binaires d'AVANT le 2026-07-07 soir (app installée !) envoient
  `qwen3-it:4b` en dur → NE PAS les utiliser contre un serveur qui charge le
  VLM (et vice-versa). `WALY_MODEL` surcharge le défaut des binaires neufs.

### Chantier 4 (mesuré 2026-07-07 soir) — le VLM est LE cerveau, l'outil `regarder` vit

- **Bascule faite** : `waly_core::llm::modele_par_defaut()` →
  `qwen3vl-it:4b` (surcharge env `WALY_MODEL`) ; bin waly, desktop, voix
  alignés. Lancer le cerveau : `start-flm.ps1 -Model qwen3vl-it:4b -Port
  52626 -PMode turbo`.
- **Boucle agentique complète mesurée avec le VLM (debug, base :memory:)** :

| Tour | Durée |
|---|---|
| « Quelle heure est-il ? » (répond depuis l'horodatage, zéro outil) | 4,3 s |
| « Mémorise que ma couleur préférée… » (outil `memoriser` + conclusion) | 6,8 s |
| « Regarde ce que tu vois » (outil `regarder` → image 1080p → description) | 11,5 s |
| idem, image **640 px JPEG 41 Ko** (frame prête pour la prod) | **9,8 s** |

  Description exacte de la scène réelle dans les deux cas. Les ~10 s = DEUX
  tours de modèle (décision d'outil ~1,2k tok re-préfillés + tour image) —
  le « moment VLM » seul reste ~5-6 s ✓. **Levier chantier 5 : raccourci
  d'intention** (l'app joint l'image au message utilisateur quand la caméra
  est active → un seul tour ≈ 5 s).
- **Budget prompt : 14 outils = 1 241 tokens** (tokenizer VLM ; `regarder`
  ≈ 69 tok). > 1k assumé depuis R2 (parade backlog : sélection contextuelle).
- **Une seule image vit dans l'historique** (les anciennes sont dégradées en
  marqueur texte par `chat::injecter_apres_round`) : sans cache de préfixe
  FLM, chaque image ancienne recoûterait ~300-800 tok de préfill par tour.

## Perception R4 — caméra native + YuNet (mesuré 2026-07-07, GATES C+D du plan R4)

- **Capture caméra native : `crates/waly-sight`** (nokhwa 0.10 backend Media
  Foundation, pur Rust + crate `windows` — cross-compile windows-gnu depuis
  WSL, **exe debug passe SAC du premier coup**). Binaire de banc :
  `C:\waly\bin\waly-sight.exe` (`cams` / `snap` / `bench-face` / `bench-live`).
- Caméra de réf. : HP True Vision FHD (NV12). ⚠ `AbsoluteHighestFrameRate`
  de nokhwa choisit n'importe quoi (1080p) et l'étiquette fps affichée est
  fausse (« @1FPS » pour un flux 30 fps réel) → demander explicitement
  `Closest(640×480@30, NV12→MJPG→YUYV)` (fait dans `camera::Cam::open`).
- **Détection visage : YuNet 2023mar** (opencv_zoo, 232 Ko,
  `engines/models/yunet/`) sur la onnxruntime.dll signée habituelle
  (load-dynamic). ⚠ Le modèle publié est à **entrée FIXE 640×640** (l'ancien
  YuNet dynamique n'est plus distribué) → letterbox interne dans
  `face::FaceDetector`. Sorties décodées façon OpenCV : score=sqrt(cls×obj),
  bbox exp()×stride, 5 points (yeux/nez/bouche — serviront à l'attention).
- **Mesures (builds DEBUG, CPU seul, Ryzen AI 5 340)** :

| Banc | Mesure |
|---|---|
| Ouverture caméra | ~250 ms |
| Frame 640×480 NV12→RGB | 21,4 ms méd |
| Frame 1920×1080 NV12→RGB | 58,1 ms méd |
| YuNet detect (640×640, letterbox inclus) | **9,6-10,8 ms méd** (p90 ~11,5) |
| Boucle live 640×480 non bridée | **29,7 fps**, visage 297/297 frames |

  → À la cadence produit (5-10 Hz), la boucle de perception coûtera ~15-30 %
  d'UN cœur. Budget « réaction perçue < 500 ms » : pire cas à 5 Hz ≈ 230 ms
  (échantillonnage + capture + détection) ✓ — la marge servira à
  l'attention/expression du chantier 3.
- Frame de banc : `engines/bench-cam-frame.png` (capturée avec Michée
  présent — la règle produit reste : aucune frame persistée par défaut).

### Service d'événements (R4 chantier 3, mesuré 2026-07-07)

- **`waly-sight.exe watch [cam] [secs]`** : boucle 8 Hz → événements JSON
  typés (`arrivee`/`depart`/`attention`/`expression`) via la machine à
  hystérésis de `perception.rs` (pure, 5 tests). Expression = **FER+ int8**
  (19 Mo, `engines/models/ferplus/`, 8 classes, crop visage 64×64 gris) —
  brique OPTIONNELLE (absente → pas d'événements expression).
- **Mesures (debug, 8 Hz, 30 s)** : démarrage 292 ms ; **`arrivee` émise
  0,30 s** après le lancement (hystérésis 2 frames = théorie ✓) ; cadence
  effective 7,9 Hz ; par cycle : capture 8,2 ms + visage 12,0 ms + humeur
  11,8 ms ; **CPU ~41 % d'un cœur** (≈ 7 % machine).
- ⚠ **Piège ort gravé** : le pool de threads par défaut SPIN-WAIT entre les
  inférences → **212 % d'un cœur** à vide mesuré avant bridage. Toujours
  `with_intra_threads(1-2)` + `with_inter_threads(1)` sur les petits
  modèles en boucle.
- ⚠ **Piège SAC re-confirmé (2×)** : un `touch` ne suffit PAS à re-roller un
  verdict (build reproductible = même hash) — il faut une VRAIE mutation du
  code (const `SAC_REROLL` à incrémenter dans le bin, faite pour ça).
- Attention = orientation de TÊTE (nez vs milieu des yeux, seuil 0,35),
  assumé approximatif — seuil à régler à l'œil avec Michée ; transitions
  attention/départ à valider en session active (il n'a pas bougé au banc).

## Cache de conversation FLM v0.9.43 (mesuré 2026-07-08, GATE A du plan R4.5)

Banc : `bench-cache-*.json`, serveur `qwen3vl-it:4b --pmode turbo
--ctx-len 8192` déjà chaud, `max_tokens:1` non-streaming (TTFB ≈ TTFT),
système long ~1 650 tok. **La mesure « aucun cache » du 2026-07-04 est
PARTIELLEMENT caduque** : v0.9.43 a bien un cache de conversation, mais à
contrat strict.

| Cas | TTFB |
|---|---|
| Prompt court à chaud (plancher) | **1,17-1,32 s** |
| Long 1 650 tok, 1re fois | 3,24 s |
| Requête IDENTIQUE rejouée | 3,09-3,12 s (**pas de réemploi**) |
| Conversation qui S'ALLONGE d'un tour (append strict) | **1,24-1,26 s = plancher** ✓ |
| Append avec contexte frais DANS le dernier message user | **1,25 s** ✓ |
| Divergence en TÊTE (1 mot du système) | 3,15 s (plein tarif) |
| Divergence en QUEUE (1 mot du contexte tardif) | 3,04 s (plein tarif) |
| Message `system` en MILIEU de conversation | 3,05-3,12 s (casse le cache — template) |
| Requête indépendante intercalée puis retour au dialogue | 3,15 s (**éviction : UNE case**) |

**Contrat gravé** : réemploi UNIQUEMENT si la nouvelle requête étend
STRICTEMENT la conversation en cache (historique verbatim + réponse
générée + nouveau contenu en fin). Toute mutation n'importe où = tout se
re-paie. Une seule case : toute requête d'une autre « conversation »
(moment VLM proactif, autre session) évince le cache du dialogue.

**Conséquences produit (re-priorisent R4.5)** :
1. **Discipline append-only dans waly-core** = LE levier TTFT interne :
   système STABLE par session (ne plus pousser la conscience sur
   `messages[0]`, ne plus re-injecter souvenirs/attentes chaque tour),
   contexte frais À L'INTÉRIEUR du dernier message user (l'horodatage y
   est déjà), fenêtre par PALIERS (append-only jusqu'au seuil, UN rebuild
   assumé) au lieu du glissement à chaque tour, dégradation d'image au
   rebuild seulement (le cache rend l'image gardée gratuite).
2. Le bloc des 13-14 outils (1 241 tok) devient **quasi gratuit dès le
   tour 2** → la sélection d'outils contextuelle du backlog perd son
   urgence.
3. TTFT attendu en régime append : **~1,3-1,5 s** (plancher + préfill du
   seul tour nouveau) au lieu de 2,2-2,4 s. Le critère R1 ≤ 1 s reste
   au-dessus du plancher serveur (~1,2 s) — upstream.
4. Un moment VLM proactif coûte SON temps + l'éviction (le tour suivant
   repasse au plein tarif ~+2-3 s) → moments à déclencher de préférence
   quand le dialogue est calme ; à re-mesurer si FLM passe multi-cases.

**Chantier 0 livré et MESURÉ le 2026-07-08 (même jour)** : discipline
append-only implémentée (système stable au rebuild, conscience/attentes
dans l'en-tête `[horodatage | …]` du dernier message user, fenêtres par
paliers, fenêtre voix persistante). Banc de bout en bout (`waly.exe chat`,
catalogue 13 outils + souvenirs, base jetable) : **tour 1 = 3,10 s (plein
préfill assumé), tours 2-3 = 1,43 s / 1,37 s** — temps de tour COMPLET,
génération comprise. Était ~2,4-3 s par tour avant. Les 3 exes rebuiltés
passent SAC (waly release, waly-voice release + feature `service`,
waly-desktop debug).

## Écran R5 — capture native + OCR + coût VLM (mesuré 2026-07-09, GATES 1-3 du plan R5)

Plan : `docs/PLAN-2026-07-09-R5-waly-sight-ecran.md`. FLM qwen3vl-it:4b sur
52626 (pinné, vérifié `Win32_Process` avant tout appel — pas de hot-swap).

**GATE 1 — capture d'écran native sous SAC : VERT.** Voie GDI (`BitBlt` +
`GetDIBits`) via la crate `windows` 0.61 (features `Win32_Graphics_Gdi`/
`_Dwm`/`Win32_UI_HiDpi`/`_WindowsAndMessaging`) — DLL système (gdi32/user32/
dwmapi) chargées par l'exe, **jamais un exe tiers** (piège 3). `waly-sight/
src/screen.rs` : `capture_screen()` (écran principal, `GetDC(None)`),
`capture_active_window()` (`GetForegroundWindow` + `DwmGetWindowAttribute`
DWMWA_EXTENDED_FRAME_BOUNDS pour exclure l'ombre → BitBlt de la région),
`rendre_conscient_dpi()` (`SetProcessDpiAwarenessContext` PER_MONITOR_V2 →
pixels réels, pas la résolution logique). Mesuré (exe **debug** windows-gnu,
`waly-sight.exe snap-screen full|window`) : **exe passe SAC du premier coup**
(exit 0), écran entier **1920×1080 en 75 ms**, fenêtre active **1920×1032 en
78 ms**, couleurs justes, texte net, **fenêtre Chrome accélérée GPU capturée
sans noir**. `PrintWindow`/`PRINT_WINDOW_FLAGS` PAS exposés dans windows 0.61
→ abandonnés au profit du BitBlt de région (la fenêtre au premier plan est
au-dessus). Limite connue : une fenêtre MASQUÉE ne serait pas rendue —
l'upgrade `Windows.Graphics.Capture`/DXGI Duplication (mêmes DLL, + bordure
système) reste l'option du chantier 0 si le terrain le réclame.

**GATE 2 — OCR local sous SAC : VERT (option B, tranchée par Michée).**
D'abord amorcé : `Windows.Media.Ocr` (OCR Windows intégré) marche et est
SAC-safe (WinRT, pas d'exe) mais SANS pack fr-FR (en-US + ar-SA). Michée a
choisi **B (OCR ONNX embarqué, aucune install système)**. Constat en route :
**aucun modèle rec latin/FR ONNX propre n'existe clé-en-main** (RapidOCR/
SWHL = chinois/anglais/japonais/coréen). Combo retenu (léger, ASCII-fort) :
**PaddleOCR PP-OCRv4 det (4,7 Mo) + PP-OCRv3 en rec (9 Mo) + `en_dict.txt`
(95)**, dans `engines/models/ocr/` (hors git), sur l'onnxruntime.dll signée
déjà là. Pipeline maison `waly-sight/src/ocr.rs` : DBNet (carte de proba →
seuil 0,3 → composantes connexes 8-voisins → boîtes ; le texte d'écran est
axis-aligné, pas besoin du contour/unclip OpenCV) + CRNN (crop h=48 → CTC
glouton, classes 97 = blanc + 95 dico + espace). **Mesuré (banc
`waly-sight.exe ocr`, debug, écran IDE DENSE 1920×1080 = pire cas)** :
chargement 1,7 s (2 sessions, une fois), **lecture 2,54 s, 116 zones, conf
moy 0,78**. Qualité vérifiée contre le PNG : anglais/code/UI **solide**
(`EXPLORER`, `OPEN EDITORS`, `PROBLEMS TERMINAL`, `Finished dev profile
[unoptimized + debuginfo]`, noms de fichiers), petit texte parfois brouillé,
**français accentué faible ASSUMÉ** (« Considerer », accents tombés) — le
VLM couvre le français et la compréhension (routage OCR-first → VLM du
chantier 2). Lecture < 3 s tenue même sur écran dense (fenêtre active /
release = marge). Lever la faiblesse FR = un modèle rec latin (à convertir).
Aucun pixel persisté (lecture en mémoire).

**GATE 3 — coût VLM d'une capture écran : VERT AVEC CONTRAINTE.** Cliché
1920×1080 → **720p (1280×720) JPEG q80 = 96 Ko → 949-957 tokens image** sur
qwen3vl. Compréhension EXCELLENTE (a lu l'appli, l'URL, la nav, le nom+
localisation de l'utilisateur, le disclaimer — tout juste). **Mais la
longueur de réponse fait tout** : réponse verbeuse (220 tok, plafond atteint)
= **17,58 s** ✗ ; réponse concise (33 tok, prompt « en UNE phrase » +
`max_tokens` 80) = **5,81 s** ✓ < 8 s. Le DÉCODAGE domine (~16-19 tok/s NPU),
pas le préfill (~950 tok ≈ 1,5 s). **Contrainte gravée pour R5 : les réponses
écran doivent être BRÈVES** (prompt court + `max_tokens` ~80) ; et l'**OCR-
first** évite le décodage VLM entier pour « lis-moi ce texte » (le critère
< 8 s tient trivialement sur ce cas dominant). Aucun pixel persisté : cliché
redimensionné/encodé en mémoire, envoyé au FLM LOCAL, supprimé après mesure.

## Modèles locaux (C:\waly\engines\models\, jamais en git)
- `silero_vad.onnx` (2,3 Mo) — VAD v5, depuis github snakers4/silero-vad.
- `onnxruntime.dll` (12,4 Mo, v1.22.0) — **DLL officielle signée Microsoft**
  (NuGet Microsoft.ML.OnnxRuntime, signature Authenticode vérifiée) → passe SAC.
  Chargée dynamiquement par waly-voice (ort load-dynamic, jamais de link statique).
- Qwen3-VL-4B GGUF Q4_K_M + mmproj (pour moteur B quand nos binaires seront signés).
- `ocr/` (R5, OCR écran, ~14 Mo) — PaddleOCR via `ort` : `det.onnx`
  (PP-OCRv4 det, HF `SWHL/RapidOCR/PP-OCRv4/ch_PP-OCRv4_det_infer.onnx`),
  `rec_en.onnx` (PP-OCRv3 en rec, `…/PP-OCRv3/en_PP-OCRv3_rec_infer.onnx`),
  `en_dict.txt` (95 lignes ASCII, PaddleOCR `ppocr/utils/en_dict.txt`).
  Surcharge du dossier : `WALY_OCR_DIR`. FR accentué faible (rec anglais) —
  couvert par le VLM ; lever = un modèle rec latin ONNX.

## Référence latence (mesuré 2026-07-03, AVANT reconstruction)
Ollama qwen3:8b 100 % CPU : préfill 38,8 tok/s, décodage 9,9 tok/s ;
qwen3:4b CPU : préfill 77,5 tok/s, décodage 13 tok/s (sous pression mémoire).
