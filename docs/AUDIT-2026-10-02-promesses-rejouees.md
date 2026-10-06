# Audit — les promesses de Waly, rejouées

> Waly promet une vie privée *vérifiable*. La moindre des choses est de
> vérifier nos propres promesses, et d'écrire ce qu'on trouve. Ce document
> est tenu à jour : tout défaut de la promesse centrale s'ajoute ici, avec
> qui l'a trouvé et comment.
>
> Première passe : 1er et 2 octobre 2026. Dernière mise à jour : 2026-10-06.

## Méthode

1. **Jouer pour de vrai.** Lancer l'app depuis des emplacements inhabituels,
   sur une base neuve, et faire un essai de sortie au lieu de lire l'état
   affiché.
2. **Tester à la marge.** Écrire les tests qui manquaient. Une revue
   extérieure du dépôt avait noté que le scellé n'avait que quatre tests pour
   la promesse centrale ; elle avait raison.
3. **Tester sur le vrai système.** La CI Windows de GitHub, en plus des tests
   locaux qui tournent sous Linux.
4. **Comparer avec ce qui existe.** Voir
   [`RESEARCH-2026-10-06-these-et-etat-de-l-art.md`](RESEARCH-2026-10-06-these-et-etat-de-l-art.md).

Trois des six défauts ci-dessous ont été trouvés grâce à un regard ou un
outil extérieur. C'est la raison d'être de ce dépôt ouvert.

## Les six défauts trouvés

### 1. Le scellé était inopérant quand l'app était lancée par un chemin court

- **Gravité** : haute — promesse centrale.
- **Constat** : lancée par un chemin court Windows (`C:\Users\NOM~1\…`), l'app
  déclarait ce chemin au service. L'identifiant que le noyau en tirait ne
  correspondait pas au processus réel : filtres posés, rien de bloqué.
  L'essai « Vérifier maintenant » a montré une sortie **réussie** alors que le
  panneau affichait le scellé comme tenu.
- **Portée** : l'app installée, lancée par son raccourci, n'était pas touchée.
  Les instances de test lancées depuis un dossier temporaire tournaient non
  scellées.
- **Correctif** : chemin résolu (noms longs, liens suivis) avant toute
  déclaration, côté app et côté service. Dans la première version publique.

### 2. Un programme sans privilège pouvait retirer le scellé par le canal du service

- **Gravité** : haute — promesse centrale.
- **Constat** : la requête « sceller » n'était pas gardée sur l'identifiant
  de session, et sceller une session remplace ses filtres. Un programme
  ordinaire pouvait re-sceller le périmètre de Waly avec un exécutable factice
  au nom admissible, et retirer ainsi les filtres des vrais processus.
- **Trouvé par** : l'écriture des tests, après la revue extérieure. Aucun
  signe d'exploitation.
- **Correctif** : une politique d'autorisation pure, appelée avant le moteur.
  Le périmètre ne se scelle ni ne se lève par ce canal ; un agent tiers exige
  l'élévation. Tests du scellé : 4 → 13. Commit public `b3b9673`.
- **Attention** : le correctif vit dans le service. Une installation
  antérieure doit **réinstaller le service** pour en bénéficier.

### 3. Les zones protégées se contournaient avec un chemin écrit en `/`

- **Gravité** : moyenne — l'approbation humaine restait en place.
- **Constat** : la garde d'écriture comparait les chemins en texte brut.
- **Trouvé par** : la CI Windows de GitHub, au premier passage. Les tests
  locaux tournaient sous Linux et ne pouvaient pas le voir.
- **Correctif** : comparaison par composant, casse et séparateurs normalisés.
  Commit public `be323ce`.

### 4. Le relais acceptait des en-têtes au-delà de sa borne

- **Gravité** : basse.
- **Constat** : la borne de 8 Ko n'était pas appliquée quand la requête
  arrivait en plusieurs blocs.
- **Correctif** : borne sur le cumul. Tests du relais : 4 → 10. Commit public
  `b3b9673`.

### 5. Au premier lancement, des fils de fond mouraient en silence

- **Gravité** : fiabilité, pas confidentialité.
- **Constat** : sur une base neuve, les fils de fond ouvraient la base pendant
  que le cœur la créait, échouaient et s'arrêtaient sans rien dire.
- **Correctif** : le cœur ouvre la base en premier, les autres attendent son
  signal. Dans la première version publique.

### 6. Le service répondait « scellé » sans avoir posé de filtre

- **Gravité** : haute — même famille que le n° 1 : un état déclaré qui ne
  correspond pas à la réalité.
- **Constat** (2026-10-06) : quand un programme rejoint le périmètre mais que
  le service ne voit pas son fichier, aucun filtre n'est posé — et le service
  répondait quand même « ok ». Mesuré : une copie de `curl` placée dans un
  dossier invisible pour le service a rejoint le périmètre, reçu « ok », et
  atteint `1.1.1.1`. La même copie dans un dossier visible : bloquée en 0 ms
  et inscrite au journal.
- **Trouvé par** : un essai de sortie réel, pendant qu'on vérifiait autre
  chose.
- **Correctif**, en deux temps :
  1. le service **refuse** désormais de répondre « scellé » pour un fichier
     qu'il ne voit pas ou pour lequel le noyau n'a rien posé ;
  2. l'app ne se fie plus au service : elle fait **elle-même un essai de
     sortie** (`sceau::sonder`) et n'affiche « tenu » que s'il n'a pas montré
     le contraire. L'essai vise une adresse de documentation (`192.0.2.1`,
     RFC 5737) : aucun hôte réel n'est contacté, que le scellé tienne ou non.
- **Attention** : le point 1 demande de **réinstaller le service**. Le point 2
  protège dès la mise à jour de l'app.

**La leçon des n° 1 et n° 6** : un état affiché doit venir d'un essai, pas
d'une déclaration. C'est maintenant le cas pour le scellé.

## Problèmes ouverts

Ce ne sont pas des défauts corrigés. Ce sont les limites actuelles, avec la
direction que nous comptons prendre. Chacune est une bonne porte d'entrée
pour contribuer : une idée, une mesure sur votre machine ou une proposition
de conception valent autant qu'un correctif.

### A. Un programme enfant d'un autre nom échappe au scellé

Le scellé filtre **par chemin d'exécutable**. Si un programme scellé en lance
un autre (un `curl`, un interpréteur), celui-ci n'est pas couvert.

- **Ce que ça protège quand même** : le modèle n'a aucun outil pour lancer un
  programme arbitraire, donc un modèle manipulé ne peut pas s'en servir. La
  limite concerne du code hostile qui tournerait *dans* un processus de Waly
  (une dépendance compromise, par exemple).
- **Mieux fait ailleurs** : le bac à sable d'Anthropic (`sandbox-runtime`,
  Windows en alpha) filtre par **identité d'un compte dédié** : tout ce que
  le programme lance porte la même identité et reste enfermé.
- **Direction** : étudier le filtrage par identité (compte dédié ou jeton
  restreint) pour les processus qui n'ont pas besoin du micro ni de l'écran,
  ou un courtier : seul le service lance les programmes auxiliaires.
- **Où aider** : une note de conception comparant les deux voies, avec leurs
  conséquences pour un assistant qui a besoin du micro et de la caméra.
- **Depuis le 2026-10-06** : la voie du compte dédié existe pour les *autres*
  agents, et pour leurs **dossiers** seulement (l'enclos,
  `ADR-2026-10-06-garde-l-enclos.md`). Leurs programmes enfants gardent le
  compte : la mesure est au banc. Il reste à filtrer le **réseau** par ce
  compte, et le problème reste entier pour les processus de Waly lui-même.

### B. Les sorties ouvertes ne sont pas scellées par destination

Une sortie que l'utilisateur ouvre passe par un programme séparé (le `curl`
de Windows), qui n'est pas scellé. Il ne va que là où le code l'envoie :
cette garantie-là repose sur le code, pas sur le noyau.

- **Mieux fait ailleurs** : même source. Le programme enfermé ne peut joindre
  qu'un **proxy local**, et c'est le proxy qui n'autorise que les domaines
  déclarés. Cela règle le problème des adresses qui changent, qu'un filtre
  par adresse ne règle pas.
- **Direction** : une passerelle à nous, scellée sauf vers un proxy local
  tenu par le service, qui n'accepte que l'hôte de la porte ouverte.
  Voir `ADR-2026-10-01-lot3-sorties-ouvertes.md`.

### C. Un moteur Ollama n'est pas dans le périmètre

Le périmètre couvre l'app, la voix et le moteur FastFlowLM. Ollama est un
programme tiers partagé, qui télécharge ses modèles lui-même : il garde son
accès réseau. On peut le sceller à la demande avec la brique `waly-seal`
(élévation requise) ; il ne peut alors plus télécharger.

- **Direction** : sceller Ollama par défaut et n'ouvrir sa sortie que le
  temps d'un téléchargement demandé. À concevoir avec B.

### D. Les sorties n'ont pas été jouées contre les vrais services

Claude, Mistral, OpenAI et Telegram n'ont pas été appelés ; le chemin est
prouvé contre des serveurs locaux. Le relais de partage n'a pas été essayé à
travers internet, et il n'existe pas de relais public.

- **Où aider** : essayer avec votre propre clé et dire ce qui marche. Ne
  collez jamais une clé dans une issue.

### E. Vérifier demande aujourd'hui de compiler

Aucun binaire signé n'est distribué. Tant que c'est le cas, « vérifiez
vous-même » s'adresse à qui sait compiler.

- **Direction** : signature de code, puis une version installable. Un build
  reproductible serait la vraie réponse : pouvoir vérifier que le binaire
  vient bien de ce code.

### F. Le scellé n'a été mesuré que sur une machine

- **Où aider** : lancer la brique sur votre Windows et rapporter ce que vous
  observez (formulaire « Measurement on my machine »). Les configurations qui
  nous intéressent le plus : un VPN ou un antivirus qui pose ses propres
  filtres, une machine d'entreprise, une installation dans un dossier
  inhabituel.

## Signaler un défaut

Voir [`SECURITY.md`](../SECURITY.md). Un contournement du scellé est traité
comme critique, et s'ajoute à ce document une fois corrigé, avec le nom de la
personne qui l'a trouvé si elle le souhaite.
