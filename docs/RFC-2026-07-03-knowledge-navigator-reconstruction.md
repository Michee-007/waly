# RFC — Waly Knowledge Navigator : reconstruction A→Z (2026-07-03)

> Suite de l'audit `AUDIT-2026-07-03-diagnostic-etat-de-lart.md`. Décisions d'architecture
> pour la reconstruction, intégrant la vision finale : assistant local complet —
> voix, vision, mémoire, agents « cowork », computer use — avec cloud opt-in.

## D1 — Monorepo : OUI

Quatre dépôts (`waly`, `waly-backend`, `waly-desktop`, `waly-research`) pour un produit
mono-binaire est une friction sans bénéfice : contrats client/serveur désynchronisés
(trois clients, trois schémas d'auth constatés à l'audit), doc éparpillée, pas de CI
unifiée. Cible :

```
waly/
├─ apps/desktop/           # Tauri 2
│   ├─ src/                # UI webview (React/TS — le SEUL TypeScript restant)
│   └─ src-tauri/          # shell + intégration des crates
├─ crates/
│   ├─ waly-core/          # orchestrateur : chat, outils, mémoire, sûreté, traces
│   ├─ waly-voice/         # VAD + STT + TTS (sherpa-rs), barge-in
│   ├─ waly-sight/         # capture écran, OCR, client VLM
│   └─ waly-hands/         # computer use : navigateur (CDP), input OS, sandbox
├─ engines/                # scripts d'install/config llama-server + FastFlowLM (modèles hors git)
├─ docs/                   # ADR, RFC, mesures (migration des ADR existants)
└─ lab/                    # ex-waly-research + spikes Python jetables
```

## D2 — Langage : Rust pour tout ce qui shippe, TS uniquement dans la webview

L'objection de Michée est retenue. L'argument « Node n'est pas le goulot » reste vrai
pour la *latence* (le LLM domine), mais il est à côté du sujet pour un *produit
distribué* :

| Critère produit | Node sidecar (actuel) | Rust in-process (cible) |
|---|---|---|
| Process backend séparé | oui (node.exe ~80 Mo shippé) | **aucun** — crate dans le process Tauri |
| RAM à vide de l'orchestrateur | ~150-200 Mo (Express+PGlite) | ~10-30 Mo |
| Accès OS pour computer use (capture, injection input, hooks) | FFI pénible | natif |
| Voix in-process | non (service Python séparé) | **sherpa-rs** : VAD+STT+TTS dans le même binaire (supporte Pocket TTS 2026, Supertonic) |
| Surface d'attaque / sandboxing | 2 runtimes + IPC | 1 binaire signé |
| Tauri | impose déjà du Rust | même langage partout |

L'inférence, elle, ne vit dans AUCUN langage applicatif : ce sont des moteurs natifs
hors-process (llama-server, FastFlowLM) parlés en HTTP/OpenAI-compat — pattern shippé
par Jan.ai (Tauri + plugin Rust + sidecar llama.cpp). Le choix Rust ne concerne que
l'orchestrateur — et il supprime le process backend au lieu de le réécrire.

**Ce qu'on garde du code Node** : pas le code, les acquis — mur financier, validation
de dispatch, gate de risque, discipline de cache KV, profil matériel. Réécrits comme
specs + tests dans `waly-core`. La base passe de PGlite (Postgres-WASM porté par Node,
plus de raison d'être) à **SQLite (rusqlite) + sqlite-vec**.

**Python** : reste dans `lab/` pour prototyper (écosystème ML), ne shippe jamais.

## D3 — Les moteurs : 2 backends, 1 API

- **Moteur NPU (prioritaire)** : FastFlowLM — qwen3:4b texte (19,6 tok/s décodage,
  615 tok/s préfill mesurés sur ce silicium) **et VLM `qwen2.5vl-it:3b` sur NPU**
  (support confirmé, ctx jusqu'à 256k). API OpenAI-compat.
- **Moteur fallback (toutes machines)** : llama-server Vulkan/CPU avec **Qwen3-VL-4B
  GGUF** — un seul modèle multimodal texte+vision (support mtmd/llama-server confirmé),
  KV q8_0 + flash attention.
- `waly-core` parle une seule API (OpenAI-compat) et route selon le profil matériel
  détecté — le `hardware-profile.js` actuel avait la bonne idée ; il devient un crate.
- Tool-calling natif qwen3 (fin du bloc de 2 500 tokens d'outils en prompting).

## D4 — Les quatre capacités du Knowledge Navigator

### Voix (P1)
Cascade Rust in-process : Silero VAD → STT streaming pendant la parole (Parakeet-TDT
0.6B v3, WER FR ~5 %) → fin de tour sémantique (~20 ms) → LLM streaming première
clause → TTS streaming FR (Pocket TTS ~200 ms / Piper). Barge-in : la lecture s'arrête
quand le VAD détecte la voix. Cible mesurable : **≤ 1 s fin de parole → premier son**
(état de l'art local : 0,5-0,8 s).

### Vision (P2) — caméra ET écran, en pyramide de perception

La vision couvre deux capteurs, avec la même architecture à coût étagé :

**Caméra — le mode « appel vidéo » (l'innovation du produit).** Quand Michée active la
caméra, Waly le voit comme dans un appel :
- **Étage 1 — perception continue (ms, CPU/NPU, toujours active quand la caméra l'est)** :
  présence, regard, gestes, signaux émotionnels via des modèles légers type MediaPipe
  (468 landmarks visage ~ms ; émotion ~2 ms/image, modèle 29M). Sortie : un petit
  vecteur d'état (« présent, attentif, sourit, montre un objet ») injecté dans le
  contexte du tour — quelques dizaines de tokens, pas des images.
- **Étage 2 — compréhension ponctuelle (VLM, 2-4 s NPU)** : sur déclencheur (« regarde
  ça », objet tendu à la caméra, changement de scène) une frame part au VLM résident.
  Jamais de flux continu vers le VLM — inutile et hors budget.
- **L'avatar Waly (déjà dans l'UI : WalyAvatar/mood) devient l'autre côté de l'appel** :
  son humeur réagit à l'état perçu — c'est la boucle de présence.
- Référence marché : Tavus CVI fait exactement cela (perception « Raven » : émotions,
  attention, réponse < 600 ms) mais **en cloud, B2B, biométrie uploadée**. En local,
  le créneau est vide — et la caméra est le capteur que le grand public n'acceptera
  qu'en local.

**Écran** : capture → **OCR léger d'abord** (écrans texte = 90 % des cas, quasi gratuit
CPU) → **VLM seulement si la question exige la compréhension visuelle** (layout, images,
UI). Screenshot 1080p ≈ 1-2k tokens → préfill NPU ≈ 2-4 s.

Les deux capteurs partagent le **même modèle multimodal résident** (pas de 2ᵉ VLM) ;
les modèles de perception continue ajoutent < 300 Mo au budget mémoire.

### Cowork agentique (P3)
Boucle agentique dans `waly-core` (portage de l'actuelle, qui marche) + traces +
approbations héritées. Tâches longues en arrière-plan avec points de contrôle humains —
le modèle 4B planifie correctement des tâches scopées, pas des projets ouverts.

### Computer use (P4) — les chiffres imposent l'humilité
État de l'art open ≤8B sur OSWorld : OpenCUA-7B ~28 %, EvoCUA-8B 46 % (mais 10,9 %
sur OpenComputer, plus dur) ; UI-TARS-1.5-7B 42,5 %. L'humain : ~72 %. Conclusion
produit : **opérateur autonome — un cowork supervisé sur des workflows scopés** :
- **Web d'abord, par DOM** (arbre d'accessibilité via CDP, pattern browser-use —
  ~10× moins de tokens que les pixels, et c'est là que les petits modèles réussissent) ;
- pixels/grounding en secours (OmniParser v2 ou UI-TARS petit) pour les apps sans DOM ;
- **workflows enregistrés et rejoués** (trajectory caching) : la 1ʳᵉ exécution est
  supervisée pas-à-pas, les suivantes ne re-consultent le LLM qu'aux points de décision ;
- latence attendue ~4-6 s/action sur cette machine (préfill NPU) — viable en supervisé.

## D5 — Budget mémoire signé (15 Go)

| Résident | RAM |
|---|---|
| Modèle multimodal 4B Q4 (texte+vision) + KV 8k q8_0 | ~3,5-4 Go |
| STT Parakeet int8 + VAD + TTS Pocket/Piper | ~1,5 Go |
| waly (Tauri + core Rust + webview) | ~0,5-0,8 Go |
| **Total stack** | **~6 Go** |
| OS Windows + marge apps utilisateur | ~9 Go |

Règle : un seul LLM résident. Le 8B n'entre dans ce budget sur aucun scénario — acté.
Exécution 100 % Windows natif ; WSL réservé au dev (et `.wslconfig` plafonné à 4 Go).

## D6 — Positionnement (recherche marché, complétée inline le 03/07 après-midi)

- **Le créneau exact est vide** : personne ne combine voix temps réel + vision d'écran
  + mémoire + agents en 100 % local grand public. screenpipe (YC S26, 16k stars,
  app 300-400 $ lifetime) prouve la demande de mémoire d'écran locale — mais c'est une
  mémoire sans cerveau (il se branche sur Claude/ChatGPT via MCP). Highlight/Cluely
  sont cloud. Jan/LM Studio sont des chat UIs sans agentique ni vision d'écran.
- **Vent de plateforme** : AMD (Lemonade/FastFlowLM), Microsoft (Windows AI Foundry),
  Qualcomm poussent les apps IA locales sur NPU — être une vitrine NPU est un canal.
- **Risques principaux** : (1) plafond de qualité des ≤8B pour l'agentique ouverte —
  mitigé par le design « supervisé + scopé » et le cloud opt-in ; (2) absorption par
  l'OS (Recall/Copilot) — mitigé par le cross-vendor et la confidentialité radicale ;
  (3) fragmentation matérielle — mitigé par les 2 moteurs + profils.

## D7 — Propositions de positionnement marché

**Identité produit : « le compagnon qui te voit » — pas un outil, une présence et un système personnel d'intelligences.**
L'expérience définissante est le mode appel : tu actives caméra + voix et tu parles à
Waly comme en visio, il te voit, réagit à ton état, se souvient. Personne n'offre cela
en local ; Tavus le vend en cloud B2B, preuve que l'expérience a de la valeur.

1. **Segment cœur — grand public confidentialité, FR/EU d'abord.** La caméra et le
   micro permanents ne sont acceptables qu'en 100 % local : le moat technique EST le
   message (« rien ne quitte ta machine, jamais »). Voix FR soignée = différenciation
   immédiate face aux produits anglo-centrés. RGPD/souveraineté = vent porteur EU.
2. **Segment payant — professions à données sensibles** (avocats, médecins, RH,
   finance) : le cowork qui voit l'écran et les documents sans fuite. Modèle prouvé
   par screenpipe : app lifetime 200-400 €, cœur open source (MIT) pour la confiance
   et la communauté.
3. **Canal plateforme — vitrine NPU.** AMD (Lemonade/FastFlowLM) et Microsoft poussent
   activement les apps NPU et manquent d'apps phares : être LA démo Ryzen AI grand
   public = distribution et co-marketing gratuits.
4. **Modèle économique** : open-core (crates MIT) + app payante lifetime + plus tard
   cloud opt-in BYOK (l'utilisateur branche sa propre clé Anthropic/autre pour les
   tâches dures — jamais nos serveurs).

## Feuille de route révisée (v2 — intègre la vision caméra)

| Phase | Contenu | Critère de sortie mesuré |
|---|---|---|
| **R0** | Monorepo + substrat : FastFlowLM/llama-server natifs, qwen3:4b, bench A/B | chat texte < 2 s/tour à chaud |
| **R1** | `waly-voice` Rust (cascade complète + barge-in) | ≤ 1 s fin de parole → premier son |
| **R2** | `waly-core` Rust minimal : chat, mémoire SQLite+vec, 15-20 outils natifs, sûreté portée | parité fonctionnelle desktop, prompt < 1k tok |
| **R3** | Desktop rebranché : UI actuelle sur le core Rust in-process | app installable double-clic, RAM à vide < 300 Mo |
| **R4** | **Mode appel** : perception caméra continue (présence/regard/gestes/émotion) + VLM ponctuel + avatar réactif, fusionné avec la voix | session « visio » fluide : réaction perçue < 500 ms, moment VLM < 8 s |
| **R5** | `waly-sight` écran : OCR + VLM NPU, « regarde mon écran » (réutilise le VLM de R4) | réponse écran < 8 s |
| **R6** | `waly-hands` : web par DOM supervisé + workflows rejouables | 3 workflows réels fiables |
| **R7** | Mémoire ambiante opt-in (style screenpipe) + cloud opt-in BYOK | — |

Le mode appel passe avant l'écran : c'est lui qui définit le produit (R4 = le moment
« wow » démontrable), et l'écran réutilise le VLM déjà en place. Chaque phase livre
seule de la valeur ; on ne passe pas à la suivante sans le critère de sortie.

## Sources clés (complément à l'annexe B de l'audit)

- llama.cpp multimodal/mtmd + Qwen3-VL GGUF : github.com/ggml-org/llama.cpp/blob/master/docs/multimodal.md ; huggingface.co/Qwen (GGUF officiels)
- FastFlowLM VLM sur NPU : fastflowlm.com/docs/models/qwen/ (`flm run qwen2.5vl-it:3b`) ; intégration Lemonade (oct. 2025), Linux (mars 2026)
- Computer use ≤8B : OpenCUA (arxiv 2508.09123, 7B ≈ 28 % OSWorld-Verified) ; EvoCUA-8B 46,1 % OSWorld mais 10,9 % OpenComputer (arxiv 2601.15876, 2605.19769) ; UI-TARS-1.5-7B 42,5 (github.com/bytedance/ui-tars)
- Web agents : browser-use 89,1 % WebVoyager (frontier, DOM+vision) ; WebVoyager saturé/benchmaxxé (michaellivs.com/blog/state-of-browser-use-2026)
- Voix Rust : crates sherpa-rs / sherpa-onnx (docs.rs) — VAD, STT, TTS dont Pocket TTS (jan. 2026) et Supertonic
- Marché : github.com/screenpipe/screenpipe (YC S26, 16k+ stars, MIT core, app payante)
