# Plan R5 — `waly-sight` écran : « regarde mon écran » (2026-07-09)

> Phase lancée le 2026-07-09 sur « lance R5 » de Michée. R5 est la dernière
> marche de la vision : après la caméra (R4) et l'intelligence visuelle
> (R4.5), Waly voit l'ÉCRAN. Critère de sortie RFC : **réponse écran < 8 s**.
> R5 RÉUTILISE tout R4.5 : cerveau unique `qwen3vl-it:4b`, journal visuel
> (`visual_memory`), delta « depuis ton dernier tour », raccourci d'intention,
> politique d'éviction du cache FLM, éclipse vivante « Marée ». Le SEUL
> capteur neuf est l'écran ; le reste est du câblage.
>
> Fondations : RFC (§ Vision P2, « Écran : capture → OCR léger d'abord →
> VLM seulement si la question exige la compréhension visuelle ») +
> `PLAN-2026-07-08-R4.5-intelligence-visuelle.md` (pyramide de perception,
> discipline append-only, gates FLM déjà mesurés).

## Dette d'ouverture (héritée de R4.5)

R4.5 a été clôturée avec une **dette de validation** : la session des 5
critères annoncés (souvenir, delta, inférence, « elle vit », budget) n'a
jamais été jouée en appel réel. Comme le verdict R3 fondu dans R4, elle se
joue **en ouverture de R5, au fil du premier usage caméra** — verdict de
Michée. Non bloquante pour le capteur écran (chantiers indépendants), mais
à ne pas oublier : c'est la vraie preuve que R4.5 tient sur le terrain.

## Décisions de cadrage (tranchées avec Michée au lancement)

- **Mode « Écran » dédié** (PAS un simple coup par coup, PAS fondu dans
  l'appel) : un bouton `▣ Écran` frère de `◐ Appel` ouvre une **session de
  partage d'écran** explicite. Tant qu'elle est active : Waly regarde à la
  demande (« regarde ») ET fait des **moments proactifs** au repos
  (changement de scène → une ligne au journal), avec la même pyramide que
  l'appel caméra. Opt-in par session = la garantie vie privée n°1.
- **Cadrage adaptatif** : par défaut la capture vise la **fenêtre active**
  (plus privé, plus petit = VLM moins cher, OCR plus net) ; « regarde
  **tout** mon écran » élargit à l'**écran principal entier**. Deux chemins
  de capture, arbitrés par le raccourci d'intention (mots-clés).
- **Vie privée durcie (l'écran > la caméra)** : (1) rien ne se capture hors
  session écran active ; (2) **aucun pixel persisté** (règle R4 inchangée) ;
  (3) **le texte OCR brut ne se persiste JAMAIS** non plus (un écran peut
  contenir mots de passe, e-mails, secrets) — seule la **description VLM
  d'une ligne** entre au journal visuel, comme pour la caméra ; (4)
  indicateur visible de partage (le bandeau de session + si l'API le fournit
  gratuitement, la bordure de capture système).

## Le principe : pyramide à coût étagé (RFC, confirmée R4.5)

L'écran est texte à 90 % : on ne réveille pas le VLM pour lire une erreur de
compilation. La pyramide :

| Étage | Déclencheur | Brique | Coût | Produit |
|---|---|---|---|---|
| 1. Capture | à la demande / scène change | capture native fenêtre|écran (SAC-safe, DLL/WinRT) + downscale ≤ 720p | ~qq ms | un cliché en mémoire, jamais sur disque |
| 2. OCR | chaque capture (quasi gratuit) | OCR local (Windows.Media.Ocr ou ONNX) | ~50-200 ms CPU | texte exact + boîtes, injecté au prompt |
| 3. VLM | seulement si la question exige le VISUEL (layout, image, UI, OCR vide/douteux) | `qwen3vl-it:4b` NPU, image ≤ 720p | ~3-6 s | compréhension de la scène écran |
| 4. Journal/delta | sur moment proactif ou tour | `visual_memory` (kind `vu`/`moment`) + delta R4.5 | ~0 (réutilisé) | Waly se souvient de l'écran, remarque les changements |

Routage OCR-first : le texte OCR va TOUJOURS au prompt (exact, pas cher) ;
l'image n'est jointe (tour zéro outil, discipline R4.5) que si l'intention
est visuelle ou l'OCR insuffisant. C'est ce qui tient le budget < 8 s sur
le cas dominant (« lis-moi ça », « c'est quoi cette erreur »).

## GATES au banc AVANT d'écrire (méthode R2/R3/R4/R4.5)

Chaque gate produit des chiffres gravés au `engines/README.md`. FLM
qwen3vl-it:4b tourne déjà sur 52626 (vérifié au lancement).

1. **[GATE 1] Capture d'écran native sous SAC.** L'inconnue n°1. Candidats,
   tous en DLL/WinRT via `windows-rs` (jamais d'exe tiers — piège 3 ; les
   DLL se chargent sous SAC, prouvé R4 avec nokhwa/ort) :
   - **Windows.Graphics.Capture** (WinRT, Win10 1803+) — couvre fenêtre
     (`GraphicsCaptureItem` depuis un `HWND`) ET moniteur (depuis un
     `HMONITOR`), API MS-bénie, **bordure de capture système** = signal vie
     privée offert. Async/WinRT, un poil lourde à câbler.
   - **DXGI Desktop Duplication** (`IDXGIOutputDuplication`) — moniteur
     entier seulement (pas de fenêtre), très rapide, GPU.
   - GDI `BitBlt` — simple, lent, fenêtre OU écran, aucune bordure.
   Cible : couvrir fenêtre active + écran entier → **Windows.Graphics.Capture
   pressenti** (seul à faire les deux + bordure). **Sortie : API choisie,
   latence de capture, RAM, VERDICT SAC (exe de banc passe/bloque), verdict
   fenêtre vs moniteur.** Banc : `waly-sight.exe snap-screen`.

   > **✅ TRANCHÉ AU BANC le 2026-07-09 (jour du lancement) — mesures :
   > `engines/README.md` § Écran R5.** GDI `BitBlt` suffit et est le
   > PLANCHER SAC-safe : exe debug windows-gnu **passe SAC du premier coup**,
   > écran entier 1920×1080 **75 ms**, fenêtre active 1920×1032 **78 ms**,
   > couleurs/texte justes, Chrome GPU capturé sans noir. `screen.rs` livré
   > (`capture_screen`/`capture_active_window`/`rendre_conscient_dpi`).
   > `PrintWindow` absent de windows 0.61 → BitBlt de région. **API retenue :
   > GDI** ; Windows.Graphics.Capture/DXGI = upgrade chantier 0 SI une fenêtre
   > masquée doit être rendue (pas le cas courant). Latence négligeable vs
   > OCR/VLM.

2. **[GATE 2] OCR local sous SAC — quasi gratuit ?** ⚡ **Déjà amorcé au
   lancement (mesure PowerShell)** : `Windows.Media.Ocr` fonctionne (WinRT,
   projection sans exe → **SAC-safe par nature**), `MaxImageDimension 10000`,
   MAIS packs installés = **en-US + ar-SA seulement, PAS fr-FR**. Décision à
   trancher, mesures à faire :
   - **Option A — Windows.Media.Ocr + pack fr-FR** : `Add-WindowsCapability
     -Online Language.OCR~~~fr-FR~...` (capability Microsoft, admin +
     téléchargement une fois, 100 % local ensuite, zéro RAM résidente, zéro
     modèle à shipper). Le plus léger si Michée accepte l'install système.
   - **Option B — OCR ONNX via l'`ort` déjà en place** (RapidOCR / PaddleOCR
     mobile int8 : détecteur + reconnaisseur ~10-15 Mo, multilingue) —
     aucune install système, même stratégie que YuNet/FER+ (DLL signée), mais
     ~10-15 Mo au budget mémoire quand la session écran est active.
   Note : le texte d'écran est souvent code/anglais/UI → en-US couvre
   beaucoup, mais un doc FR accentué sera abîmé. **Sortie : moteur OCR
   choisi, latence sur un cliché 720p (fenêtre vs plein écran), qualité sur
   code + UI + texte FR, coût RAM.** Banc : `waly-sight.exe ocr <png>`.

   > **✅ TRANCHÉ le 2026-07-09/10 — VERT (option B choisie par Michée ;
   > mesures : `engines/README.md` § Écran R5).** Aucun modèle rec latin/FR
   > ONNX propre off-the-shelf → combo **PP-OCRv4 det + PP-OCRv3 en rec +
   > `en_dict.txt`** (~14 Mo, `engines/models/ocr/`). `ocr.rs` livré (DBNet
   > CCL + CRNN CTC). Banc écran IDE DENSE 1920×1080 : **lecture 2,54 s,
   > 116 zones, conf 0,78** ; anglais/code/UI solide, **FR accentué faible
   > (assumé, couvert par le VLM)**. < 3 s tenu. → Chantier 1 cœur LIVRÉ.

3. **[GATE 3] Coût VLM d'une capture écran.** RFC estime 1080p ≈ 1-2k tok →
   préfill NPU 2-4 s ; cible réponse complète < 8 s. Mesurer sur qwen3vl
   NPU : cliché **fenêtre active** vs **écran entier**, downscalés à 720p /
   540p, avec la forme de prod (historique court + image jointe, tour zéro
   outil comme R4.5). **Sortie : chiffres gravés, confirmation < 8 s,
   résolution de downscale retenue (compromis lisibilité OCR/VLM ↔ latence).**
   Banc : `bench-vlm-screen-*.json` + curl.exe.

   > **✅ TRANCHÉ AU BANC le 2026-07-09 — VERT AVEC CONTRAINTE (mesures :
   > `engines/README.md` § Écran R5).** Cliché 1080p → 720p JPEG q80 (96 Ko) =
   > **949-957 tok image** ; compréhension excellente (appli, URL, nav, texte
   > lus juste). La LONGUEUR de réponse décide tout : 220 tok = 17,58 s ✗ ;
   > 33 tok (prompt bref + `max_tokens` 80) = **5,81 s ✓ < 8 s**. Le décodage
   > (~16-19 tok/s) domine, pas le préfill. **Contrainte gravée : réponses
   > écran BRÈVES** (prompt court + cap ~80 tok) ; l'OCR-first évite tout
   > décodage pour « lis-moi ce texte ». 720p retenu.

Ordre : GATE 1 d'abord (make-or-break capture), puis 2 et 3 en parallèle.
Si GATE 1 bloque sous SAC malgré la nature DLL/WinRT (imprévu vs R4), repli
documenté (autre API / bordure désactivée) avant tout code de chantier.

## Chantiers

| # | Contenu | Sortie mesurée |
|---|---|---|
| 0 | **Capteur écran dans `waly-sight`** (`screen.rs`, `cfg(windows)`, stub Linux comme `camera.rs`) : capture fenêtre active + écran principal via l'API du GATE 1, downscale ≤ 720p, `jpeg(max_w)` (même surface que le `Percepteur` caméra) ; bin de banc `snap-screen`. Aucun pixel écrit sur disque. | cliché fenêtre + écran capturés et vérifiés, latence gravée, exe passe SAC |
| 1 | **OCR (`ocr.rs`)** selon GATE 2 : extraction texte (+ boîtes si utile) d'un cliché ; routage OCR-first exposé à waly-core (texte → prompt ; image jointe seulement si intention visuelle / OCR faible). | « lis-moi l'erreur à l'écran » répondu par OCR SEUL (pas de VLM), < 3 s |
| 2 | **Outil + raccourci d'intention écran (waly-core, mutualisé desktop/voix)** : `regarder_ecran` (fournisseur de cliché branché par l'hôte, comme `regarder` R4) + `intention_visuelle` étendue aux mots-clés écran (« mon écran », « à l'écran », « ce code », « regarde tout mon écran » → plein écran) ; image jointe = tour zéro outil (discipline R4.5), texte OCR préfixé. | à la voix ET au clavier : « regarde mon écran » → capture+OCR(+VLM) fusionnés, < 8 s — **✅ 2026-07-10 (waly-core + banc)** : `native_tools` = `CadrageEcran`/`CaptureEcran`/`FournisseurEcran` + outil `regarder_ecran` (injecte l'image, REND le texte OCR dans le résultat) ; `chat` = `intention_ecran` (fenêtre/plein, accents+sans-accent), `mode_ecran` (routage lecture vs compréhension), `fusion_ecran` (raccourci : lecture = OCR seul sans image ; compréhension = image jointe + OCR en aide ; indice de brièveté GATE 3). Banc `waly.exe ecran` (fournisseur factice PNG+OCR, registre vide = tour ZÉRO outil) VÉRIFIÉ end-to-end contre FLM : **lecture** (OCR seul, pas d'image) → réponse juste sur l'erreur ; **compréhension** (image jointe) → description fidèle de VS Code. 56 tests core (4 neufs). Latence À FROID (processus neuf, persona re-préfillé) 5,6 s / 9,3 s ; en session CHAUDE (cache append-only R4.5, régime réel du mode Écran) → vers GATE 3 (~5,8 s) — à confirmer au ch. 3. Hôtes desktop/voix branchés au ch. 3 (ils ont la source d'écran) |
| 3 | **Mode « Écran » dédié (desktop UI + service)** : bouton `▣ Écran` (bandeau de session, indicateur permanent), service loopback étendu (`/cliche-ecran`) ; perception écran in-process (SceneDetector dHash R4.5 réutilisé sur la vignette écran → moments proactifs au repos, politique de préemption GATE C R4.5) ; **journal visuel écran** (kind `vu`/`moment`, description VLM UNIQUEMENT — jamais l'OCR brut) ; voix compagnon capable de regarder l'écran (via loopback, parité R4) ; éclipse : état « regard écran ». | session écran : regard à la demande + un moment proactif observé + souvenir écran restitué ; RAM < 300 Mo ; zéro pixel/texte OCR persisté — **✅ 3a LIVRÉ + E2E VERT 2026-07-10** : bouton `▣ Écran` (toggle, bandeau `mstate`), `Cmd::EcranStart/EcranStop` (charge/libère l'`Ocr`), `regarder_ecran` enregistré au worker (fournisseur = capture+OCR via `capture_ecran_ocr`), `intention_ecran` branché au handler Send AVANT la caméra, `fusion_ecran` → tour (image comprehension / OCR-seul lecture), `llm_court` (`max_tokens` 120 = plafond GATE 3), **`contexte_ecran` injecté au tour mais JAMAIS persisté** (nouveau param de `run_streamed_turn` ; vérifié en base : seul le message BRUT est stocké, zéro OCR). E2E `e2e-ecran.js` VERT sur l'app : OCR **chargé sous SAC in-app**, « lis-moi l'écran » → capture+OCR+réponse (11,1 s à froid debug, cible atteinte en session chaude+release). `Shot::jpeg(max_w)` ≤720p ajouté au capteur. **✅ 3b (moments proactifs) LIVRÉ + VÉRIFIÉ 2026-07-10** : `boucle_scene_ecran` (thread dédié, capture toutes les 4 s pendant la session → `dhash` → `SceneDetector` R4.5 réutilisé → drapeau `scene_ecran` ; aucun pixel gardé, seul le hash survit) ; `proactif_au_repos` étendu (branche ÉCRAN au repos, GATE C : cooldown `WALY_ECRAN_MOMENT_S` défaut 90 s → capture 720p + VLM une phrase → journal kind `moment`, **non parlé** — mode écran tapé ; le delta du tour suivant le raconte). Test comportemental (cooldown 5 s, « afficher le bureau ») : moment journalisé « L'application Waly est active… » ✓. `ecran_actif` (AtomicBool cross-thread) levé/baissé avec l'`Ocr`. Réflexion neutralisée (caméra ET/OU écran). **DÉCISION MICHÉE : `▣ Écran` = mode VOIX + chat** (comparé à Copilot Vision ; voix « moche et lente » gravée en mémoire, chantier parallèle). **✅ MODE ÉCRAN-VOIX LIVRÉ + VÉRIFIÉ 2026-07-10** : `/cliche-ecran` loopback (écran 720p, gardé par `ecran_actif`, avant `/cliche` — testé JPEG 66 Ko) ; **`▣ Écran` spawn waly-voice** en mode écran (`WALY_ECRAN_MODE=1`, slot `voix_ecran`, tuée à l'arrêt — cycle de vie testé : spawn=1 / stop=0) ; **waly-voice** : `mode_ecran()`, `regarder_ecran` sur `/cliche-ecran`, `intention_ecran`/`fusion_ecran` (image jointe, tour zéro outil, bref), conscience « tu regardes l'écran », sémantique système écran (caméra gatée `!mode_ecran`) ; **moments proactifs écran DITS** au repos via la machinerie moments existante (kind `moment` partagé). Test `waly-voice text "regarde mon écran"` (desktop en mode écran) : « Je vois une page web avec un article sur les nouvelles technologies… » ✓. **RESTE : éclipse/page d'appel réutilisées pour la session écran (état « regard écran », polish visuel), release + réinstall NSIS de sortie, verdict 5 critères. PUIS chantier qualité VOIX (Michée : A d'abord)** |
| 4 | **Terrain + docs** : pièges gravés (SAC capture, OCR, VLM écran), JOURNAL, mesures au `engines/README.md`, CLAUDE.md § État, RFC (R5 ✅). | phase documentée |

Ordre : 0 → 1 → 2 → 3 (capteur → OCR → fusion → mode). Le chantier 3 réutilise
tel quel le SceneDetector, le journal, le delta et l'éclipse de R4.5.

## Critères de sortie — ANNONCÉS avant le test (règle Michée)

Testable en une session « partage d'écran » réelle :

1. **Lecture (OCR-first)** : afficher une erreur / un bout de code, « lis-moi
   ça » → texte exact restitué, **sans réveiller le VLM**, en < 3 s.
2. **Compréhension (VLM)** : une UI / un graphe / une capture sans texte,
   « qu'est-ce que je regarde ? » → Waly décrit le VISUEL (layout, sens),
   réponse complète **< 8 s** (critère RFC).
3. **Cadrage** : « regarde ça » vise la fenêtre active ; « regarde tout mon
   écran » élargit — le bon périmètre est capturé.
4. **Mémoire + delta écran** : montrer un écran, faire autre chose, « c'était
   quoi tout à l'heure ? » → juste (journal) ; changer d'écran → Waly le
   REMARQUE (delta/moment proactif).
5. **Vie privée + budget** : partage seulement pendant la session `▣ Écran`
   (indicateur visible) ; **zéro pixel ET zéro texte OCR persisté** (vérifié
   base + disque) ; RAM desktop < 300 Mo en session ; TTFT tour parlé non
   dégradé de plus de +0,5 s vs R4.5.

**Sortie de phase = verdict de Michée sur ces 5 points, annoncés tels quels
avant la session de test** (+ la dette R4.5 jouée au passage).

## Décisions actées

- **Aucun nouveau gros modèle** : le cerveau reste `qwen3vl-it:4b` unique.
  L'OCR est un petit moteur local (WinRT intégré ou ONNX ≤ 15 Mo), pas un LLM.
- **OCR-first** : le texte exact et pas cher passe toujours ; le VLM ne se
  réveille que pour le VISUEL. C'est ce qui tient le < 8 s sur le cas dominant.
- **Écran = capteur le plus sensible** : opt-in par session explicite, aucun
  pixel persisté, **aucun texte OCR brut persisté** (seule la description VLM
  d'une ligne entre au journal), indicateur de partage visible.
- **Réutilisation maximale de R4.5** : `visual_memory`, delta, raccourci
  d'intention, SceneDetector, éclipse, discipline append-only, service
  loopback — R5 ajoute UN capteur, pas une architecture.
- **DLL/WinRT jamais exe** (piège 3) ; cible de build inchangée : boucle WSL
  → `x86_64-pc-windows-gnu`, pièges 3/6/7/9 en vigueur ; `SAC_REROLL` prêt.
