# Waly — Knowledge Navigator 100 % local

Agent personnel(a personal intelligence system) local : voix temps réel FR & EN, vision (caméra « mode appel » + écran),
mémoire, agents cowork supervisés, computer use. Cloud strictement opt-in (BYOK).
Réponds en français. Documente les décisions en ADR dans `docs/` (culture existante).

**Ce dépôt vit à `C:\waly` (Windows), PAS dans WSL** — code, moteurs et modèles au même
endroit, conformément à la règle « exécution 100 % Windows natif ». Depuis WSL :
`/mnt/c/waly` (I/O 9P plus lentes — préférer un terminal/Claude Code côté Windows pour
les gros travaux ; `core.autocrlf false` déjà posé sur le dépôt).

## Lis d'abord
1. `docs/RFC-2026-07-03-knowledge-navigator-reconstruction.md` — architecture cible + feuille de route R0-R7
2. `docs/AUDIT-2026-07-03-diagnostic-etat-de-lart.md` — pourquoi l'ancien système échouait
3. `ROADMAP.md` — priorités actuelles (le journal de construction détaillé n'est pas publié)
4. `engines/README.md` — moteurs d'inférence, mesures, pièges

## La machine (référence, celle de Michée)
AMD Ryzen AI 5 340 (3× Zen 5 + 3× Zen 5c) · iGPU Radeon 840M (4 CU, RDNA 3.5) ·
NPU XDNA 2 50 TOPS (driver 32.0.203.329, OK) · **15 Go RAM utiles** · Windows 11 + WSL2 (NAT).
Loi physique : décodage LLM borné par la bande passante (~15-20 tok/s max pour un 4B,
tous moteurs). Les accélérateurs gagnent sur le PRÉFILL (NPU : ×16 mesuré).

## Règles non négociables (issues de l'audit)
- **Exécution 100 % Windows natif.** WSL = édition de code/git seulement. Jamais
  d'inférence, d'audio ou de serveur dans WSL (NAT bloque, NPU inaccessible, RAM ÷2).
- **Rust pour tout ce qui shippe** ; TypeScript uniquement dans la webview Tauri.
  Pas de backend Node. SQLite (rusqlite + sqlite-vec), pas de PGlite.
- **Un seul modèle LLM résident à la fois** (budget ~6,5 Go pour tout le stack IA).
  qwen3:4b est le point optimal ; le 8B ne sera JAMAIS fluide ici — ne pas y revenir.
- **Budget mémoire signé** : toute nouvelle brique résidente se justifie dans les 6,5 Go.
- Critères de sortie mesurés par phase (voir RFC) — pas de passage à la phase suivante
  sans mesure qui valide.

## Moteurs d'inférence (état : INSTALLÉS et MESURÉS le 2026-07-03)
- **Moteur A (principal) : FastFlowLM sur NPU** — `C:\waly\engines\flm\`, serveur
  OpenAI-compat `http://127.0.0.1:52625/v1`. Lancer : `engines/start-flm.ps1`.
  qwen3:4b mesuré : préfill 617,9 tok/s, décodage 16-19 tok/s, tour court ~2 s.
  ⚠ Mesuré 2026-07-04 : **aucun cache de préfixe inter-requêtes** (requête
  identique = même TTFB) → chaque token d'historique se re-paie à chaque tour ;
  TTFB plancher ~1,2 s (4B, prompt court), `--pmode turbo` -0,07 s,
  **`--prefill-chunk-len` = CRASH sur prompt long, à proscrire** (v0.9.43).
  ⚠ AFFINÉ 2026-07-08 (R4.5 GATE A, mesures au README engines) : v0.9.43
  A un **cache de conversation append-only, une seule case** — conversation
  étendue VERBATIM = TTFB plancher 1,25 s (historique gratuit) ; toute
  mutation n'importe où, requête identique, `system` en milieu de
  conversation, ou requête intercalée (éviction) = plein tarif. Le TTFT
  2,2-2,4 s en app venait de NOTRE prompt (conscience sur `messages[0]`,
  souvenirs re-injectés, fenêtre glissante) → discipline append-only =
  chantier 0 de R4.5.
  Catalogue notable : `qwen3vl-it:4b` (vision R4), `qwen2.5vl-it:3b`, Whisper-V3-Turbo
  (STT, flag `-a`), `qwen3.5:2b/4b`. Un modèle NPU à la fois.
- **Moteur B (fallback) : Ollama Vulkan iGPU** — port 11434. Env User déjà posées :
  `OLLAMA_VULKAN=1`, `OLLAMA_IGPU_ENABLE=1` (sans elle l'iGPU est écarté !),
  `OLLAMA_KV_CACHE_TYPE=q8_0`, `OLLAMA_FLASH_ATTENTION=1`. Mesuré : préfill 119,6,
  décodage 12-15 tok/s. ROCm impossible (gfx1152).
  **Fenêtre de contexte (2026-09-30, décision Michée)** : Ollama charge 4 096
  par défaut alors que système + outils font déjà ~3 000 tokens/tour → alias
  `qwen3vl-it:4b` recréé avec `PARAMETER num_ctx 8192` (Modelfile
  `FROM qwen3vl-it:4b`, les autres réglages sont conservés) : 3,22 Go chargés
  au lieu de 2,90. À refaire si l'alias est recréé par `ollama cp`.
  Diagnostic : `WALY_DEBUG_PROMPT=1` → `%TEMP%\waly-systeme.txt` et
  `%TEMP%\waly-requete.json` (ce que le modèle reçoit vraiment).
- Vérifier l'environnement : `powershell.exe -File engines/check-setup.ps1` (depuis WSL).

## Pièges connus (chèrement appris — ne pas re-payer)
1. **FLM `--ctx-len` obligatoire** : défaut 32k → KV ~4,5 Go → erreur `0xc01e0200`
   (échec de pagination mémoire vidéo). Utiliser 8192. Si l'erreur survient quand même :
   RAM insuffisante au moment du chargement (viser ≥ 7 Go libres), tuer/relancer flm.
   **Vécu 2026-07-08 : le worker peut aussi mourir EN COURS DE ROUTE sous
   pression RAM** (app + appel + builds WSL simultanés → 2,5 Go libres) —
   signature : le parent garde le port (Get-NetTCPConnection OK) mais
   WorkingSet ~1 Mo et toute requête = 10054. Remède : tuer le PID parent,
   `wsl --shutdown` (libère plusieurs Go de vmmem après des builds),
   relancer start-flm.ps1.
2. **PowerShell 5.1 et l'UTF-8** : `Invoke-RestMethod` casse les corps JSON accentués
   → toujours `curl.exe -d @fichier.json` (fichier écrit depuis WSL). Et les scripts
   `.ps1` écrits en UTF-8 sans BOM sont lus en Windows-1252 → **scripts .ps1 en ASCII
   pur** (pas d'accents, pas de tirets longs).
3. **Smart App Control est ACTIVÉ** (irréversible) : tout .exe non signé téléchargé est
   bloqué (llama-server.exe l'est). Ne pas proposer de le désactiver. Produit : signer
   nos binaires avant distribution.
   **Aggravation mesurée le 2026-07-03** : SAC bloque AUSSI des binaires compilés
   localement (erreur 4551) — les build-scripts cargo (serde, proc-macro2…) sont
   bloqués, un hello-world sans dépendance passe : verdicts ISG imprévisibles
   → `cargo build` natif Windows **impraticable** en l'état (rustup + VS BuildTools 18
   sont pourtant posés et fonctionnels).
   **Boucle de dev VALIDÉE (2026-07-03, mingw-w64 posé dans WSL)** : compiler DEPUIS
   WSL vers Windows (`--target x86_64-pc-windows-gnu`, build-scripts en ELF hors de
   portée de SAC), exécuter l'exe côté Windows. Le binaire de tests waly-voice
   (serde inclus) passe SAC : 20/20 natifs. ⚠ Verdicts ISG PAR binaire, imprévisibles :
   un exe-test serde équivalent a été bloqué, le nôtre passe. Si un build est bloqué
   (erreur 4551) : modifier/rebuilder change le hash → nouveau verdict ; ne pas
   conclure trop vite à un blocage systémique. Le produit final sera signé de toute
   façon (cf. plus haut). **Constat affiné (3 jours de données)** : les builds *debug*
   passent (3/3), les *release* non signés se font bloquer (2/3) → exécuter en local
   les builds dev, réserver release aux binaires signés. **SAC ne filtre que les
   lancements d'EXE : les DLL se chargent, signées (onnxruntime Microsoft) COMME non
   signées (sherpa-onnx-c-api.dll, testé LoadLibrary + FFI complet le 2026-07-03)**
   → stratégie : tout ce qui est natif tiers passe par des DLL chargées dynamiquement
   (ort load-dynamic, libloading), jamais par des exe tiers ni du link statique C++.
   **Vécu 2026-07-05 (R2)** : les verdicts flippent AUSSI en debug — plusieurs
   blocages consécutifs le même après-midi (exe de tests 84 Mo, binaires
   fraîchement rebuildés), à chaque fois la boucle « toucher la source →
   rebuilder → relancer » a suffi. Un exe contenant `std::process::Command`
   (spawn de curl.exe) a été bloqué 2× et est passé après retrait du spawn
   (corrélation non prouvée — préférer notre client HTTP interne de toute
   façon). Filet : les tests de logique tournent aussi sur l'hôte WSL
   (`cargo test`, ELF, hors SAC) — seule la validation du binaire produit
   exige la boucle Windows. **Vécu 2026-07-08 : le verdict peut flipper
   SANS rebuild** — l'app installée (release R4, inchangée, lancée des
   dizaines de fois) s'est fait bloquer au double-clic (events
   CodeIntegrity 3033/3077, log `Microsoft-Windows-CodeIntegrity/
   Operational` = le bon endroit pour identifier QUEL exe est bloqué).
   Remède : incrémenter `SAC_REROLL`, rebuild release, régénérer
   `Waly-Setup.exe` (`apps/desktop/installer/build-installer.sh release`,
   target `~/waly-target-wsl-tauri`), réinstaller `/S`.
4. Chemins Windows depuis WSL : `/mnt/c/waly/...` ; profil utilisateur (chemin pouvant contenir des espaces)
   (avec espace — toujours quoter).
5. **FLM v0.9.43 : ASR en standalone uniquement.** `/v1/audio/transcriptions` ne
   fonctionne que via `flm serve --asr 1` SANS modèle LLM (sinon `null` HTTP 200
   silencieux en 3 ms). LLM et ASR = deux processus flm, deux ports (52626 / 52625) —
   cohabitation NPU mesurée OK. Script : `engines/start-flm-asr.ps1`.
   ⚠ Vécu 2026-07-07 : un envoi malformé (WAV non-16 kHz / sans `;type=audio/wav`)
   laisse l'endpoint en `null` PERMANENT → tuer/relancer flm (parent ET worker,
   deux PID). Et Whisper-NPU n'aide PAS sur les énoncés < 1 s (mesures :
   `engines/README.md` section STT).
6. **Ne jamais partager `target/` entre WSL et Windows** : artefacts ELF et PE se
   mélangent → échecs incompréhensibles. Si les deux mondes buildent, poser un
   `CARGO_TARGET_DIR` distinct côté WSL (ex. `target-wsl/`).
7. **Sockets Windows : jamais `set_read_timeout` (SO_RCVTIMEO) sur un flux long.**
   Après expiration, Winsock laisse le socket dans un état indéterminé → RST
   envoyés au serveur en plein stream ; FLM en est MORT (vécu 2026-07-04, le
   processus zombie gardait le port 52626 et résistait au restart — tuer
   l'ancien PID). Pour sonder pendant une attente : `set_nonblocking(true)` +
   boucle WouldBlock + sleep. Et ne jamais chauffer FLM en avortant un stream :
   warmup non-streaming `max_tokens:1` lu jusqu'au bout.
8. L'anciennne stack (waly-backend Node + waly-desktop Tauri) reste dans les dépôts
   frères `~/waly-backend`, `~/waly-desktop` : source de vérité pour porter la logique
   validée (mur financier, validation dispatch, prompts skills) — ne pas y développer.
9. **Tauri en `windows-gnu` : la lib desktop DOIT être `crate-type = ["rlib"]`,
   PAS `cdylib`** (vécu 2026-07-05, R3 gate). Le cdylib du template Tauri
   (`*_lib.dll`, utile au mobile seulement) génère une table d'exports que le
   linker BFD de mingw refuse (`export ordinal too large`) → link échoue. En
   rlib, le bin EXE linke sans table d'exports : **build + lancement OK sous
   SAC** (Tauri 2.11 + WebView2 + wry cross-compilés depuis WSL, fenêtre
   ouverte, ~21 Mo au repos). ⚠ `cargo build --bin` NE contourne PAS : cargo
   compile tous les crate-types de la lib en un passage — changer crate-type
   est obligatoire. Pas besoin de msvc/cargo-xwin.
   ⚠ **Corrigé 2026-07-06 (R3 ch.1)** : `WebView2Loader.dll` est dans la TABLE
   D'IMPORTS de l'exe (pas load-dynamic) → elle DOIT être **à côté de l'exe**,
   sinon lancement mort immédiat `exit 0xC0000135` (STATUS_DLL_NOT_FOUND) et
   « ça ne s'ouvre pas » en double-clic. (Un lancement peut réussir par hasard
   si le CWD/PATH la contient — piège trompeur.) La copier depuis le build :
   `target/.../build/webview2-com-sys-*/out/x64/WebView2Loader.dll` à côté du
   `.exe` ; l'installeur final la bundlera. Les DLL mingw (libgcc/libstdc++/
   winpthread) ne sont PAS requises : Rust windows-gnu les lie en self-contained.
10. **Ports : jamais dans la zone dynamique Windows (49152-65535).** WinNAT/
   Hyper-V y réserve des plages AU HASARD au démarrage de WSL (vécu
   2026-09-10 : 52579-52678 réservé → Ollama `bind ... forbidden by its
   access permissions` sur 52626, 52625 idem). Diagnostic :
   `netsh interface ipv4 show excludedportrange protocol=tcp`. **Port LLM de
   l'app = 42626** (`waly_core::llm::port_par_defaut()`, surcharge
   `WALY_LLM_PORT`, même contrat dans `waly-voice` config) ; `start-flm.ps1`
   et `start-waly-voice.ps1` l'ont par défaut. Les mentions « 52626 » plus
   haut et dans les docs datées sont historiques.
11. **SAC bloque AUSSI des DLL non signées chargées par un exe tiers**
   (vécu 2026-09-03, corrige le « les DLL se chargent » du piège 3) :
   `flm.exe` 0.9.43 bloqué au lancement, puis FastFlowLM **1.0.4** (ROCm,
   toujours NON signé, zip ET msi) : l'exe démarre mais `llama_npu.dll` /
   `gemma_embedding.dll` sont refusées (CodeIntegrity 3077 « process X
   attempted to load DLL ») → mort silencieuse. Pas re-rollable (binaire
   tiers). Nos DLL via `ort`/`libloading` passent encore — surveiller.
   **Secours = moteur B, AUTOMATIQUE** : si 42626 ne répond pas,
   `port_par_defaut()` bascule sur **Ollama standard 11434** (démarré par
   l'app Ollama à chaque session, signé, survit aux redémarrages) + alias
   `ollama cp qwen3:4b-instruct-2507-q4_K_M qwen3vl-it:4b` — **cerveau de
   secours retenu le 09-10** : non-réfléchissant par entraînement, outils
   natifs, bon français ; banc 3-11 s, 13-14 s/tour en CLI avec 17 outils
   (JOURNAL 09-10). Écartés : qwen3:4b qui « pense »
   49 s/tour et sa réflexion est IMPOSSIBLE à couper sur Ollama 0.33.3 :
   5 méthodes mesurées le 09-10, gabarit Jinja du GGUF qui force
   `<think>` ; `qwen3-vl:4b-instruct` y PLANTE à l'init vision, bug
   Vulkan ; llama3.2 = 18-19 s, outils OK, et ses appels écrits en texte
   sont rattrapés par `rattrapage.rs`). Limites : pas de vision,
   lent. ⚠ Vécu 2026-09-10 : un `ollama serve` lancé par WMI
   (`Win32_Process Create`) est MORT en silence après quelques minutes (log
   propre, aucun crash) — **WMI n'est PAS une recette durable** (nuance le
   remède FLM du 21/07) ; `start-ollama-secours.cmd` (42626) = manuel.
   FLM ne reviendra que signé (suivre les releases ROCm/FastFlowLM).
12. **Un document généré se prouve en le faisant OUVRIR par le vrai
   lecteur** (`lab/documents-banc/`) : Word/Excel/PowerPoint par COM
   (lecture seule, invisible, hors récents, ne quitter QUE l'instance
   lancée par le script — ceux de l'utilisateur sont souvent ouverts) ; PDF
   par `Windows.Data.Pdf` (WinRT) + rendu de la page 1 en PNG à regarder.
   ⚠ **Jamais de PDF dans Word par COM** : dialogue de conversion
   invisible → script bloqué indéfiniment (vécu 2026-09-10 ; seule issue :
   tuer l'instance Word lancée par le script, identifiée par son heure de
   démarrage). OOXML : l'ORDRE des éléments du schéma compte, Office
   refuse sinon.

13. **Sorties ouvertes (lot 3, ADR 2026-10-01)** : Waly reste scellé ; tout ce
   qui sort (modèle extérieur, messagerie) passe par `exterieur::requete` /
   `converser` = le `curl` de `System32` lancé à part, secret sur son entrée
   standard seulement. Ne jamais ajouter de client HTTP sortant dans
   `waly.exe` (le sceau le bloquerait, 10013) ni mettre une clé en argument ou
   dans une erreur. Un modèle extérieur ne reçoit que
   `conversation_sortante` : pas de système local, pas d'outils. Toute sortie
   s'inscrit au journal (`sceau::noter`, genre `sortie`). Les téléchargements
   de modèles sont faits par Ollama, à la demande.
   Un fil de fond ouvre sa base par `ouvrir_base_de_fond` (au premier
   lancement le cœur la crée encore : un `store::open` unique échoue et le fil
   mourrait en silence). Partage entre deux Waly : `partage.rs` + relais
   `crates/waly-relais` — ne pas toucher au chiffrement (`crypto_box`) sans
   ADR.

## Commandes
- Itération rapide (WSL) : `cargo check --workspace`, `cargo test` (waly-voice : 20 tests).
- **Binaire Windows (boucle officielle, depuis WSL)** :
  `CARGO_TARGET_DIR=~/waly-target-wsl cargo build --release --target x86_64-pc-windows-gnu`
  puis copier l'exe sous C:\ et l'exécuter côté Windows (piège 3 si erreur 4551).
  `CARGO_TARGET_DIR` hors du dépôt : jamais le `target/` partagé (piège 6) + I/O 9P lentes.
- Toolchain Windows native : rustup (profil minimal, PATH non modifié → préfixer
  `$env:USERPROFILE\.cargo\bin`) + VS BuildTools 18 posés — réservée au jour où SAC
  laissera passer les build-scripts (piège 3).
- Bench moteur : fichiers `bench-*.json` dans `C:\waly\engines\`, envoyer avec curl.exe.


## État
Voir `ROADMAP.md` (priorités) et les ADR / PLAN / RFC de `docs/`.
Le journal de construction détaillé n'est pas publié : les références
« JOURNAL » restantes dans le code et les docs pointent vers lui.
