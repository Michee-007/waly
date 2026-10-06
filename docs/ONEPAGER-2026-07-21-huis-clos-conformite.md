# Huis clos — la confidentialité par l'architecture (one-pager)

> Waly, un système d'intelligence personnelle, sous ta garde. **Tu vois ce que l'IA touche. Tu coupes
> quand tu veux.** (Identité du 2026-10-06 ; ce document ne traite que du
> scellé réseau, le « Huis clos ».)
> Document destiné aux professions à secret (avocats, experts-comptables,
> médecins) et à leur DPO/RSSI. Rédigé pour être vérifiable, pas pour rassurer.
> Écrit le 2026-07-21, **mis à jour le 2026-10-06** : sorties ouvertes par
> l'utilisateur, périmètre exact, mesure de pose, renvoi vers l'audit.

## Le problème réglementaire

Utiliser une IA cloud sur un dossier couvert par le secret professionnel, c'est
transmettre ce dossier à un tiers. En France : secret professionnel
(art. 226-13 du Code pénal), guide déontologique du CNB (mars 2026) sur l'IA et
le secret de l'avocat, hébergement de données de santé (HDS) pour les médecins,
AI Act pleinement applicable en août 2026. Les rustines habituelles
(pseudonymisation, « on ne s'entraîne pas sur vos données ») ne changent pas le
fait de base : **les données sortent de votre poste.**

## La réponse de Waly : rien ne sort sans votre accord, et c'est prouvable

Waly tourne **en local** sur votre poste (modèle, voix, vision, mémoire), tant
que vous n'ouvrez pas vous-même une sortie.
Le **Huis clos** est une garantie *technique*, pas une promesse — et il est
**actif par défaut, en permanence** : dès l'installation, Waly **bloque au
niveau du noyau Windows** toute sortie réseau de ses propres processus. Il n'y
a rien à activer, aucun bouton à ne pas oublier : comme un logiciel de
confiance, Waly est simplement toujours clos. Toute tentative de sortie est
**journalisée** dans un registre local que vous pouvez ouvrir et exporter.

**Les seules sorties sont celles que vous ouvrez.** Depuis octobre 2026, vous
pouvez ouvrir vous-même, une par une : un modèle extérieur avec votre propre
clé, une passerelle de messagerie, l'envoi d'une conversation à un contact, le
téléchargement d'un modèle. Chaque sortie est affichée tant qu'elle est
ouverte et inscrite au journal. Les processus de Waly, eux, restent scellés :
la sortie passe par un programme séparé, vers la seule adresse déclarée. Un
modèle extérieur ne reçoit que le texte de la conversation affichée — jamais
la mémoire, les instructions, l'écran, les images ni les outils. **Pour un
dossier couvert par le secret : n'ouvrez aucune sortie.** Le contrat reste
alors celui du titre.

### Comment, concrètement (pour votre RSSI)

- Le blocage s'appuie sur la **Windows Filtering Platform (WFP)** — le moteur
  de filtrage du noyau, celui du pare-feu Windows lui-même — via la DLL système
  signée par Microsoft (`fwpuclnt.dll`). Aucun composant tiers non signé.
- Un **service Windows tournant en compte SYSTEM** pose les filtres. Le
  privilège reste dans SYSTEM : **votre session utilisateur ne peut pas
  manipuler le sceau**, donc un logiciel malveillant s'exécutant sous votre
  compte ne peut pas le percer en silence.
- Pour chaque processus de Waly, deux règles : **autoriser** le trafic
  interne à la machine (loopback, pour que la voix et le modèle continuent de
  fonctionner) et **bloquer tout le reste**. Un blocage de ce niveau ne peut
  pas être annulé par une autre règle de pare-feu.
- **Fail-closed** : si le service tombe, les filtres **restent** (rien ne
  fuit) ; ils ne sont retirés qu'explicitement.

### Vérifiez-le vous-même (30 secondes)

Dans Waly, cliquez sur le bouton **« Garde »**, en haut de la fenêtre, puis sur
**Refaire l'essai** :
Waly tente une vraie connexion sortante (vers un serveur public neutre). Elle
est **bloquée**, et la tentative apparaît immédiatement dans le journal, datée,
avec l'adresse visée. La **démo avion** est la version radicale : coupez le
Wi-Fi en pleine conversation, la conversation continue.

Waly fait aussi cet essai **de lui-même**, sans rien contacter (vers une
adresse de documentation) : la mention « Scellé · vérifié » n'est affichée que si
l'essai n'a pas montré le contraire.

Ce que l'essai prouve exactement : que le noyau bloque une sortie **du
processus de Waly**. Il ne prouve pas que rien ne quitte la machine (voir les
limites ci-dessous). Et tant qu'aucun binaire signé n'est distribué, le faire
soi-même demande de compiler l'application.

## Ce que le sceau garantit — et ce qu'il ne garantit PAS

L'honnêteté fait partie de la garantie. Le sceau :

- **couvre les processus de Waly** (l'application, la voix, le moteur
  d'inférence FastFlowLM), **pas la machine entière** : votre navigateur ou un
  autre logiciel gardent leur accès réseau. Le contrat est précis — *ce qui
  entre dans la session Waly ne ressort pas par Waly*.
- **ne couvre pas un moteur Ollama.** Si vous utilisez Ollama, c'est un
  programme tiers partagé, qui télécharge ses modèles lui-même et garde son
  accès réseau. Vous pouvez le sceller à la demande (voir plus bas, élévation
  requise) ; il ne pourra alors plus télécharger de modèle.
- **filtre par programme, pas par descendance.** Un autre programme lancé par
  un processus de Waly n'est pas couvert. Le modèle n'a aucun outil pour en
  lancer un ; la limite concerne du code hostile qui tournerait dans Waly.
- **ne scelle pas encore par destination les sorties que vous ouvrez.** Le
  programme séparé qui les porte (le `curl` de Windows) n'est pas scellé. Il
  ne va que là où le code de Waly l'envoie : cette garantie-là repose sur le
  code, pas sur le noyau.
- **ne persiste aucun pixel ni aucun texte d'écran brut** (règle de conception,
  vérifiable en base) : seule une description d'une ligne peut être gardée dans
  la mémoire locale, jamais le contenu capté.
- **s'appuie sur un composant d'interface (le moteur de rendu WebView2, partagé
  avec le système) qui n'est pas couvert par le blocage WFP** — car c'est un
  exécutable partagé, le filtrer casserait d'autres logiciels. Il est durci
  autrement, en défense en profondeur : l'interface ne charge QUE des fichiers
  locaux ; une **politique de sécurité de contenu (CSP)** stricte interdit à la
  page toute connexion sortante (`connect-src` limité au canal interne) ; et le
  moteur de rendu est lancé avec le **réseau de fond désactivé** (télémétrie,
  vérification de réputation, synchronisation, sondes — coupés). Ce point est
  documenté ici plutôt que caché.
- **peut être retiré par un administrateur de la machine** (c'est le propre de
  tout dispositif local) — mais la **pose et la levée du sceau sont
  journalisées** : une interruption de couverture se voit.

## Le huis clos pour n'importe quel agent (pas seulement Waly)

Le même mécanisme peut fermer le réseau de **n'importe quel programme** de la
machine — un autre agent IA local, un outil tiers — pas seulement Waly. C'est
une brique indépendante : en ligne de commande
(`waly-seal-svc seal <programme>` / `unseal` / `list` / `journal`) ou depuis
l'application (Vie privée › « Autres agents sur cette machine »), qui liste
les agents connus en cours d'exécution et ce que chacun a tenté une fois
scellé. Parce que sceller un programme tiers bloque **son** réseau, cette
action **exige une élévation** (Windows demande l'autorisation une fois) : un
logiciel ordinaire ne peut pas couper le réseau d'un autre en silence.

Trois limites à connaître. Le blocage est par **chemin de programme** : un
agent écrit en Python ou en Node tourne sur un moteur que d'autres logiciels
utilisent, et le sceller les coupe aussi (l'application le dit, avec le nombre
de programmes concernés, avant de sceller). Le scellé ne voit **que le
réseau** : il ne surveille pas les fichiers qu'un agent lit ou modifie. Et
seuls les agents que Waly sait reconnaître sont listés ; pour les autres, on
choisit le programme à la main.

## Où en est le produit (transparence sur la maturité)

Le mécanisme est **implémenté et mesuré** sur une machine de référence
(blocage effectif, pose du scellé en ~59 ms par le service, coût réseau
interne nul, journal d'audit fonctionnel). Restent avant distribution : la
**signature de code** du service et de l'application (en cours de décision),
l'essai des sorties contre les vrais services, et la validation par des
professionnels du secret en conditions réelles.

Nous avons rejoué nos propres promesses avant de publier : six défauts
trouvés et corrigés, six problèmes encore ouverts. Tout est écrit dans
[`AUDIT-2026-10-02-promesses-rejouees.md`](AUDIT-2026-10-02-promesses-rejouees.md).
Ce document décrit ce qui existe, pas une feuille de route.

## En un mot

Les autres vous demandent de leur **faire confiance** avec vos dossiers. Waly
vous laisse **vérifier** que rien ne part — et vous donne le journal pour le
prouver à un tiers. C'est la confidentialité par l'architecture, pas par la
politique de confidentialité.
