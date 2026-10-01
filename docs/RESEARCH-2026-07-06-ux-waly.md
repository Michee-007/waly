# Recherche UX — conclusions pour l'app Waly (2026-07-06)

> Recherche approfondie (harness multi-agents : 6 angles, 25 sources fetchées,
> 108 claims, 25 vérifiés en vote adversarial → 20 confirmés / 5 réfutés).
> Déclenchée parce que la disposition A avait été dessinée depuis les premiers
> principes **sans** étudier les apps citées (screenpipe, Rewind, Tavus, GPT
> Store, Operator/UI-TARS, Obsidian graph…). But : valider/amender A sur preuves.

## Verdict sur la disposition A : AMENDER, pas remplacer

**Confirmé (garder) :**
- **Présence abstraite (éclipse) > avatar** : les avatars vidéo tombent dans la
  vallée dérangeante (lip-sync, tour-de-parole rigide qui casse à la pause).
  Une présence non-humaine est le bon choix. *(newatlas, intellyx — 3-0)*
- **Les 3 portes parler/écrire/appeler** : le voix-first doit basculer vers le
  visuel quand la tâche se complique. *(arXiv 2306.09992 — 3-0)*

**À amender (fautes/risques identifiés) :**
1. **Dock affichant des navigateurs "à venir" = vaporware** (Rabbit R1, Humane :
   livrer sur une roadmap détruit la confiance). → montrer **uniquement les
   navigateurs livrés** ; masquer/grisé+non-cliquable pour le reste. *(3-0)*
2. **Dock d'icônes = « GPT Store UX disaster »** : icônes seules n'expliquent
   rien, découverte échoue sans recherche sémantique. → **labels explicites**,
   petit nombre, **accès par intention vocale** (« ouvre ma mémoire »). *(3-0)*
3. **Éviter « dashboard + chat + rail » générique** : la réf. ambient (Rewind,
   empreinte = icône barre d'état, rappel à la demande) plaide pour un **défaut
   minimal/ambiant** — éclipse + portes ; navigateurs déployés **à la demande**.
4. **Porte « appeler » = PERCEPTION, pas avatar parlant** : Waly te *voit*
   (caméra, réagit à ton état), pas un visage en visio. *(3-0)*
5. **Souveraineté visible** : cadrage explicite « 100% local · rien ne sort »
   est un différenciateur crédible ; l'afficher, et signaler quand le cloud
   BYOK est engagé. Caveat : « zéro egress » = cadrage marketing dès qu'un
   cloud opt-in existe → signalisation, pas garantie absolue. *(2-1/3-0)*

## Patterns validés pour les navigateurs futurs

- **Ambient (R7)** : rappel par **timeline DVR + recherche OCR** (pas juste un
  chat) ; empreinte UI minimale (icône barre d'état). *(3-0)*
- **Computer-use / Écran (R5-R6)** : boucle **screenshot→action** (VLM lit
  l'écran, pilote souris/clavier) ; **humain-dans-la-boucle pour le fort
  impact** + **supervision par observation** (feedback temps réel) plutôt
  qu'approbation pas-à-pas. Aligne avec le HITL déjà porté dans waly-core.
  *(OpenAI computer-use, UI-TARS — 3-0)*
- **Graphe de connaissances / Liens** : ⚠️ **piège** — la vue graphe (Obsidian)
  est belle mais **inutile pour naviguer**, « hairball » > 200 notes, surtout
  décorative/diagnostique. → NE PAS faire de Liens une visualisation-graphe
  comme navigation principale. *(3-0)*
- **États vides** : enseigner **au point de besoin** (amorces actionnables),
  pas de tutoriel forcé au démarrage. *(NNGroup — 3-0)*
- **Voix-first** : ne pas *lire* du texte brut à voix haute (noie l'utilisateur)
  → **résumer / jalonner / découper**, basculer au visuel si complexe/comparatif.

## Réfutés (à ne pas colporter)
- « La voix seule échoue pour la majorité des cas » → **réfuté (0-3)**. Ce qui
  échoue, c'est *lire du texte non restructuré*, pas la voix.
- « Le rappel ambient est surtout du langage naturel, pas un scrub de
  timeline » → **réfuté (0-3)** : c'est bien timeline + recherche.

## Caveats
- Critique du dock : sources blog (GPT Store) — vote unanime mais qualité
  secondaire. A n'a pas été testée en usage → **amender par itération live**
  (méthode Michée), pas trancher à froid.
- Rewind.ai décommissionné/rebrandé Limitless (capture off déc. 2025) — vaut
  comme **pattern historique**, pas produit vivant.

## Décisions actées pour l'app (dérivées)
1. **A amendée** : défaut minimal = éclipse + les 3 portes + signal souveraineté.
2. **Navigateurs = livrés seulement** (mémoire, tâches, rappels), labels
   explicites, ouvrables **à la voix** ; futurs masqués ou clairement « à venir ».
3. **« Appeler » = Waly te voit** (perception R4), jamais un avatar qui parle.
4. **Liens ≠ graphe** comme nav principale.
5. Persistance des sources : voir `tasks/` (rapport complet) ; angles & votes
   dans le journal du workflow.
