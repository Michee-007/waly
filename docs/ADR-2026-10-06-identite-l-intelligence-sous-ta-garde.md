# ADR 2026-10-06 — Identité : « Waly, un système d'intelligence personnelle, sous ta garde »

Remplace le nom d'identité et la devise du
[RFC du 2026-07-20](RFC-2026-07-20-identite-produit-l-intelligence-qui-reste.md).
Le reste de ce RFC (piliers, publics, refus de la mémoire ambiante) tient.

## Pourquoi changer

L'identité du 20 juillet disait : **« Waly, l'intelligence qui reste »** /
**« Il voit tout. Rien ne sort. »** Trois choses l'ont rendue fausse ou trop
étroite.

1. **« Rien ne sort » n'est plus vrai dès qu'on ouvre une porte.** Depuis le
   lot 3 (2026-10-01), l'utilisateur peut brancher un modèle en ligne avec sa
   clé, un téléphone, un partage. Michée, le 6 octobre : « trompeur dès qu'on
   met une clé API ». L'interface a déjà cessé de le dire ; le dépôt le disait
   encore.
2. **Le sujet a changé de côté.** L'ancienne devise parle de ce que *Waly*
   voit. Ce que le produit apporte depuis la Garde, c'est ce que *tu* vois :
   chaque geste de l'IA inscrit, chaque accès coupable d'un clic. La
   définition de Michée : « voir ce que l'IA touche, c'est la bonne définition
   d'un système ».
3. **Waly ne garde plus que lui-même.** Les étapes 2 à 4 de la Garde portent
   sur les *autres* agents de la machine : leur couper internet, les figer,
   voir ce qu'ils touchent, leur fermer un dossier. « L'intelligence qui
   reste » ne dit rien de cela.

La thèse, elle, n'a pas bougé
([recherche du 6 octobre](RESEARCH-2026-10-06-these-et-etat-de-l-art.md)) : un
système d'intelligence personnelle ne vaut que ce que son propriétaire peut
vérifier à bas coût. L'identité doit la dire.

## Décision

| | Français | Anglais |
|---|---|---|
| Ce qu'est Waly | **un système d'intelligence personnelle** | **a personal intelligence system** |
| Nom d'identité | **Waly — un système d'intelligence personnelle, sous ta garde** (court : « l'intelligence sous ta garde ») | **Waly — a personal intelligence system, under your watch** (short: "AI under your watch") |
| Devise | **Tu vois ce que l'IA touche. Tu coupes quand tu veux.** | **See what AI touches. Cut it off whenever you want.** |
| En une phrase | Un système d'intelligence personnelle pour Windows : un assistant qui t'entend, te voit, se souvient et agit, et le poste de garde des agents IA de ta machine. | A personal intelligence system for Windows: an assistant that hears you, sees you, remembers and acts, and the guard post for the AI agents on your machine. |

**« Système d'intelligence personnelle » est gardé** (demande de Michée, le
soir même : « je veux vraiment renvoyer l'image d'un système »). C'est le nom
de la catégorie depuis la vision du 2026-07-05 et le fil de ses prises de
parole. Le mot « système » est tenu par deux faits, pas par l'ambition : Waly
réunit perception, mémoire et action dans un même ensemble ; et la Garde place
sur un même graphe Waly *et* les autres agents de la machine. La définition de
Michée sert de critère : un système, c'est ce dont on peut voir ce qu'il
touche. Dans l'app, la phrase figure dans Réglages › À propos.

**« Sous ta garde »** a deux sens, et les deux sont voulus : *confiée à toi*
(ta mémoire, tes fichiers et tes clés restent sous ta main) et *sous ton
regard* (elle agit devant toi). C'est aussi le nom de la page où tout se
passe : la Garde.

**La devise retourne l'ancienne.** « Il voit tout » devient « Tu vois » :
celui qui regarde n'est plus l'assistant, c'est son propriétaire.

## Ce que chaque mot couvre, et ne couvre pas

La devise ne dit ni « tout » ni « rien ». Voici ce qu'elle recouvre le jour où
elle est gravée ; toute phrase publique doit rester dans ces bornes.

| | Waly | Un autre agent |
|---|---|---|
| **Tu vois** | Chaque geste, inscrit par notre code : le fichier, le geste, jamais le contenu. Exact. | Ce que Windows rapporte de lui, si tu allumes la surveillance : fichiers ouverts ou écrits, programmes lancés, adresses contactées. Trié : un geste peut manquer. |
| **Tu coupes** | Fichiers, mémoire, écran, caméra, micro : un clic, immédiat, la tentative suivante est refusée et notée. Internet : fermé d'origine, tu ouvres porte par porte. | Internet (accord Windows). Tout (le figer). Un dossier, s'il est dans l'enclos. Pas un dossier à un agent resté sous ton compte. |
| **Quand tu veux** | À tout moment, sans redémarrer. | Mettre un agent dans l'enclos le ferme et le relance. |

Ce que l'identité ne dit pas : que Waly est plus intelligent, plus rapide ou
plus complet que les agents du marché. Il ne l'est pas.

## Mots retirés

- « Rien ne sort », seul : remplacé par « rien ne sort sans ton accord », ou
  par l'état prouvé (« Scellé · vérifié », « Une porte est ouverte »).
- « 100 % local », en bannière : Waly est *local et scellé par défaut* ; un
  modèle en ligne peut répondre si tu l'as branché, et l'interface dit alors
  « Waly » et non « Waly local ».
- « Il voit tout » : faux pour la surveillance (triée), et ce n'est pas la
  promesse.

« Huis clos » reste le nom du scellé réseau, qui n'a pas changé.

## Portée

Changés : les deux README, le one-pager de conformité, le README du crate
`waly-seal`, les instructions du dépôt, le dossier de publication, la consigne
du scellé donnée au modèle.

Inchangés : les documents datés (RFC, ADR, plans, recherches, journal). Ils
disent ce qui était vrai à leur date ; le RFC d'identité reçoit une note de
tête qui renvoie ici.

À faire hors du dépôt, par Michée : la description du dépôt GitHub, et le
texte de présentation du profil si l'ancien slogan y figure.
