# ADR 2026-09-11 — L'écran se lit en TEXTE (arbre d'accessibilité) ; les mains d'écran agissent par patterns, chaque geste approuvé

- **Statut** : accepté (gates 1-2 vertes au banc, `lab/ecran-banc/README.md`)
- **Contexte** : phase B de la ROADMAP (agent d'écran), plan
  `docs/PLAN-2026-09-11-B-agent-ecran.md`. Hermes pilote déjà le bureau
  (UIAutomation + SendInput) ; voir l'écran en direct pour aider et apprendre
  d'une démonstration n'existent pas chez lui (vérifié à la source le 11/09).

## Constat

Le mode ▣ Écran de R5 répondait d'après une **image** fraîche à chaque tour.
Sur la machine de référence, NPU/FastFlowLM bloqué par Smart App Control, le
cerveau de secours (qwen3 instruct sur Ollama) ne voit pas : la vision est
déléguée à gemma3:4b, rechargé à chaque tour visuel → **86 s par tour**. Un
tour texte coûte ~3,5 s. Et même avec un VLM rapide, une image ne donne ni le
nom exact d'un bouton, ni une poignée pour cliquer dessus.

## Décision

1. **L'écran se lit d'abord par UI Automation** (`waly-sight::uia`) : un seul
   aller-retour cross-process (`CacheRequest` Subtree, filtre ControlView),
   élagage (interactifs nommés, textes, conteneurs nommés), rendu texte
   indenté avec des ids `[n]`, sous budget de 5 000 caractères (compaction des
   frères répétés, puis troncature honnête). Mesuré : 25-169 ms pour les
   fenêtres usuelles, 384-445 ms pour Zoom (3 570 éléments), ≤ 1 308 tokens
   partout. L'image + OCR (R5) ne servent plus que si la demande est
   **visuelle** (`intention_visuelle`) ou si l'arbre est **pauvre** (canvas,
   PDF image, jeu : `est_riche`).
2. **Une seule lecture vivante dans le contexte** (balises `⟦écran⟧`) : les
   anciennes sont remplacées par « [lecture d'écran plus ancienne retirée] »
   avant chaque nouvelle — même principe que `degrader_images`. Mutation en
   QUEUE d'historique (re-prefill court), assumée face à la discipline
   append-only.
3. **Agir = outil sensible `agir_ecran {id, action, texte}`** : avant la mise
   en attente, `Tool::preparer` (nouveau point d'extension du dispatch)
   traduit l'id éphémère en **identité stable** (fenêtre, rôle, nom, rang)
   + **libellé humain** — c'est ce libellé que l'utilisateur approuve (carte
   HITL). À l'exécution, l'élément est **re-cherché** (jamais de poignée COM
   gardée) : introuvable ⟹ aucune action. Actions par patterns UIA (Invoke,
   Value, Toggle, SelectionItem, ExpandCollapse) : **sans bouger la souris ni
   voler le premier plan** (mesuré sur une fenêtre réduite qui l'est restée).
   Seul repli clavier (`SendInput` unicode, pas de ValuePattern) : il prend
   le focus, et le constat le dit.
4. **Murs** : mur financier sur le LIBELLÉ de l'élément (liste dédiée aux
   interfaces — la liste des noms d'outils contient « save » et bloquerait
   « Save ») et sur le texte (numéro de carte) ; un champ mot de passe n'est
   jamais lu (`••••`) ni écrit ; hors partage d'écran, les deux outils sont
   absents du catalogue (`Tool::disponible`, zéro token payé).
5. **Apprendre en regardant (B3)** = session « Regarde-moi » ouverte et
   fermée par l'utilisateur (bouton ◉ du pop-up, indicateur rouge), bornée à
   10 min : hooks bas niveau souris/clavier, clic → élément sous le curseur
   (texte), clavier → raccourcis et Entrée/Tab/Échap SEULEMENT (un caractère
   ne quitte jamais le callback), saisies relues à la sortie du champ (sauf
   mot de passe). Les étapes deviennent une **compétence** (module
   `competences`) rejouable par `agir_ecran` sous approbation.

## Conséquences

- Le mode Écran devient utilisable sur une machine sans vision rapide, et
  Waly peut **faire** (parité Hermes) avec une preuve de sûreté plus forte :
  libellé humain approuvé, identité re-vérifiée, murs sur les libellés.
- Limites assumées : applications sans accessibilité (jeux, canvas,
  certaines apps Electron) → lecture par OCR/image, pas d'action ; UWP
  réduite (suspendue) illisible ; Explorateur réduit ne publie pas son
  contenu. Linux (AT-SPI) et macOS (AX) après R-L.
- Rien de neuf n'est persisté : ni pixel, ni texte d'écran, ni frappe. Les
  étapes d'une démonstration ne vivent qu'en mémoire jusqu'à la distillation
  (seule la recette générale, sans données personnelles, est gardée).
- La voix du mode Écran lit aussi en texte (`GET /lecture-ecran` du desktop)
  mais n'a pas de mains en v1 (les actions passent par le chat et ses cartes
  d'approbation).
