# Huis clos — la confidentialité par l'architecture (one-pager)

> Waly, l'intelligence qui reste. **Il voit tout. Rien ne sort.**
> Document destiné aux professions à secret (avocats, experts-comptables,
> médecins) et à leur DPO/RSSI. Rédigé pour être vérifiable, pas pour rassurer.

## Le problème réglementaire

Utiliser une IA cloud sur un dossier couvert par le secret professionnel, c'est
transmettre ce dossier à un tiers. En France : secret professionnel
(art. 226-13 du Code pénal), guide déontologique du CNB (mars 2026) sur l'IA et
le secret de l'avocat, hébergement de données de santé (HDS) pour les médecins,
AI Act pleinement applicable en août 2026. Les rustines habituelles
(pseudonymisation, « on ne s'entraîne pas sur vos données ») ne changent pas le
fait de base : **les données sortent de votre poste.**

## La réponse de Waly : rien ne sort, et c'est prouvable

Waly tourne **100 % en local** sur votre poste (modèle, voix, vision, mémoire).
Le **Huis clos** est une garantie *technique*, pas une promesse — et il est
**actif par défaut, en permanence** : dès l'installation, Waly **bloque au
niveau du noyau Windows** toute sortie réseau de ses propres processus. Il n'y
a rien à activer, aucun bouton à ne pas oublier : comme un logiciel de
confiance, Waly est simplement toujours clos. Toute tentative de sortie est
**journalisée** dans un registre local que vous pouvez ouvrir et exporter.
(L'accès à des ressources externes — recherche web, etc. — n'existe pas
aujourd'hui ; le jour où il arrivera, ce sera une capacité que *vous activez*
explicitement, bornée et journalisée — jamais l'inverse.)

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

Dans Waly, cliquez sur **« rien ne sort »** dans la barre d'état (c'est la
preuve elle-même) puis sur **Vérifier maintenant** :
Waly tente une vraie connexion sortante (vers un serveur public neutre). Elle
est **bloquée**, et la tentative apparaît immédiatement dans le journal, datée,
avec l'adresse visée. La **démo avion** est la version radicale : coupez le
Wi-Fi en pleine conversation, la conversation continue.

## Ce que le sceau garantit — et ce qu'il ne garantit PAS

L'honnêteté fait partie de la garantie. Le sceau :

- **couvre les processus de Waly** (l'application, la voix, le moteur
  d'inférence), **pas la machine entière** : votre navigateur ou un autre
  logiciel gardent leur accès réseau. Le contrat est précis — *ce qui entre
  dans la session Waly ne ressort pas par Waly*.
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
l'application (Vie privée › « Sceller un autre agent »). Parce que sceller un
programme tiers bloque **son** réseau, cette action **exige une élévation**
(Windows demande l'autorisation une fois) : un logiciel ordinaire ne peut pas
couper le réseau d'un autre en silence. Le blocage est par **chemin d'exe** :
on ferme exactement le programme visé, rien d'autre.

## Où en est le produit (transparence sur la maturité)

Le mécanisme est **implémenté et mesuré** (blocage effectif, latence de pose
~5 ms, coût réseau interne nul, journal d'audit fonctionnel). Restent avant
distribution : la **signature de code** du service et de l'application (en
cours de décision), et la validation par des professionnels du secret en
conditions réelles. Ce document décrit ce qui existe, pas une feuille de route.

## En un mot

Les autres vous demandent de leur **faire confiance** avec vos dossiers. Waly
vous laisse **vérifier** que rien ne part — et vous donne le journal pour le
prouver à un tiers. C'est la confidentialité par l'architecture, pas par la
politique de confidentialité.
