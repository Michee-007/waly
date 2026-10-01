# RESEARCH — Hermes agent (Nous Research) vs Waly : face à face honnête

> **Révisé le 2026-09-11 à la demande de Michée (« pas de bullshit, avec
> honnêteté »).** La version du 10/09 était **fausse sur plusieurs points**,
> écrite de mémoire au lieu d'être vérifiée à la source. Tout ce qui suit a
> été relu le 11/09 sur le dépôt, les notes de version et la documentation
> officielles d'Hermes. « Non trouvé » veut dire non trouvé dans leur
> documentation — pas « n'existe pas ».

## Ce que j'avais faux le 10/09

| J'avais écrit | La réalité (sources du 11/09) |
|---|---|
| Hermes : « voix = transcription de mémos seulement, pas de temps réel, pas de barge-in, pas de mot d'éveil » | **Faux.** v0.20 (3 août 2026) : voix conversationnelle temps réel, synthèse en streaming clause par clause, interruption en parlant, mot d'éveil **sur l'appareil**, à phrase libre, multi-profils. |
| Hermes : « cerveau cloud par défaut » | **Trompeur.** Les modèles locaux sont de premier rang : Hermes Desktop installe un modèle local **en un clic** (5 sept.) — lit le matériel, télécharge un llama.cpp adapté (CUDA/Metal/Vulkan/HIP/CPU), verdict vert/orange/rouge par modèle, télécharge les poids, sans compte. En revanche plusieurs outils sont en ligne par défaut (synthèse vocale « edge » = service Microsoft, navigateur Browser Use, images FAL). |
| Hermes : « pas de vision » | **Faux.** Il analyse des images. Caméra en direct ou écran partagé : **non trouvé**. |
| Hermes : « pile Python+Node, pas d'app installable » | **Incomplet.** Hermes Desktop existe (Windows 10/11, macOS 12+, Linux) et Hermes tourne en **Windows natif**. |
| Waly « devant » en voix | **Surévalué** : fonctionnellement c'est désormais une égalité (voir plus bas). |

## Échelle — sans commune mesure

- **Hermes** : 244 k étoiles, 50 k forks, 33 500 commits, 5 000+ PR ouvertes,
  une version toutes les ~4 semaines (v0.20 le 3 août, v0.21 le 31 août,
  v0.21.1 le 7 sept.). Une équipe (Nous Research) et une communauté.
- **Waly** : un développeur accompagné d'IA, **aucun utilisateur externe**,
  dépôt **non publié**, **pas de CI**, mesuré sur **une seule machine**.

## Axe par axe

| Axe | Hermes | Waly | Verdict honnête |
|---|---|---|---|
| Communauté, écosystème | énorme | inexistante | **Hermes, écrasant** |
| Installer un modèle local | un clic, télécharge et configure | recommande, l'utilisateur tape `ollama pull` (Waly est scellé) | **Hermes** — notre choix de sécurité coûte de l'ergonomie |
| Plateformes | Windows, macOS, Linux | Windows ; Linux = socle de chemins, **rien d'exécuté sur un vrai Linux** | **Hermes** |
| Voix temps réel | streaming, barge-in, mot d'éveil libre ; fin de tour = **3,0 s** de silence par défaut (réglable) ; STT local faster-whisper ; TTS par défaut en ligne (edge), Piper/NeuTTS/Kittentts en local | streaming, barge-in, fin de tour **0,28-0,8 s** mesurée ; premier son **1,69 s** mesuré (secours Ollama, 10/09) ; voix française Pocket choisie à l'oreille ; tout local | **Égalité fonctionnelle.** Waly mieux sur la fin de tour et le français local ; Hermes mieux sur le mot d'éveil et la maturité. Aucune latence publiée par Hermes → pas de comparaison chiffrée possible. |
| Mot d'éveil | phrase libre, sur l'appareil, multi-profils | « Waly » seul, affiné sur **une** voix (celle de Michée), modèles de features **non commerciaux** | **Hermes** |
| Vision | analyse d'images ; caméra/écran en direct : non trouvé | mode Appel (caméra) + mode Écran avec OCR, journal visuel — **mais aujourd'hui 86 s par tour visuel** sur la machine de référence (NPU bloqué, modèle délégué) | **Waly a la fonction, pas la performance**, tant que FastFlowLM n'est pas signé |
| Vie privée **prouvable** | aucune garantie technique trouvée ; outils par défaut en ligne | scellé réseau au niveau du noyau (WFP), journal des sorties bloquées, test en un clic, fail-closed prouvé — **limites** : WebView et serveurs MCP hors scellé, Windows seulement | **Waly** — c'est notre seul fossé réellement unique |
| Sûreté des actions | 8 couches, approbation des commandes, cron fail-closed, **isolation par conteneurs** (7 backends) | murs financiers, gate de risque, approbations persistées, anti-boucle ; **pas d'isolation** | **Comparable**, avantage Hermes pour l'isolation |
| Plateforme d'agent (compétences, mémoire, MCP, multi-agents, cron, messageries, navigateur, sous-agents) | mature sur tout | compétences **v1 d'hier jamais vues en usage réel**, mémoire SQLite, MCP stdio (prouvé en CLI), le reste absent | **Hermes, largement** |
| Documents Office natifs | non trouvé | .docx/.xlsx/.pptx/.pdf prouvés dans Office | **Waly** |
| Poids | Python (+ Node pour certaines surfaces) | app native Rust, 120-250 Mo de RAM mesurés | **Waly**, sans être décisif |
| Performance sur la machine de référence | non mesurée (même matériel : llama.cpp sur Vulkan, probablement comparable) | 3,5 s par tour à chaud ; 23-30 s à froid en CLI ; vision 86 s | **Personne ne brille ici** — c'est la machine, pas le logiciel |

## L'écran — l'axe que j'avais oublié (ajout du 11/09, retour de Michée)

Trois capacités distinctes, vérifiées le 11/09 sur la doc « Computer Use »
d'Hermes :

| Capacité | Hermes | Waly | Verdict |
|---|---|---|---|
| **Voir ton écran en temps réel et t'aider en direct** (co-présence : il regarde pendant que tu travailles, commente, conseille à la voix) | non trouvé — son contrôle du bureau part d'actions qu'IL lance (captures + arbre d'accessibilité) | mode ▣ Écran (R5) : capture fraîche à chaque question, OCR local, réponse vocale ; 5,8 s mesurés sur NPU. **Les remarques spontanées sur l'écran sont désactivées depuis le 10/07** (elles polluaient) | **Waly a la base, pas encore le « temps réel » vivant** |
| **Le faire sous tes yeux** (cliquer, taper, modifier dans tes applications) | **oui, prêt pour la production** : arrière-plan sans voler le curseur, UIAutomation + SendInput sous Windows, AX (macOS), AT-SPI (Linux), modèles locaux acceptés, approbation des actions destructrices | **non** : Mains v1 = fichiers seulement ; mains d'interface au ROADMAP (R7) | **Hermes** — retard à combler |
| **Enregistrer ton travail pour l'apprendre** (tu montres une fois, il retient la méthode) | non trouvé — ses compétences viennent de SES tâches, pas de tes démonstrations | non ; prévu au mandat du 20/07 (R7 : « workflows enregistrés/rejoués ») — la mémoire ambiante CONTINUE, elle, est supprimée (anti-Recall) | **Personne** — terrain libre, et compatible avec l'identité si c'est une session que TU déclenches, gardée en étapes texte, jamais en pixels |

Conséquence : l'écran est l'axe où Waly peut être **premier** — voir en
direct + apprendre en te regardant, avec la preuve que rien ne sort — à
condition de combler le « faire sous tes yeux » (parité avec Hermes).

## Verdict sans complaisance

1. **Hermes est en avance sur presque tout ce qui fait une plateforme
   d'agent**, et il a **rattrapé nos fonctions phares de juillet** (voix
   temps réel, mot d'éveil), avec des moyens sans commune mesure.
2. **Ce que Waly a et qu'Hermes n'a pas (vérifié)** : le scellé réseau
   prouvable ; la caméra et l'écran en direct intégrés à la conversation ;
   le français d'abord avec une fin de tour rapide ; une app native légère ;
   les documents Office natifs.
3. **Ce qui est fragile chez Waly** : la performance dépend d'un moteur NPU
   que Windows bloque ; une seule machine ; aucun utilisateur externe ;
   plusieurs fonctions livrées les 10-11/09 jamais utilisées en vrai
   (compétences apprises, MCP dans l'app, panneaux) ; mot d'éveil lié à une
   voix et non commercial.
4. **« Au même niveau si ce n'est plus »** : **pas atteignable** comme
   plateforme d'agent généraliste face à cette équipe. **Atteignable, et
   dépassable, sur un créneau** : l'assistant local, prouvablement privé,
   francophone, qui voit et entend — les professions à secret.

## Ce que je recommande

1. **Assumer le créneau** : la preuve de confidentialité est le produit ;
   pour le reste, viser une parité suffisante, pas une course.
2. **Option stratégique à étudier** : le « Huis clos » comme brique
   indépendante, capable de sceller n'importe quel agent local — Hermes
   compris. C'est le seul endroit où nous sommes uniques.
3. **Ingénierie** : la performance d'abord (FastFlowLM signé ou un moteur
   qui passe Smart App Control) ; valider en usage réel ce qui a été livré
   cette semaine ; publier (dépôt neuf) pour avoir une CI, Linux et des
   utilisateurs.

## Sources (consultées le 2026-09-11)

- https://github.com/NousResearch/hermes-agent (métriques, fonctions, Windows natif)
- https://github.com/NousResearch/hermes-agent/releases (v0.20, v0.21, v0.21.1)
- https://hermes-agent.nousresearch.com/docs/user-guide/features/voice-mode (fournisseurs STT/TTS, fin de tour 3,0 s, barge-in)
- https://www.marktechpost.com/2026/09/05/nous-research-hermes-desktop-one-click-local-model-setup/ (modèle local en un clic)
- https://www.mayhemcode.com/2026/09/hermes-agent-review-2026-features.html (sécurité : 8 couches, limites)
- État Waly : docs/JOURNAL.md (mesures des 10-11/09)
