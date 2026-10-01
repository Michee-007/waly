# Plan R4 — Mode appel : Waly te voit (2026-07-07)

> Phase R4 du RFC : « **Mode appel** : perception caméra continue
> (présence/regard/gestes/émotion) + VLM ponctuel + avatar réactif, fusionné
> avec la voix ». **Critère de sortie mesuré : session « visio » fluide —
> réaction perçue < 500 ms, moment VLM < 8 s.**
> C'est la phase qui définit le produit (« le compagnon qui te voit ») :
> le moment « wow » démontrable, que personne n'offre en 100 % local.
>
> Lancée le 2026-07-07 sur instruction de Michée. ⚠ La sortie de phase R3
> (verdict sur l'app installée) reste formellement ouverte — elle se joue en
> parallèle, R4 n'y touche pas.

## L'architecture de la phase : deux étages de perception

Le critère « < 500 ms » et le critère « < 8 s » ne peuvent PAS être servis
par la même brique — c'est le cœur du design, calqué sur la perception
humaine (réflexe rapide + attention lente) :

1. **Boucle rapide (continue, < 500 ms, CPU, jamais le VLM)** : capture
   caméra → petits modèles ONNX (ort load-dynamic, le chemin validé
   Silero/Parakeet) → **événements sémantiques** : présence/absence,
   attention (visage orienté écran ou pas), expression grossière
   (sourire/neutre/froncé). Ces événements nourrissent (a) l'éclipse
   réactive de l'UI, (b) le contexte du prompt (comme les souvenirs R2).
   Budget : < 100 Mo résidents, quelques % CPU à ~5-10 Hz.
2. **Moment VLM (ponctuel, < 8 s, NPU)** : sur intention (« regarde »,
   « tu vois ça ? ») ou déclencheur explicite → une frame → `qwen3vl-it:4b`
   via FLM → la description entre dans le tour de conversation normal
   (mémoire + outils compris). Jamais en continu : le NPU décode à
   16-19 tok/s, une frame VLM coûte des secondes — c'est un *moment*, pas
   un flux.

## La question structurante : le VLM peut-il devenir LE cerveau unique ?

Règle non négociable : **un seul modèle LLM résident** (budget 6,5 Go).
Trois options, à départager AU BANC, pas au feeling :

| Option | Principe | Risque |
|---|---|---|
| **A. Cerveau unique** | `qwen3vl-it:4b` REMPLACE `qwen3-it:4b` partout (texte+vision, mêmes 13 outils) | tool-calling/qualité FR/TTFT du VLM inconnus |
| B. Swap ponctuel | LLM résident, on charge le VLM au moment vision puis on revient | coût de (dé)chargement NPU ×2 dans les 8 s — probablement mort |
| C. Cohabitation | 2 processus flm (comme LLM+ASR en R1) | 4B + 4B ≈ 8-9 Go > budget — mort sur 15 Go sauf surprise |

L'option A est la seule qui tient le budget ET la latence *si* le VLM est un
bon cerveau texte. C'est LE verrou de la phase (GATE B).

> **✅ TRANCHÉ AU BANC le 2026-07-07 (jour du lancement) : option A —
> cerveau unique.** Mesures complètes dans `engines/README.md` (section
> VLM R4). En bref : TTFT texte 1,07-1,12 s et décodage 17,1-17,8 tok/s
> **identiques à qwen3-it:4b** ; tool-calling natif OK et calibré ; outils +
> image dans la même requête OK ; moment VLM 6,8-7,6 s (640×480) et **4,5-
> 4,7 s en forme de prod** (historique+outils+image, 1 083 tok). L'option B
> est morte de sa belle mort : le hot-swap FLM a tué le serveur (OOM,
> zombie NPU) — un serveur = un modèle épinglé. Réserves à lever au
> chantier 4-5 : catalogue 13 outils complet, qualité FR à l'oreille de
> Michée, budget RAM réel (VLM = 5,96 Go privés, ~1,7 Go de plus que le
> LLM texte — stack complète à re-mesurer).

## GATES au banc avant d'écrire (méthode R2/R3)

1. **[GATE A] Moment VLM < 8 s.** `qwen3vl-it:4b` téléchargé (3,9 Go, dont
   `vision_weight.q4nx` 792 Mo séparé), servi par FLM : vérifier la forme
   d'API vision (OpenAI-compat `image_url` base64 ? `flm run` seulement ?),
   mesurer image réelle → réponse complète : encode vision + préfill +
   décodage, effet de la taille d'image, RAM au chargement (`--ctx-len`
   obligatoire, piège 1). **Mesure de sortie : secondes de bout en bout.**
2. **[GATE B] VLM = cerveau unique ?** Rejouer sur `qwen3vl-it:4b` les
   vérifications R2 : tool-calling natif (catalogue 13 outils), TTFT texte
   court, qualité FR à l'oreille des réponses. Comparer à `qwen3-it:4b`.
   **Décision gravée : A, B ou C.**
3. **[GATE C] Capture caméra native sous SAC.** Crate candidat : `nokhwa`
   (backend Media Foundation) ou `windows-rs` MF direct. Cross-compile
   `x86_64-pc-windows-gnu` depuis WSL, exe côté Windows : énumération des
   caméras + une frame RGB écrite en PNG. (Tauri a déjà fait passer
   `windows-rs` — a priori OK, à prouver.) Matériel de référence recensé le
   2026-07-07 : **HP True Vision FHD Camera** (USB, `VID_04F2&PID_B7F3`,
   exposée Media Foundation). ⚠ Le test de capture réelle se fait AVEC
   Michée présent (règle vie privée : pas de frame capturée hors session
   d'appel consentie — la règle vaut aussi pour le dev).
4. **[GATE D] Perception légère temps réel.** Sourcer les petits ONNX :
   détection visage (YuNet ~100 Ko ou équivalent), orientation/attention,
   expression (FER). Mesurer latence/frame et % CPU sur flux caméra réel.
   **Mesure de sortie : ms/frame, % CPU, fiabilité subjective.**

> **✅ GATES C+D LEVÉS le 2026-07-07 (soir même du lancement).** Mesures dans
> `engines/README.md` § Perception R4. GATE C : nokhwa/Media Foundation
> cross-compile windows-gnu et **passe SAC** ; frame 1080p capturée, décodée,
> vérifiée visuellement (Michée présent). GATE D : YuNet 2023mar (232 Ko,
> ⚠ entrée fixe 640×640 → letterbox) détecte à **~10 ms/frame CPU**, boucle
> live 640×480 à 29,7 fps avec visage sur 297/297 frames → le < 500 ms est
> tenu avec ~10× de marge. Reste du chantier 2 devenu chantier 3 : sourcer
> l'expression (FER) et dériver l'attention des 5 points YuNet.

## Chantiers

| # | Contenu | Sortie mesurée |
|---|---|---|
| 1 | **[GATES A+B] Banc VLM NPU** : moment vision + cerveau unique | mesures dans `engines/README.md`, décision A/B/C gravée |
| 2 | **[GATES C+D] Banc caméra + perception** : capture native, petits ONNX | frame PNG sous SAC ; ms/frame et % CPU mesurés |
| 3 | **`waly-sight` caméra** : capture MSMF + boucle perception → événements typés (présence, attention, expression) consommables par le desktop et le prompt | événement émis < 500 ms après le fait, CPU tenu, tests — **✅ 2026-07-07 : `arrivee` à 0,30 s, 7,9 Hz, ~41 % d'un cœur (debug), 8 tests ; reste la validation active des transitions par Michée (seuil attention à régler à l'œil)** |
| 4 | **Moment VLM branché waly-core** : intent « regarde » / outil vision → frame → FLM → réponse dans le fil (desktop texte d'abord) | **image → réponse < 8 s de bout en bout** — **✅ 2026-07-07 : outil `regarder` + injection d'image + cerveau unique basculé ; tour vision complet 9,8 s (2 tours de modèle), moment VLM seul ~5-6 s ✓ ; raccourci d'intention (1 seul tour ≈ 5 s) = chantier 5** |
| 5 | **Mode appel desktop** : écran appel (éclipse réactive aux événements < 500 ms), voix compagnon in-app (`waly-voice` processus, backlog R3 honoré ici), fusion voix+vision+mémoire | **session visio fluide — verdict de Michée** — **✅ 2026-07-08 : perception in-app + raccourci d'intention (tour vision 5,1 s) + VOIX COMPAGNON (waly-voice spawn/kill avec l'appel, fil parlé suivi à l'écran, busy_timeout 2 écrivains) ; app réinstallée (release NSIS, cerveau VLM) ; RAM 188,5/255,6 Mo < 300 ✓. Limite v1 : « regarde » à la voix ne voit pas (caméra au desktop) — fusion backlog. RESTE : verdict de Michée** |
| 6 | Terrain + docs : pièges gravés, JOURNAL, mesures | phase documentée |

**Sortie de phase R4** : une session d'appel réelle — Michée active le mode
appel, Waly le voit arriver (réaction éclipse < 500 ms), il lui parle, lui
montre un objet (« tu vois ça ? » → réponse < 8 s), tout en local.

> **✅ R4 BOUCLÉE le 2026-07-08 (verdict de Michée).** Critères mesurés
> tenus : réaction < 500 ms ✓ (arrivée 0,30 s, événements suivis en live),
> moment VLM 5,1 s < 8 s ✓, session voix+vision+écran d'appel+conscience
> fonctionnelle de bout en bout, app installée. **Verdict qualitatif gravé
> comme ouverture de la suite** : l'intelligence visuelle est
> SUPERFICIELLE (constater ≠ comprendre) et l'éclipse n'est pas assez
> vivante — une session dédiée « intelligence visuelle & fusion réelle »
> est demandée AVANT R5 (voir REPRISE du JOURNAL pour les pistes à
> instruire).

## Décisions actées (à compléter au fil des gates)

- **La caméra vit dans `waly-sight`** (coquille déjà au workspace) : c'est
  l'œil de Waly — caméra en R4, écran en R5, même crate, même client VLM.
  Pas de crate `waly-call` : l'appel est une *expérience* (desktop + voice +
  sight), pas une brique.
- **La perception continue ne passe JAMAIS par le VLM** — petits ONNX CPU
  seulement (latence + budget NPU).
- **Vie privée gravée dans le design** : caméra = opt-in par session d'appel,
  indicateur visible en permanence, **aucune frame persistée par défaut**
  (les événements sémantiques, oui ; les pixels, non). C'est l'identité
  produit (« rien ne quitte ta machine » inclut « rien ne s'écrit sans
  raison »).
- **Le VLM remplace, ne s'ajoute jamais** au modèle NPU résident (règle du
  modèle unique) — la forme exacte (cerveau unique vs swap) attend GATE B.
- **Voix in-app = processus compagnon** `waly-voice.exe` piloté par le
  desktop (spawn/stop, même session partagée qu'en R3) — le mode appel est
  sa raison d'être.
- Cible de build inchangée : boucle WSL → `x86_64-pc-windows-gnu`, pièges
  3/6/9 en vigueur.
