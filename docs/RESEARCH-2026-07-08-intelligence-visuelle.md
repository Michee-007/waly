# Recherche — Intelligence visuelle & fusion réelle (2026-07-08)

> Instruction du verdict R4 (Michée) : « il n'a pas vraiment d'intelligence
> visuelle, il ne fait que me dire qu'il perçoit mon visage » + « le cercle-là
> c'est naze, il faudrait quelque chose qui réagisse ». Cette note synthétise
> l'état de l'art (5 volets de recherche, sources datées) et ce que NOTRE
> matériel permet. Le plan qui en découle :
> `PLAN-2026-07-08-R4.5-intelligence-visuelle.md`.

## 1. Comment les références font la « vision continue » — personne ne fait de flux

- **Gemini Live (officiel)** : vidéo = **1 frame/s max, 258 tok/frame**
  (66 en `media_resolution` low), audio 32 tok/s, contexte 128k, sessions
  audio+vidéo **~2 min sans compression** puis *context window compression*
  (fenêtre glissante qui jette/résume les vieux tours). Sources :
  ai.google.dev/gemini-api/docs/live-api, firebase.google.com/docs/ai-logic/
  live-api/limits-and-specs, docs/video-understanding. La console de référence
  open source envoie même du **0,5 fps**. Gemini « voit » une diapositive
  toutes les 1-2 s — et paie 300 tok/s de flux.
- **OpenAI Realtime (officiel, `gpt-realtime`, 28 août 2025)** : **PAS de
  vidéo du tout** — « more like adding a picture into the conversation,
  rather than a live video stream » et surtout **« your application decides
  which images to share and when »**. Images discrètes événementielles dans
  l'historique = exactement notre `Msg::UserImage`. Notre architecture est
  validée par la référence en production.
- **Littérature 2024-2026 (streaming video understanding)** : la voie
  dominante est **un frontend léger par frame qui décide QUAND réveiller le
  VLM lourd** — VideoLLM-online (CVPR 2024, « Streaming EOS » : le modèle
  apprend quand parler), StreamBridge (NeurIPS 2025 : petit modèle
  d'activation), LION-FS (CVPR 2025 : Fast-Slow), Dispider (CVPR 2025 :
  perception/décision/réaction asynchrones), Flash-VStream (mémoire STAR :
  flux compressé en mémoire hiérarchique, questions asynchrones). Sharingan
  (Microsoft, arXiv 2411.08768) : **décrire le DELTA entre deux frames, pas
  l'état** — la réponse littérale à « constater ≠ comprendre ».

## 2. La mémoire visuelle s'écrit en TEXTE horodaté (pas en pixels)

Patterns convergents (screenpipe, Generative Agents, MemGPT, Socratic
Models, ProVideLLM ICCV 2025 « long terme verbalisé, court terme visuel ») :

- **screenpipe** (Rust, SQLite) : capture ÉVÉNEMENTIELLE (événements OS,
  filet ~5 s d'inactivité), texte extrait → SQLite FTS5, **jamais de pixels
  dans le contexte**. ~600 Mo RAM, 5-10 % CPU en 24/7.
- **Generative Agents (UIST 2023)** : *memory stream* = observations en
  langage naturel horodatées (« [14:02] il tient une tasse »), récupération
  récence×importance×pertinence, et **réflexion périodique** : ~100
  observations → inférences de plus haut niveau re-stockées. **La réflexion
  est ce qui transforme l'étiquette en compréhension** (« Michée compare
  deux écrans depuis 10 min, il cherche quelque chose »).
- **MemGPT** : contexte chaud (résumé roulant compact) / archive froide
  (SQLite) — compatible avec notre `user_memory`/`inject_context` existants.
- Budget chez nous (préfill 618 tok/s si pas de cache) : bloc scène de
  **150-300 tok = +0,25-0,5 s de TTFB/tour** (soutenable) ; une liste brute
  de 30 observations (~900 tok ≈ +1,5 s) est à proscrire.

## 3. Détection de changement bon marché (le déclencheur des moments)

| Technique | Coût | Usage |
|---|---|---|
| Événements waly-sight (YuNet+FER+hystérésis) | déjà payé (~34 ms/cycle) | présence/attention/humeur |
| Diff d'histogramme (canal Y) / dHash-pHash 64 bits sur vignette | **< 1 ms** (le resize = 95 % du coût du hash → hasher la vignette 320 déjà produite ≈ gratuit) | changement de scène/cadrage/objet |
| Embedding CLIP/MobileCLIP int8 + cosinus | 20-80 ms CPU | changement *sémantique*, si le filtre 1 déclenche |
| VLM 4B NPU | ~5 s | RARE : seulement si les filtres confirment |

Cascade partout dans la littérature : **personne ne pousse toutes les frames
dans un VLM**. Notre hystérésis waly-sight EST déjà l'étage 1.

## 4. Modèles locaux : pas d'omni, un petit VLM d'appoint est plausible

- **Omni (audio+vision+texte joints) hors de portée** : Qwen2.5-Omni-7B
  ~31 Go BF16, Qwen3-Omni 30B-A3B > 18 Go même quantifié, MiniCPM-o 8B
  ~5-6 Go à lui seul, Gemma 3n vision/audio non câblé llama.cpp Windows.
  → **La fusion se fait par orchestration dans le CONTEXTE** (notre principe
  R4), pas par un modèle unique. Piste audio-in native la plus proche :
  Gemma4-audio au catalogue FLM.
- **Petits VLM d'appoint réalistes** (captions ~0,5-1 s hors NPU, chiffres
  VÉRIFIÉS sur les cartes HF le 2026-07-08) :
  **SmolVLM-500M** (GGUF ggml-org : Q8 437 Mo + mmproj 109 Mo ≈ **0,55 Go** ;
  démo webcam llama.cpp ngxson/smolvlm-realtime-webcam, ~0,5 s/caption
  laptop), **LFM2-VL-450M/1.6B** (GGUF OFFICIELS Liquid : 450M Q8 379 Mo ;
  1.6B Q4 696 Mo ; tuilage déterministe = latence prévisible),
  **Moondream2** (1.9B, Q4 ~1-1,2 Go, GGUF + ONNX int8, fort en
  detect/point), **FastVLM-0.5B** (ONNX officiel communautaire, TTFT
  revendiqué 85× vs LLaVA-OV et 5,2× vs SmolVLM — l'outsider anti-préfill,
  ⚠ licence apple-amlr, pas de GGUF). Tous coexistent avec le 4B NPU dans
  ~0,5-1,5 Go. **Trouvaille : Qwen2.5-Omni-3B a un GGUF ggml-org (Q4 2,1 Go
  + mmproj 1,54 Go ≈ 3,6 Go) = le SEUL « yeux+oreilles » réel sous
  llama.cpp** (audio+vision IN, pas de voix out) — déborde le budget
  d'appoint, à garder pour une phase future.
  iGPU 840M (4 CU, gfx1152) : aucun bench publié ; extrapolation depuis la
  780M (12 CU : 30-50 tok/s sur 3B Q4 Vulkan) → un 500M devrait décoder
  > 30 tok/s, le vrai coût par frame = l'encodage mmproj (centaines de ms).
  ⚠ Ollama-Vulkan a un bug de détection gfx1152 connu (ollama #14562) —
  llama.cpp Vulkan direct est la voie sûre. Éliminés : Gemma 3n (llama.cpp
  = texte seul, confirmé), Phi-4-multimodal (pas de llama.cpp), MiniCPM-o
  (omni = fork, 4,7 Go), Qwen3-Omni/GLM-4.1V (taille/support).
  ⚠ Chez nous, chemin d'exécution SOUS SAC : DLL chargée dynamiquement
  (llama.dll Vulkan via libloading, ou ONNX via ort load-dynamic) — jamais
  d'exe tiers (llama-server.exe est bloqué).
- **Cohabitation NPU+iGPU mesurée viable** (Strix Halo, 2026,
  sleepingrobots.com) : sous contention **iGPU -14,1 % décodage, NPU -5,8 %
  seulement**, TTFT +15-19 % (bus saturé au préfill) ; simultané = 1,42× de
  gain temps-mur vs séquentiel. Notre 840M (4 CU) est faible en
  préfill/compute — aucun bench publié sur ce GPU : à mesurer localement.
- **XDNA 2 multi-modèles** : partitionnement spatial par colonnes documenté
  (jusqu'à 8 contextes) — notre cohabitation FLM-LLM + FLM-ASR en est la
  preuve empirique. CLIP-NPU officiel AMD existe (amd/NPU-CLIP-Python),
  YOLO NPU à 22 ms/inférence (Vitis AI EP).

## 5. ⚠ FLM : le cache de préfixe a PEUT-ÊTRE bougé upstream (à re-mesurer)

Release notes FLM (~mai 2026, github.com/FastFlowLM/FastFlowLM/releases) :

- **v0.9.41** : bug corrigé — « les schémas d'outils étaient ré-injectés
  dans le KV cache à chaque tour » (notre bloc de 14 outils re-payé !).
- **v0.9.43** : « cached conversations » fiabilisées — **diff au niveau
  token après templating**, raisonnement exclu du KV cache = réemploi de KV
  inter-requêtes.
- Des versions ≥ v0.9.44 existent (tool-calling amélioré).

Notre mesure « aucun cache de préfixe » date du 2026-07-04. Soit elle
précède le correctif effectif, soit le diff ne s'activait pas (thinking dans
l'historique, templating). **Si ça se confirme : le TTFT multi-tours
s'effondre, le récit visuel devient quasi gratuit, et la dérive gravée
« ≤ 1 s sans chemin interne » peut se rejouer.** Par ailleurs AMD assume le
sujet dans tout l'écosystème 2026 (KV cache reuse Ryzen AI SW 1.7.1,
« continuous decoding + conversation rewind » ; Lemonade v10 fait de FLM le
backend NPU officiel d'AMD). Précision doc FLM : « un modèle à la fois »
vaut PAR TYPE (LLM + embedding + ASR peuvent coexister).

Veille connexe (question de Michée) : **TurboQuant** (Google Research,
mars 2026) = quantization vectorielle du KV cache à ~3 bits (clés 3-bit /
valeurs 2-bit), training-free, précision quasi indistinguable du FP16.
Pertinent pour NOUS le jour où FLM ou llama.cpp l'implémentent (KV 8k
divisé par ~5, décodage borné bande passante = potentiellement plus rapide
sur long contexte) — mais aujourd'hui l'implémentation publique vise
Triton/vLLM (GPU datacenter), rien sur NPU/llama.cpp : levier UPSTREAM à
suivre, pas actionnable localement. Notre goulot immédiat reste le
re-préfill (cache de préfixe), pas la taille du KV.

## 6. Éclipse/avatar réactif : la recette consensuelle

- **Recette de mapping (Siri orb, ChatGPT blob, Gemini)** : **l'ÉTAT pilote
  la palette et la vitesse de base** (idle / écoute / réflexion / parole),
  **l'audio pilote la modulation** (RMS lissé par lerp 0,05-0,1 — le brut
  vibre désagréablement). Basses → pulsation, médiums (voix) → turbulence,
  état « réflexion » = mouvement interne SANS entrée audio (pattern « inner
  activity » de Google Design).
- **Implémentation légère** : Canvas 2D suffit (blob = cercle dont le rayon
  est modulé par bruit simplex par angle, < 2 ms/frame) ; sinon UN fragment
  shader WebGL (~200 lignes, sans Three.js). Références : kopiro/siriwave,
  aguscruiz/voiceorb (4 états explicites, FFT médiums, MIT).
- **Terrain vierge différenciant** : AUCUN grand assistant ne fusionne la
  perception caméra DANS l'avatar (tous juxtaposent viewfinder + orbe).
  Faire réagir l'éclipse à waly-sight (présence → halo, position du visage
  → parallaxe, humeur → teinte/tempo, moment VLM → anneau de regard) est
  unique et trivial à brancher — les signaux arrivent déjà à l'UI.
- Leçon UX OpenAI fin 2025 : l'orbe plein écran abandonné au profit d'un
  visuel intégré au fil (rupture de contexte) — à garder en tête.
- **Le trou chez nous** : l'amplitude micro/TTS et l'état de tour vivent
  dans le processus voix, AUCUN canal ne les remonte à l'UI desktop
  (le loopback ne va que desktop→voix).

## 7. Synthèse : réaliste chez nous vs hors de portée

**Réaliste (chiffres à l'appui)** :
1. Pyramide événementielle : waly-sight (payé) + pHash vignette (< 1 ms) →
   moments VLM rares et contextualisés (~5 s, mesuré) — architecturalement
   IDENTIQUE au schéma cloud, aux latences près.
2. Mémoire visuelle en texte horodaté + résumé roulant : 150-300 tok
   injectés = +0,25-0,5 s/tour (0 si le cache FLM se confirme).
3. Réflexion périodique entre les tours (appel LLM texte hors tour) —
   c'est elle qui produit « tu sembles chercher quelque chose » au lieu de
   « je te vois ».
4. Éclipse réactive Canvas/shader pilotée par état + perception + amplitude.
5. (À mesurer) Petit VLM d'appoint 450-500M hors NPU pour des captions ~1 s.

**Hors de portée locale aujourd'hui** :
1. Modèle omni fusionné.
2. Vision 4B à ~1 fps (chaque image = re-préfill ; même le cloud n'envoie
   qu'1 fps — mais lui a le cache).
3. Réaction visuelle RICHE < 1 s (chemin réaliste : réaction légère < 500 ms
   par l'éclipse/événements, compréhension riche en 3-5 s).

**Réserve de fiabilité** : volet modèles locaux (§4 tailles/latences) issu
de connaissances jusqu'à janv. 2026 + recherches partielles — les chiffres
à re-vérifier avant décision sont dans les gates du plan (versions FLM,
tok/s réels 840M, GGUF LFM2-VL).
