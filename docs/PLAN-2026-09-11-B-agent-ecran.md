# PLAN B — L'agent d'écran : voir, faire sous tes yeux, apprendre en regardant (2026-09-11)

> **État (2026-09-11 soir)** : gates 1-2 ✅ au banc, gate 3 🟠 (code prêt,
> terrain = Michée) ; chantiers 0-4 LIVRÉS en code ; E2E LLM ✅ (tâche en 3
> gestes approuvés sur fenêtre réduite). Reste : terrain de Michée sur l'app
> réinstallée. Détail : `lab/ecran-banc/README.md`, ADR
> `docs/ADR-2026-09-11-ecran-lu-en-texte.md`, JOURNAL « 🖐 B ».

> Brief : ROADMAP « Now » B, séquence gravée par Michée le 11/09 (B → C →
> démo → publier → LinkedIn). Comparaison à la source :
> `docs/RESEARCH-2026-09-10-hermes-agent-vs-waly.md` § « L'écran ».
> Hermes PILOTE déjà le bureau (UIAutomation + SendInput) → **B2 = parité** ;
> voir en direct pour aider (B1) et apprendre d'une démonstration (B3) : non
> trouvés chez Hermes → **notre avance possible**.

## Le constat qui décide de l'architecture

Sur la machine de référence, **un tour visuel coûte 86 s** (NPU/FLM bloqué par
SAC, cerveau de secours qwen3 instruct qui ne voit pas, gemma3:4b délégué et
rechargé à chaque tour). Un tour TEXTE coûte ~3,5 s à chaud. Un agent d'écran
qui passe par l'image est donc mort-né ici — et il le serait aussi chez tout
utilisateur sans NPU.

**Décision : l'écran se lit d'abord en TEXTE, par l'arbre d'accessibilité
Windows (UI Automation).** UIA donne ce que les pixels ne donnent qu'à grand
prix : le nom exact de chaque bouton, champ, onglet, élément de liste, sa
valeur, son état (activé, coché, mot de passe) — et surtout des **poignées
pour agir** (Invoke, Value, Toggle, SelectionItem, ExpandCollapse) **sans
bouger la souris ni voler le premier plan**. C'est la même brique pour les
trois capacités :

| Capacité | Ce que fait UIA | Pixels ? |
|---|---|---|
| **B1 voir et aider** | instantané texte de la fenêtre active (rôles, noms, valeurs) au prompt, rafraîchi à chaque tour | non (OCR R5 en complément si l'arbre est pauvre ; VLM seulement sur demande visuelle explicite) |
| **B2 faire sous tes yeux** | l'agent désigne un élément par son `[id]` de l'instantané ; l'action passe par un pattern UIA | non |
| **B3 apprendre en regardant** | session « regarde-moi » explicite : clic → `ElementFromPoint` → étape texte (« clic bouton « Enregistrer » dans « Bloc-notes » ») | **jamais** |

Cohérence avec l'identité : *« Il voit tout. Rien ne sort. »* — l'arbre UIA
est lu localement, le modèle est local, le sceau WFP tient ; aucune capture
n'est persistée ; B3 est une session déclenchée par l'utilisateur, jamais un
enregistrement continu (mandat anti-Recall du 20/07).

## Règles de sûreté (non négociables)

1. **Lecture** (instantané) : libre pendant une session écran ouverte par
   l'utilisateur (le mode ▣ Écran existant = opt-in explicite). Hors session :
   aucun outil d'écran au catalogue (règle d'honnêteté R4 : pas d'outil
   fantôme).
2. **Champs mot de passe** (`IsPassword`) : valeur JAMAIS lue, jamais écrite
   au prompt, jamais enregistrée par B3 (remplacée par `••••`).
3. **Agir** = outil SENSIBLE (`irreversible`) → approbation humaine par action
   (gate de risque existant, `tools.rs`), avec un libellé HUMAIN de l'action
   (« cliquer « Envoyer » dans « Outlook » ») — pas un id. Le mur financier
   s'applique aux intentions ET aux libellés d'éléments (« Payer », « Acheter »,
   « Virement » → refus, sans exception).
4. **Pas d'aveugle** : chaque action re-vérifie que l'élément existe toujours et
   porte le même nom/rôle (sinon : re-lire l'écran, pas de clic dans le vide).
5. **Le geste de l'utilisateur prime** : si la souris/le clavier de
   l'utilisateur bougent pendant une séquence, la séquence s'arrête (B2).
6. **B3** : hooks bas niveau clavier/souris SEULEMENT pendant la session
   explicite ; le hook clavier ne retient QUE les raccourcis (Ctrl/Alt+touche)
   et Entrée/Tab/Échap — **jamais les caractères tapés** (le texte saisi est
   relu dans le champ à la sortie de focus, sauf mot de passe).

## Gates (au banc, AVANT le code produit — `lab/ecran-banc/`)

- **GATE 1 — Lire (B1)** : exe Rust windows-gnu (UIA via `uiautomationcore`,
  COM, DLL système signée → SAC-safe) qui produit l'instantané texte d'une
  fenêtre. Critères : passe SAC ; **< 300 ms** pour une fenêtre usuelle
  (Explorateur, Bloc-notes, Paramètres) avec un seul aller-retour
  cross-process (CacheRequest) ; texte élagué **< ~1 500 tokens** ; mesure
  aussi le pire cas (navigateur, VS Code) et la parade s'il déborde.
- **GATE 2 — Agir (B2)** : sur un Bloc-notes lancé pour le banc : saisir par
  ValuePattern (repli SetFocus + SendInput unicode), déplier/replier un menu
  par ExpandCollapse, relire pour prouver. Critères : action < 100 ms,
  **curseur souris inchangé**, premier plan inchangé pour les patterns
  (le repli clavier, lui, exige le focus — mesuré et nommé).
- **GATE 3 — Regarder (B3)** : session « regarde-moi » 30 s : clics →
  étapes texte justes (rôle + nom + fenêtre), saisies relues au départ du
  focus, raccourcis nommés, mots de passe masqués. Critère : un scénario
  Bloc-notes réel (ouvrir menu, taper, Ctrl+S, nommer, Enregistrer) restitué
  en étapes lisibles par un humain ET rejouables par B2.

## Chantiers (après gates vertes)

0. `waly-sight::uia` (module Windows, stub ailleurs) : instantané élagué
   (`Instantane { texte, elements: Vec<Element> }`, ids stables par
   instantané), actions par pattern, garde « même élément ».
1. **B1 — voir et aider** : en mode ▣ Écran, chaque tour reçoit l'instantané
   UIA de la cible (fenêtre choisie / active) dans le contexte frais du tour —
   remplace l'image par défaut (tour texte ~3,5 s au lieu de 86 s) ; OCR si
   l'arbre est pauvre (jeux, canvas, PDF image) ; image VLM seulement si
   l'intention est visuelle. Outil `lire_ecran` (lecture).
2. **B2 — faire sous tes yeux** : outil `agir_ecran {id, action, texte}`
   (sensible, libellé humain à l'approbation, murs, garde même-élément,
   arrêt au geste utilisateur) ; cadre de présence qui montre l'élément visé.
3. **B3 — apprendre en regardant** : bouton « Regarde-moi » (session bornée,
   indicateur visible) → étapes texte → distillées en **compétence** (module
   `competences.rs` existant) rejouable par B2 sous approbation.
4. Docs : ADR « l'écran se lit en texte », README/ROADMAP, JOURNAL, AGENTS.md.

## Hors périmètre (assumé)

- Contrôle d'applications sans accessibilité (jeux, canvas, apps Electron
  sans a11y activée) : repli OCR en lecture, pas d'action à l'aveugle par
  coordonnées en v1.
- Linux (AT-SPI) et macOS (AX) : après R-L.
- Rejouer sans approbation : jamais en v1 (l'auto-approbation apprise reste
  au backlog R2).
