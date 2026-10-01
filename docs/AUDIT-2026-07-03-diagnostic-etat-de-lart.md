# Audit Waly — diagnostic contre l'état de l'art (2026-07-03)

> Périmètre : `waly-backend` (Node, 17,6k lignes src), `waly-desktop` (Tauri 2 + React 19),
> `voice/voice_loop.py`, et la **topologie machine réelle**. Référentiel : état de l'art
> juillet 2026 pour un assistant vocal 100 % local sur CE matériel — pas les attentes
> écrites dans le projet.

## 1. Résumé exécutif

Le projet est « vert » selon ses propres critères (931 tests passent : 65 desktop + 866
backend) et « rouge » en tant que produit. Mesuré aujourd'hui sur le système réellement
en marche :

| Mesure (réelle, 2026-07-03) | Valeur mesurée | État de l'art sur cette machine |
|---|---|---|
| Tour de chat texte, question triviale (à chaud) | **4,5–9,2 s** | < 1 s |
| Tour de chat à froid / cache évincé | **14,6 s** (83 s de prefill si prompt complet ré-ingéré) | 1–2 s |
| Voix : fin de parole → premier son | **3–6 s+** (journal de bord) | **≤ 0,8 s** (barre « fluide » ≈ 0,6 s) |
| « Quelle heure est-il ? » | **Réponse fausse** (« 07h00 » = UTC arrondie, réel : 09h41) | exacte |
| RAM libre Windows en fonctionnement | **1,6 Go / 15 Go** | ≥ 4–6 Go de marge |
| Accélérateurs utilisés (iGPU 840M, NPU XDNA 2 ~50 TOPS) | **0 % — tout est 100 % CPU** | iGPU et/ou NPU actifs |

La cause n'est pas un bug isolé : c'est un **empilement de trois décisions structurelles**
qui plafonnent le produit quoi qu'on optimise par-dessus (§3). Les optimisations déjà
faites (cache KV, grain d'horloge, keep_alive, prompt voix 11,7k→3,6k tokens) sont
compétentes mais optimisent l'intérieur d'une enveloppe condamnée.

## 2. La machine réelle vs les hypothèses du projet

**Matériel** : AMD Ryzen AI 5 340 (3× Zen 5 + 3× Zen 5c, 12 threads) · iGPU Radeon
840M (RDNA 3.5, 4 CU) · **NPU XDNA 2 ~50 TOPS** · **15 Go de RAM** utilisables (pas 16) ·
bande passante mémoire ~90–128 Go/s · WSL2 en mode NAT.

**Loi physique de cette machine** (vérifiée par toutes les mesures publiées) : le
décodage LLM est borné par la bande passante mémoire partagée, quel que soit le moteur —
**~12 tok/s max pour un 8B Q4, ~15–20 tok/s pour un 4B Q4** (CPU, iGPU ou NPU). Ce que
les accélérateurs changent radicalement, c'est le **préfill** : NPU XDNA 2 mesuré à
**457–615 tok/s** (FastFlowLM, même NPU sur Ryzen AI 7 350) contre **39 tok/s ici en
CPU** — un facteur ×12–15 sur l'ingestion du prompt, qui est précisément le goulot
diagnostiqué (éviction KV → re-préfill).

Mesures d'inférence (Ollama 0.30.11, Windows, qwen3:8b Q4, `ollama ps` = **« 100% CPU »**) :

- **Décodage : 9,9 tok/s** → une réponse de 2 phrases (~45 tokens) coûte ~4,5 s de
  génération seule. C'est le plafond dur de TOUTE latence actuelle.
- **Prefill : 38–39 tok/s** (mesuré sur 32 et sur 3 223 tokens) → le prompt voix de
  3,6k tokens coûte **~90 s** à ré-ingérer à chaque éviction de cache. D'où la
  sensibilité extrême du système à l'éviction KV (déjà diagnostiquée dans les ADR).
- **Répartition RAM absurde** : la VM WSL2 réserve ~7,5 Go (50 % par défaut,
  `.wslconfig` absent) pour héberger… les fichiers sources et VS Code. L'inférence,
  elle, tourne côté Windows dans les ~7,5 Go restants avec l'OS. Résultat mesuré :
  1,6 Go libre, compression mémoire active, éviction KV fréquente.

**Hypothèses du projet contredites par la machine** :
- Les docs raisonnent sur « 16 Go » homogènes — en réalité c'est 2×7,5 Go cloisonnés.
- qwen3:8b (5,2 Go + KV) a été retenu comme modèle voix — il ne laisse aucune marge.
- L'iGPU a été écarté (« Radeon 840M incompatible » — vrai pour ROCm, **faux pour
  Vulkan** : llama.cpp/Ollama Vulkan natif Windows fonctionne sur RDNA 3.5 ; sur 4 CU
  le gain est surtout en préfill + libération du CPU, pas en décodage).
- Le NPU (l'argument d'achat de cette machine « Ryzen AI ») n'est jamais mentionné
  comme cible d'exécution. Inaccessible depuis WSL2 (pas de passthrough), il est
  exploitable depuis Windows natif : **FastFlowLM mesure qwen3:4b à 19,6 tok/s de
  décodage et 615 tok/s de préfill sur ce NPU exact** — resp. ×2 et ×15 vs l'actuel ;
  Lemonade Server (AMD officiel) fait de l'hybride NPU préfill + iGPU décodage.
- qwen3:8b est au-dessus du point optimal de cette machine : à ~12 tok/s plafond
  physique, le 8B ne sera JAMAIS fluide en voix ici ; le 4B (15–20 tok/s, 2,6 Go)
  est le bon point qualité/latence/mémoire.

## 3. Diagnostic racine — trois étages de causes

### Étage 1 (structurel) — le stack est à cheval sur deux OS

État constaté, vérifié process par process :

```
Windows                          WSL2 (VM, 7,5 Go confisqués, NAT)
├─ Ollama + qwen3:8b (CPU pur)   ├─ Code source (backend + desktop)
├─ node.exe spike-b-server.js    ├─ VS Code server (~1 Go)
│    └─ lit le code via          ├─ Tests, git, dev
│      \\wsl.localhost (9P ~11× ├─ [app Tauri dev → backend node WSL
│      plus lent, mesuré MSFT)   │    └─ OLLAMA à 127.0.0.1:11434
├─ voice_loop.py (micro/TTS)     │       → INJOIGNABLE (NAT) ]
└─ PGlite dans C:\temp-waly      └─ ~/waly-tauri.log (dernier run : 19/06)
```

Conséquences documentées (sources primaires Microsoft/llama.cpp, cf. §5) :
- **−50 % de RAM** pour l'inférence (défaut WSL2) ;
- **NPU inaccessible** (aucun passthrough accélérateur dans WSL2) ;
- **Vulkan iGPU dégradé/indisponible** dans WSL2 (Dozen incomplet) alors qu'il marche
  nativement sous Windows ;
- **I/O 9P ~11× plus lentes** pour le backend Windows qui lit son code dans WSL ;
- **L'app desktop Tauri ne peut PAS fonctionner** : lancée depuis WSL (NAT), son
  backend sidecar vise `127.0.0.1:11434` → code 000, injoignable (vérifié au curl).
  Le produit phare ne tourne que via un lanceur PowerShell manuel, hors de l'app.

### Étage 2 (substrat d'inférence) — mauvais modèle, zéro accélérateur

- **qwen3:8b sur 6 cœurs CPU = 9,9 tok/s** : en dessous du minimum pour de la voix
  fluide (une clause de 12 tokens = 1,2 s rien qu'à générer).
- Prefill CPU 39 tok/s : chaque tour ajoute l'historique + le bloc dynamique → même
  à cache chaud, plusieurs centaines de tokens neufs = 5–10 s avant le premier token.
- Aucun des trois accélérateurs de la machine n'est utilisé : pas de Vulkan (Ollama
  stock ne l'active pas), pas de NPU, pas de flash-attention/KV quantifié
  (`OLLAMA_KV_CACHE_TYPE` non configuré — vérifié : env vide).
- `keep_alive=-1` épingle 6,4 Go en permanence dans une machine à 1,6 Go de marge :
  la « solution » anti-éviction aggrave la pression qui cause l'éviction.

### Étage 3 (architecture logicielle) — un backend cloud rétrofité en local

Cartographie complète (agents d'exploration, chemins exacts en annexe A) :

- Le backend est un **serveur cloud multi-canal** (Telegram, WhatsApp, webhooks
  Stripe/GitHub, BullMQ, S3, Twilio — Twilio : dépendance **morte**, zéro usage)
  rétrofité en sidecar local par deux flags (`WALY_PROFILE=core`, `WALY_VOICE_PROFILE`).
  ~30–40 % du code et des dépendances sont inertes sur le chemin desktop.
- `src/llm/index.js` : **1 796 lignes** — routage, prompt, boucle agentique, dispatch
  de ~60 outils, sûreté financière et traces dans un seul fichier.
- Chaque tour de chat exécute séquentiellement : analyse d'intention + contexte
  (météo/calendrier/emails) + 4+ requêtes SQL + charge mentale Redis + historique +
  prompt ~3,6k tokens (voix) → **avant** le premier token LLM.
- ~60 outils injectés en « prompting » (~2 500 tokens) car le modèle local n'a pas de
  tool-calling natif géré — l'état de l'art 2026 utilise le tool-calling natif de
  qwen3 via Ollama/llama.cpp (format Hermes), qui évite ce bloc et le parsing regex.
- **Trois clients, trois contrats** : l'app Tauri (port aléatoire + `X-Sidecar-Token`),
  le lanceur PowerShell (port 3000 fixe, sans token), `voice_loop.py` (port 3000 en
  dur, aucun header). Desktop et voix ne peuvent pas partager la même instance.

### Bugs concrets confirmés (échantillon vérifié à la main)

| Bug | Preuve | Effet |
|---|---|---|
| Heure fausse | `prompt-builder.js:174` `user.timezone \|\| 'UTC'` + grain `hour` | « 07h00 » répondu à 09h41 ; jusqu'à 59 min d'erreur par conception, +2 h de TZ |
| App desktop sans LLM | `sidecar.rs` n'injecte ni `LLM_PROVIDER` ni `OLLAMA_BASE_URL` ; WSL NAT → 11434 injoignable | l'app UI ne peut jamais répondre en local |
| Capabilities Tauri | `capabilities/default.json` cible la fenêtre `"panel"`, la seule fenêtre s'appelle `"main"` | permissions (shortcut, shell) potentiellement non appliquées |
| Écran Réglages orphelin | `Reglages.tsx` codé, branché, **jamais rendu** (absent de `Shell.tsx`) | le toggle « cloud opt-in » est inatteignable |
| Plugin `opener` non enregistré | présent dans `Cargo.toml`, absent du Builder `lib.rs` | l'OAuth Google ne peut pas ouvrir le navigateur |
| Voice loop sans auth | `voice_loop.py` n'envoie aucun `X-Sidecar-Token` | ne marche que contre un backend lancé sans token |
| Micro codé en dur | `WALY_MIC_DEVICE=5` (mort depuis, cf. journal 03/07) | VAD muet, session vocale inutilisable |
| Pas de barge-in | `sd.play()+sd.wait()` synchrone | impossible d'interrompre Waly pendant qu'il parle |

## 4. Ce que dit l'état de l'art (juillet 2026, sources en annexe B)

**Latence voix** : la norme de fluidité est **< 600–800 ms** fin-de-parole → premier
son, atteinte en 100 % local par des cascades sur du matériel comparable ou inférieur
(Mac M-series < 800 ms ; RTX 4090 ~500 ms ; la cible « conversation cassée » est
> 1,5 s — Waly est à 3–6 s+). Recette convergente :

1. **STT en streaming pendant la parole** (coût résiduel ~0 à la fin du tour) —
   pas après le tour comme `voice_loop.py`.
2. **Fin de tour sémantique** (classifieur ~20 ms) au lieu d'un silence fixe de 800 ms
   qui s'additionne à chaque tour.
3. **LLM petit et accéléré, réponse streamée dès la première clause.**
4. **TTS CPU streaming à ~200 ms de premier son** (Kyutai Pocket TTS 100M, FR inclus,
   2 cœurs CPU ; ou Piper fr_FR ; Kokoro n'a qu'UNE voix FR notée B- par ses auteurs —
   le problème de phonémisation FR constaté au journal du 03/07 est structurel).
5. **Tout résident en mémoire** — donc un budget mémoire qui tient dans la machine,
   ce que le stack 8B actuel ne permet pas.

**STT français sur ce CPU** : Parakeet-TDT 0.6B v3 ONNX int8 (WER FR ~5 %, meilleur
que Whisper large-v3, ~10–20× temps réel sur ce CPU) ou Kroko ASR streaming
(partiels à ~320 ms). faster-whisper `base` actuel est en dessous de cet état de
l'art en qualité ET architecture (chunké, pas streaming).

**Speech-to-speech intégral** (Moshi, Qwen-Omni…) : AUCUN modèle S2S ne tient en
qualité sur 15 Go CPU/iGPU x86 en 2026 — la cascade reste le bon choix d'architecture.
Ce point du projet est validé.

**Mémoire** : `OLLAMA_KV_CACHE_TYPE=q8_0` + flash attention = KV ÷2 sans perte
mesurable ; qwen3:4b Q4 (2,6 Go) + STT + TTS + VAD ≈ **6 Go tout résident** — le seul
budget qui coexiste sereinement avec l'OS et l'UI dans 15 Go. Le 8B (~8–9 Go de stack)
est précisément ce qui force les compromis mortels actuels.

**Architecture d'app** : le pattern dominant (Jan.ai, LM Studio, Ollama desktop) est
**UI desktop + moteur d'inférence natif hors-process sur le même OS**. Tauri 2 est un
bon choix (validé) ; le split Windows/WSL2 est l'anti-pattern documenté.

## 5. Verdict

« Ça ne marche pas » a une racine unique en trois couches, et aucune n'est là où le
projet a concentré ses efforts :

1. **Topologie** : tant que l'inférence, la voix, l'UI et les données ne vivent pas
   sur le MÊME OS, l'app desktop ne peut pas parler au LLM, la moitié de la RAM est
   confisquée, et les deux accélérateurs de la machine restent morts.
2. **Substrat** : qwen3:8b/CPU à 9,9 tok/s est arithmétiquement incompatible avec de
   la voix fluide. Aucune optimisation de prompt ne franchit ce mur.
3. **Poids mort** : le backend transporte un serveur cloud entier pour servir un
   sidecar mono-utilisateur ; chaque tour paie ce passif en tokens et en requêtes.

Les 931 tests verts mesurent la conformité du code à sa propre spécification, pas la
viabilité du produit — d'où l'écart entre « tout passe » et « rien ne marche ».

## 6. Architecture cible (reconstruction A→Z)

> Principe : **un seul OS (Windows natif), un seul processus par rôle, un budget
> mémoire signé** (~6 Go inférence, ~9 Go pour l'OS + UI + marge).

```
Windows natif (15 Go)
├─ waly-engine (moteur d'inférence natif)
│    modèle : qwen3:4b-instruct Q4_K_M (2,6 Go) · KV q8_0 + flash-attn (~0,6 Go/8k)
│    voie A (recommandée) : FastFlowLM/Lemonade → NPU préfill 615 tok/s + décodage 19,6
│    voie B (fallback sûr) : llama.cpp/Ollama Vulkan natif (préfill ×2-3, CPU libéré)
│    tool-calling NATIF (plus de bloc outils de 2 500 tokens dans le prompt)
├─ waly-voice (service unique, même machine que le micro)
│    Silero VAD + fin de tour sémantique · STT streaming (Parakeet v3 int8 / Kroko FR)
│    TTS streaming CPU (Pocket TTS FR ou Piper fr_FR) · barge-in (lecture interruptible)
├─ waly-core (le cerveau, DRASTIQUEMENT réduit)
│    routes chat/contexte/outils · SQLite ou PGlite locale · prompt < 1k tokens stable
│    ~15-20 outils réellement utilisés (pas 60) · horloge = timezone OS, minute exacte
│    via outil get_current_time (préserve le cache KV SANS mentir sur l'heure)
└─ waly-desktop (Tauri 2, inchangé dans l'esprit)
     sidecar = waly-core natif Windows · un seul contrat client (port + token)
```

**Langage du backend reconstruit** : le Node.js n'est PAS le goulot mesuré (2 s de
SQL+orchestration vs 5-80 s de LLM) — le réécrire en Rust ne rachète pas la latence.
La reconstruction change le **périmètre** (≈ 3-4k lignes au lieu de 17,6k) plutôt que
le langage ; TypeScript strict recommandé pour partager les types avec le front.
Le seul composant où le langage compte : la boucle voix → Python assumé (écosystème
ONNX/audio) ou Rust si intégrée au shell Tauri plus tard.

**Ordre de reconstruction proposé** (chaque phase livre une amélioration mesurable seule) :
1. **P0 — substrat (1 jour)** : qwen3:4b + FastFlowLM (NPU) benchmarké contre
   llama.cpp Vulkan natif et l'existant. Attendu (mesures publiées sur ce silicium) :
   décodage 9,9 → 15-20 tok/s, préfill 39 → 400-600 tok/s, −2,6 Go de RAM résidente.
   Le prompt de 3,6k tokens passe de 90 s à ~6 s d'ingestion à froid, ~0 à chaud.
2. **P1 — voix reconstruite (2-4 jours)** : nouveau `waly-voice` avec STT streaming,
   fin de tour sémantique, TTS streaming FR, barge-in ; cible mesurée ≤ 1 s.
3. **P2 — cœur réduit (1 semaine)** : nouveau `waly-core` minimal (chat, mémoire,
   15-20 outils natifs, horloge juste) ; portage des migrations utiles.
4. **P3 — desktop rebranché (2-3 jours)** : sidecar natif Windows, un seul contrat,
   correctifs Tauri (capabilities, opener, Réglages).
5. **P4 — option NPU** : Lemonade en second moteur si le gain mesuré le justifie.

---

### Annexe A — preuves code (chemins exacts)
- `waly-backend/src/llm/index.js` (1 796 l.) ; `tools.js` (959 l., ~60 outils) ;
  `prompt-builder.js:174` (TZ UTC), `:187-190` (grain horloge) ;
  `runtimes/ollama.runtime.js:8` (127.0.0.1:11434 par défaut), `:64` (keep_alive -1),
  `:81-83` (num_ctx) ; `local-config.js:8` (cloud OFF par défaut → self-hosted).
- `waly-desktop/src-tauri/src/sidecar.rs:53-67` (spawn node sans env LLM) ;
  `capabilities/default.json` (fenêtre « panel » inexistante) ; `lib.rs:55-56`
  (opener absent) ; `Shell.tsx:22-32` (Reglages non routé).
- `waly-backend/voice/voice_loop.py:28` (port 3000 en dur), `:31-33` (chemins
  `C:\temp-waly` en dur), `:119` (lecture bloquante), `:88` (normalisation à 0,95
  qui amplifie le bruit).

### Annexe B — mesures et sources état de l'art
- Mesures locales 2026-07-03 : chat 14,6 s (froid) / 9,2 s / 4,5 s (chaud) ;
  `ollama ps` = 100 % CPU, ctx 6144, keep_alive Forever ; prefill 3 223 tok = 83,1 s
  (38,8 tok/s) ; décodage 35 tok = 3,5 s (9,9 tok/s) ; RAM libre 1,6/15 Go ;
  WSL NAT → 11434 code 000.
- Latence cascade locale : RealtimeVoiceChat ~500 ms (4090) ; Kyutai Unmute ~750 ms
  (L40S) ; Pipecat local Mac < 800 ms ; barre « fluide » 600-800 ms (convergence
  multi-sources) ; > 1,5 s = « cassé ».
- WSL2 : RAM 50 % par défaut (Microsoft learn, wsl-config) ; 9P ~11× plus lent
  (microsoft/WSL#4197) ; pas de passthrough NPU (WSL#5492, amd/xdna-driver) ;
  Vulkan WSL = Dozen incomplet vs natif OK.
- STT FR : Parakeet-TDT 0.6B v3 (WER FR 5,15 % Fleurs, CC-BY, ~36× RT CPU 9800X3D) ;
  Kroko ASR streaming FR ; distil-large-v3-fr.
- TTS FR CPU : Pocket TTS (100M, FR, ~200 ms premier son, 2 cœurs) ; Piper fr_FR
  (RTF 0,03-0,1) ; Supertonic 2/3 FR ; Kokoro FR = 1 voix, grade B- (VOICES.md officiel).
- Mémoire : OLLAMA_KV_CACHE_TYPE q8_0 = KV ÷2 quasi sans perte (ollama#6279) ;
  qwen3:4b Q4_K_M = 2,6 Go ; stack voix 4B complet ≈ 6 Go résident.
- LLM sur Ryzen AI 300 : bande passante = plafond décodage (~12 tok/s 8B / 15-20 4B
  tous moteurs) ; FastFlowLM NPU qwen3:4b = 19,6 tok/s décodage + 615 tok/s préfill,
  qwen3:8b = 11,9 + 457 (mesuré Ryzen AI 7 350, même NPU/bande passante) ; ancre CPU
  du 340 exact : Llama 3.2 3B = 23,8 tok/s (geerlingguy/ai-benchmarks) ; Lemonade
  v10.9 (hybride NPU+iGPU, Windows) ; Ollama ≥0.12.6 a Vulkan (expérimental→défaut) ;
  840M : aucun benchmark publié (extrapolation 4 CU = décodage ≈ CPU, préfill ×2-3) ;
  NPU dans WSL : impossible aujourd'hui (WSL3 preview seulement, support AMD incertain).
