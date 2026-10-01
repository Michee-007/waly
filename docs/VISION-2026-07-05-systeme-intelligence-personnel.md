# Waly — Système d'intelligence personnel (premiers principes)

> Socle premier-principe du produit, établi avec Michée le 2026-07-05.
> Tout le reste — identité visuelle, app desktop, feuille de route — se
> dérive d'ici. Ce document précède la RFC (architecture) et la gouverne.

## En une phrase

**Waly est ton compagnon d'intelligence personnelle — le premier et principal
*navigateur de connaissance agentique* : il te voit, te connaît, bosse avec toi
et pour toi, sur un socle 100 % local et souverain.**

> Précision de modèle (2026-07-06) : Waly n'est **pas** « le système ». Une
> présence chaude et nommée ne peut pas être une infrastructure. Voir
> « Architecture conceptuelle » plus bas — Socle / Waly / Navigateurs.

## Le problème, dépouillé

Une personne n'a **aucune intelligence qui soit vraiment la sienne**. Les IA
actuelles sont *dans le cloud* (elles voient tout ce que tu leur confies et le
monétisent), *sans mémoire* (elles t'oublient à chaque session), et ce sont
*des outils qu'on opère*, pas une présence qui te connaît.

Le vrai besoin : **une intelligence à qui je peux me confier et me fier en
continu, parce qu'elle est à moi et ne me quitte jamais.** Souveraineté **+**
intimité.

## Le principe qui rend tout possible

Le « 100 % local » n'est **pas une feature de confidentialité** — c'est **la
condition qui autorise l'intimité**. Tu ne peux laisser une caméra et un micro
allumés, partager ton écran, tes fichiers, ta journée entière — **que** parce
que rien ne sort. Le local est ce qui permet à Waly de *te voir* sans que ce
soit une intrusion.

## Ce que Waly fait — trois verbes

**Waly VOIT** *(perception)* — il te voit en **appel vidéo** et réagit à ton
état ; il voit ton **écran**, peut le **prendre en main** et te **guider à la
souris** ; il voit ce que tu fais **en continu** (mémoire ambiante).

**Waly CONNAÎT** *(savoir)* — il **se souvient de toi** ; il **relie tes
documents entre eux** ; il sait **où est chaque fichier**, peut le **consulter
et le modifier** — quand *tu* le décides.

**Waly FAIT** *(action)* — il **bosse avec toi et pour toi** (cowork), va sur
le **web**, agit — toujours **supervisé, sous garde-fous** (murs financiers,
approbations humaines).

**Trois portes d'entrée** : lui **parler**, l'**écrire**, l'**appeler en
vidéo** (pour qu'il voie).

**Fondation** : **tout en local**, avec un **cloud optionnel** que
*l'utilisateur* débloque pour plus de puissance (BYOK — jamais nos serveurs).

## Architecture conceptuelle — Socle / Waly / Navigateurs (2026-07-06)

Trois niveaux, à ne **jamais confondre** (et à ne PAS exposer à l'utilisateur —
il ne rencontre que **Waly**) :

1. **Le Socle** *(le « système », impersonnel — nom de code `waly-core`)* : la
   fondation invisible et souveraine — ta **mémoire**, tes **fichiers**, la
   garantie « rien ne sort », le **runtime** qui héberge les agents, et
   l'**orchestration/routage**. Aucune personnalité. On ne lui parle pas.

2. **Waly** *(la présence — PAS le système)* : le **compagnon**. Le visage
   (l'**éclipse**), la voix, la **relation** (il te connaît, tu te confies), et
   le **chef d'orchestre**. Waly est le **premier et principal navigateur** ;
   il habite le Socle et mobilise les autres. Waly porte l'**intimité** ; le
   Socle porte la **souveraineté**. C'est *lui* qu'on appelle.

3. **Les Navigateurs** *(la rangée)* : les agents spécialisés, chacun défini
   par ses **attributs + capacités**, que Waly (et toi) mobilisez. La feuille
   de route EST cette séquence de navigateurs qui se branchent : Regard
   (caméra, R4), Écran (R5), Mains (web/computer-use, R6), Ambient (R7), plus
   Fichiers / Liens (graphe) / Cowork. Les **outils de R2** (mémoire, tâches,
   rappels) sont les **premiers navigateurs**.

**Nommage** : un seul nom vécu par l'utilisateur — **Waly** (le produit *et* la
présence, modèle Alexa/Jarvis, pas macOS+Siri). Le Socle garde son nom de code
`waly-core`, jamais montré. Les navigateurs spécialisés auront leurs noms
propres au fil de l'eau. On ne brande la **plateforme** à part *que* si elle
devient un jour un **écosystème/marché** de navigateurs pairs (dont des tiers)
— décision stratégique reportée, pas une nécessité.

**Côté code, on ne reconstruit pas — on nomme mieux** : `waly-core` = déjà le
Socle (mémoire + souveraineté + registre + gates) ; le prompt/voix = déjà
Waly ; les outils = les premiers navigateurs. On *généralise* « outil » →
« navigateur » quand le cadre d'hébergement arrivera.

## Pour qui

**Pour Michée d'abord** (c'est son système personnel — boussole : *est-ce que
je ne peux plus m'en passer*), **et pour tous** (grand public confidentialité,
FR/EU d'abord), **puis pour les professions qui en ont le plus besoin**
(données sensibles : le 100 % local devient argument de vente).

## L'âme — la relation

Waly n'est **ni un majordome** (trop servile — on veut du *cowork d'égal à
égal*), **ni un cerveau passif en fond** (il *prend le contrôle, agit,
guide*), **ni un simple confident** (il *fait le travail*).

> **Waly est un compagnon *et* ton bras droit.** Assez intime pour que tu te
> confies et qu'il te voie ; assez capable pour que tu lui passes les
> commandes. Il agit toujours **en ton nom, sous ton autorité.**

Deux faces, une âme : **présence** (il voit, connaît, chaleureux) +
**capacité** (il fait, agit, sous ton contrôle).

## L'insight qui gouverne tout le design

Plus Waly **voit tout** et **fait tout** (ton écran, tes fichiers, ta journée),
plus il serait — pour n'importe qui d'autre — **terrifiant**. Ce qui le rend
*désirable* au lieu d'*effrayant* tient en une sensation permanente :

> **« Cette intelligence est puissante, elle voit tout — et elle est
> indiscutablement *à moi*, sous mon contrôle, elle ne fuit jamais. »**

Donc l'identité et l'app doivent irradier **trois choses à la fois** :
- **VIVANTE** — une présence, pas un outil ;
- **CAPABLE** — une puissance calme, qui sait faire ;
- **SOUVERAINE** — tienne, sûre, sous ton autorité, jamais fuyante.

C'est cette triade qui rend acceptable une intelligence qui te regarde vivre.

## Ce que Waly n'est PAS

Pas un chatbot (on ne « prompte » pas une présence). Pas une app de
productivité (les tâches sont un moyen, pas la fin). Pas un dashboard de
métriques. Pas le cloud. Pas une IA qui s'impose — elle attend, voit, et agit
quand tu le décides.

## Ce que ça impose au reste

- **Identité visuelle** : exprimer *vivante + capable + souveraine* — une
  présence qu'on ne craint pas parce qu'elle est nôtre. Ni mascotte mignonne,
  ni outil froid.
- **App desktop (R3)** : le **foyer** de cette présence — d'abord parler /
  écrire / (bientôt) appeler, mémoire et actions à portée, architecturée pour
  accueillir la vue (écran, caméra), l'action (web, computer-use) et
  l'ambiant. On bâtit le vaisseau, pas une page de plus.
- **Modalités simultanées** : voix, texte, appel vidéo — une seule présence
  derrière les trois.
- **Souveraineté visible** : à tout moment on doit *sentir* que rien ne sort
  et que Waly attend notre feu vert pour le sensible.

## Signature visuelle — l'éclipse (validée le 2026-07-06)

**La présence de Waly est une éclipse totale, réduite à son anneau.** Un
disque dont l'intérieur **se fond dans le fond**, cerné d'un **bord épais** qui
seul dessine le cercle, avec des **protubérances roses** posées sur l'anneau.
Deux thèmes selon le fond : **blanc** (bord foncé) / **noir** (bord clair).

- **Pourquoi ça tient le socle** : le cœur d'ombre = *souverain, mystérieux* ;
  l'anneau = *présent, vivant* ; les roses = la seule couleur, un signe de vie.
  Pas de corona/aura/brillance — épuré, jamais tape-à-l'œil.
- **Vivant, discret** : l'anneau et les roses **frémissent** et réagissent à
  l'état (repos → écoute → réflexion → parole), **sans rotation ni saccade** —
  « comme une vraie éclipse », presque immobile.
- **Différenciation** : le marché des IA a convergé sur l'orbe/l'étincelle/la
  lumière ambiante ; personne n'utilise l'éclipse → signature ownable.
- **Rendu de référence** (Canvas, 100 % local/inline, à porter dans la
  webview) : [`docs/identity/eclipse-signature.html`](identity/eclipse-signature.html).
- **À venir (R4)** : l'avatar d'appel **voxel** (genré par la voix) reste le
  second registre, pour le mode appel — distinct de cette présence ambiante.

Historique de la recherche (10+ directions écartées avec Michée) : blob
iridescent, présence voxel, esprit en poussières, soleil (rayons/lobes) — tous
recalés au profit de l'éclipse. Méthode : maquettes live, verdict à l'œil sur
options nommées, on grave et on supprime le reste.
