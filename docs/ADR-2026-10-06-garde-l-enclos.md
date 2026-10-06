# ADR 2026-10-06 — La Garde, étape 4 : l'enclos

## Contexte

La Garde sait couper à Waly chacun de ses accès, couper internet à un autre
agent, le figer, et voir ce qu'il touche. Il restait une chose qu'elle ne
savait pas faire : **empêcher un autre agent d'ouvrir un dossier**. Voir n'est
pas empêcher (ADR « voir ce que les programmes touchent », même jour).

Un programme lancé par l'utilisateur a les droits de l'utilisateur : rien, dans
le même compte, ne peut lui retirer un dossier sans le retirer à tout le
monde. Les bacs à sable d'agents publiés en 2026 pour Windows (Anthropic
`sandbox-runtime`, OpenAI Codex) sont arrivés à la même réponse : un **compte
à part**. C'était l'étape 4 convenue avec Michée : « l'enclos (compte Windows
à part) pour couper un dossier à un autre agent ».

## Décision

1. **Un compte Windows local, `WalyEnclos`**, sans groupe, caché de l'écran
   d'ouverture de session. Un agent « mis dans l'enclos » est fermé puis
   relancé sous ce compte.
2. **C'est Windows qui refuse, pas Waly.** Le compte n'a aucun droit sur le
   profil de l'utilisateur : la règle vient du système. On lui **donne** un
   dossier (lecture, ou lecture et écriture), on le lui **reprend**, on lui
   **coupe** un dossier situé hors du profil (refus explicite, hérité par les
   sous-dossiers). « Lecture » pose aussi un refus d'écrire : hors du profil,
   tout compte écrit déjà.
3. **Une seule élévation, à la création du compte** (`waly-seal-svc enclos
   creer`). Tout le reste se fait sans élévation, dans l'app : l'utilisateur
   règle les droits de dossiers qui lui appartiennent, et lance un programme
   sous l'autre compte par l'ouverture de session secondaire de Windows.
4. **Le mot de passe du compte** (36 caractères tirés au hasard) est créé par
   l'app, remis au programme élevé par un fichier du profil que celui-ci
   efface, jamais en argument, puis gardé chiffré par Windows (DPAPI) dans la
   base. L'agent de l'enclos ne peut ni lire ce fichier ni le déchiffrer ; et
   ce secret n'ouvre que son propre compte, moins puissant que celui de
   l'utilisateur.
5. **Chaque réglage est suivi d'un essai** (piège 14). Une sonde lancée sous
   le compte de l'enclos tente réellement de lire le dossier, puis d'y
   écrire ; l'interface montre « vérifié » ou « NON tenu ». « Refaire l'essai »
   rejoue toutes les sondes, plus celle du profil, qui doit rester fermé.
6. **Ce qu'il faut pour démarrer est donné d'office, et montré.** Un agent
   installé dans le profil (Ollama, Claude) ne pourrait pas lire son propre
   programme : son dossier est donné en lecture, avec la mention « pour
   démarrer X ». Si ce don échoue, l'instance en cours n'est pas fermée.
7. **La mémoire de Waly est coupée d'office.** Sa base vit dans `C:\waly\data`,
   hors du profil : sans coupure, l'enclos la lirait. Elle est coupée à la
   création, essayée, et ce réglage ne se retire pas.
8. **Les processus de l'agent sont tenus dans un « job » Windows** : on sait
   lesquels tournent (lui et ce qu'il lance), on les arrête ensemble. L'agent
   garde lui-même une poignée du job : il reste retrouvable après un
   redémarrage de Waly.
9. **L'enclos tient sans Waly.** Les droits sont dans le système de fichiers ;
   Waly fermé, un agent de l'enclos n'y gagne rien.

## Ce que cela ne fait pas

- **Hors du profil, rien n'est fermé par défaut.** Un autre disque, `C:\projets`,
  `C:\Users\Public` : l'enclos y lit et y écrit comme tout compte de la
  machine. Il faut couper ce qu'on veut protéger. L'interface le dit.
- **L'enclos est commun.** Ce qu'on donne, tous les agents de l'enclos le
  voient. Un compte par agent isolerait mieux ; pas fait.
- **Le réseau n'est pas concerné.** Un agent de l'enclos sort sur internet
  tant qu'on ne le scelle pas (par programme, comme avant). Filtrer par
  compte plutôt que par programme couvrirait aussi ses programmes enfants :
  c'est la suite naturelle, et la réponse au problème ouvert A de l'audit.
- **L'agent repart de zéro.** Ses réglages et sa connexion sont dans le profil
  de l'utilisateur, qu'il ne voit plus. On lui donne le dossier voulu, ou on
  le reconfigure.
- **Une instance relancée à la main sort de l'enclos.** Elle tourne sous le
  compte de l'utilisateur. La Garde la compte et l'affiche (« n processus hors
  de l'enclos »), elle ne l'empêche pas.
- **Un dossier dont l'héritage est coupé** ne reçoit pas le réglage de son
  parent. L'essai porte sur le dossier réglé, pas sur chacun de ses
  sous-dossiers.
- **Un compte de plus sur la machine**, visible dans les outils
  d'administration. `waly-seal-svc enclos supprimer` (console élevée) le
  retire avec son profil.

## Mesures (`lab/garde-banc`, essais 3 et 4)

Essai 3, faisabilité (PowerShell, compte d'essai créé puis supprimé) :

| Question | Mesure |
|---|---|
| Créer le compte | 40 ms, sans groupe ; session ouverte sans groupe |
| Lancer sous le compte, sans élévation | oui ; les programmes enfants gardent le compte |
| Profil de l'utilisateur, sans rien régler | fermé (dossier, Documents, Bureau, `.ssh`, programmes) |
| Hors du profil, sans rien régler | lit et écrit (`C:\waly`, `C:\Users\Public`) ; lit `C:\Program Files` |
| Donner en lecture, en écriture, reprendre | tient à chaque étape ; 24 ms |
| Couper hors profil, rendre | tient ; 14 ms ; 25 ms sur un dossier de 100 entrées |
| L'enclos lève-t-il sa propre coupure ? | non (accès refusé) |
| Programme installé dans le profil | ne se lance pas, puis se lance une fois son dossier donné |
| Réseau depuis l'enclos | sort (rien n'est réglé) |

Essai 4, le code livré (`waly enclos …`, compte réel) : douze réglages
enchaînés, tous confirmés par la sonde ; trois refus attendus (tout le profil,
la mémoire de Waly, un lecteur) ; un programme lancé, suivi (2 processus),
arrêté ; réglage et sonde en 190 ms. Deux défauts trouvés par cet essai et
corrigés : « reprendre » laissait le refus en place (`REVOKE_ACCESS` ne retire
pas les refus), et le nom du job disparaissait avec le lanceur.

## Conséquences

- `waly-seal` gagne `enclos.rs` et la commande `enclos creer|supprimer|etat` ;
  `waly-core` gagne `enclos.rs` et le CLI `waly enclos`. Le protocole du
  service (canal nommé) ne change pas.
- Le service installé doit être **réinstallé** pour que l'app puisse créer le
  compte (sinon : « service trop ancien »).
- La mise à jour du service ne touche pas au compte ; la désinstallation de
  Waly non plus pour l'instant (à brancher dans l'installeur).

## Essai sur l'app, avec un vrai agent (2026-10-06, soir)

`apps/desktop/e2e/e2e-garde-enclos.js` pilote la vraie interface. Agent
d'essai : un Claude Code installé par WinGet, lancé pour l'occasion (les
deux autres « Claude » de la machine, dont la session qui menait l'essai,
ne sont pas touchés : le script vérifie le programme affiché avant de
cliquer). Tout est passé :

- l'agent est vu sous le compte de l'utilisateur, choisi sur le graphe ;
- « Mettre dans l'enclos » : fermé, son dossier d'installation donné en
  lecture d'office, relancé sous `WalyEnclos`, vu comme tel par la Garde ;
- le dossier personnel lui est fermé, chaque réglage confirmé par sa sonde ;
- un dossier de `Documents` donné en écriture (confirmé), « Refaire
  l'essai », puis repris en cliquant sa ligne ;
- « Sortir de l'enclos » : arrêté, retiré de la liste.

Les captures du README viennent de cet essai. Trois défauts d'affichage vus
sur les vraies données et corrigés : un long chemin débordait du panneau, les
nœuds se touchaient à cinq agents, le programme n'était pas rappelé dans le
panneau de l'enclos.

Observé : la relance dans l'enclos a pris de quelques secondes à plus de
45 s selon la charge de la machine (ouverture de session secondaire). Rien
ne l'indique à l'écran hormis « Relance… » : à améliorer.

## Non vérifié

- **La surveillance dans le service installé** : la demande d'accord de
  Windows n'a pas été validée pendant l'essai.
- Un agent Ollama, un agent à fenêtre, un agent Node ou Python dans l'enclos.
  Un agent qui dépend de variables d'environnement de l'utilisateur ne les
  retrouve pas sous l'autre compte.
- La tenue sur un dossier de plusieurs dizaines de milliers de fichiers.
