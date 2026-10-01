# ADR 2026-09-14 — L'UI de Waly : la disposition de Claude, les noms et les règles de Waly

**Statut** : accepté (Michée, « c'est bon » sur la planche v3 du 14/09, avec « beaucoup plus blanc »).
**Planche de référence** : Artifact « Planche UI Waly » v3 (accueil, conversation, mission, voix, projets, créations, personnaliser, programmé, paramètres).

## Décision

L'app desktop (`apps/desktop/ui/index.html`) est réécrite sur la disposition de Claude, avec l'identité et le vocabulaire de Waly :

- **Barre latérale** : Nouvelle conversation, Rechercher, **Personnaliser** (Compétences · Connecteurs · Plugins — le « Carnet » disparaît), **Projets**, **Créations** (notre équivalent des artefacts), **Programmé** ; puis les listes **Missions** et **Conversations**, repliables par une petite flèche ; en pied, l'état du scellé et l'accès aux Paramètres. Aucun compte.
- **Saisie** unique : « + » (fichier, projet, **Partager l'écran** en conversation / **Enregistrer une compétence** en mission, compétences, connecteurs, plugins), bascule Conversation / Mission à l'accueil, réglages (réflexion), modèle, appel voix, envoyer.
- **Conversation** : Waly lit, répond et **crée** dans le fil ; les **approbations vivent en Mission**.
- **Voix** : la Marée plein écran, comme un appel ; la caméra fait passer l'appel en vidéo.
- **Paramètres** : Général, **Mémoire visible et modifiable**, Modèles, Voix, Capacités, Partage et contacts, Utilisation, Vie privée, À propos.
- **Palette blanche** (`#FFFFFF`, barre `#FAFAFA`, encre `#111111`) ; polices système (l'app est scellée, aucune police distante) ; styles en classes (la CSP Tauri à empreintes refuse les `style="…"`).
- **Honnêteté** : ce qui n'a pas encore de moteur (projets, passerelles, modèles extérieurs, routeur, partage et appel entre Waly, utilisation, aides en parallèle, plugins, réflexion) est affiché « bientôt », jamais simulé.

Branché dès aujourd'hui : conversations et missions, approbations au libellé, écran partagé et « Regarde-moi », appel voix/vidéo et éveil, compétences apprises et intégrées, connecteurs MCP, créations (Documents\Waly), rappels et tâches (annulation depuis Programmé), **mémoire lue / corrigée / oubliée / ajoutée** (4 nouvelles commandes : `core_memoires`, `core_memoire_maj`, `core_memoire_oublier`, `core_rappel_annuler`), machine et recommandation de modèle, preuve du scellé.

## Est-ce cohérent ? Oui — à trois conditions qui deviennent des règles

1. **« Rien ne sort » devient « rien ne sort sans ton accord ».** Passerelles de messagerie, modèles extérieurs, téléchargement de modèles, partage et appel entre utilisateurs sont des **sorties que l'utilisateur ouvre une par une**, listées et coupables dans Vie privée (déjà l'esprit de l'ADR 2026-07-21 : « l'accès extérieur sera une capacité qu'on active »). Conséquence technique : le scellé WFP est **par exécutable** — ouvrir Telegram pour `waly.exe` ouvrirait tout `waly.exe`. Il faudra des autorisations **par destination** (filtres WFP adresse/port) ou un **processus passerelle séparé**, scellé à part. C'est le vrai chantier de ces briques.
2. **Un modèle extérieur ne reçoit jamais la mémoire, les fichiers ni l'écran** sans autorisation explicite pour ce message — imposé par le harness, pas confié au modèle. (La planche v3 disait « même mémoire quel que soit le modèle » : c'était contradictoire, corrigé ici.) Le routeur choisit un modèle ; il ne décide pas de ce qui sort.
3. **Les aides en parallèle ne sont réellement parallèles qu'avec de la marge** : sur la machine de référence (un seul modèle résident, 15 Go), elles attendent leur tour ; le parallélisme vrai viendra d'un modèle extérieur autorisé ou d'une machine plus grosse.

Et deux ajustements moteur à faire pour que l'UI dise vrai :

- **Conversation sans approbation** ⇒ « créer » = **nouveau fichier seulement, jamais d'écrasement** sans passer par une mission ; aujourd'hui `ecrire_fichier` demande une approbation partout, et `agir_ecran` est aussi disponible en conversation — à réserver aux missions.
- **Appeler un autre Waly sans compte** ⇒ identité par clé + code d'appairage ; direct sur le même réseau, mais à travers internet il faut un **relais de mise en relation** (auto-hébergeable) — une sortie de plus, déclarée comme les autres.

Rien d'autre (projets, créations, programmé, mémoire modifiable, réflexion désactivée par défaut, compétences) ne crée de tension.

## Ajouts du même jour (retour de Michée)

- **Supprimer et renommer** une conversation ou une mission : menu ⋯ sur chaque ligne de la barre latérale et en haut du fil, confirmation dans l'app. `store::delete_session` efface messages et journal visuel ; **le Fil principal ne se supprime pas** (la voix y écrit) ; **le journal du sceau reste** (c'est la preuve). Les projets auront la même action quand ils existeront.
- **Les fichiers s'ouvrent DANS l'app** : visionneuse à droite, depuis Créations et depuis les cartes du fil. `waly_core::apercu` (Rust pur : lecteur ZIP, Word avec ses tableaux, Excel par feuille, PowerPoint par diapositive, CSV, markdown, texte, images) et `waly_sight::pdf` (pages rendues par `Windows.Data.Pdf`). Aperçu limité au dossier Documents\Waly ; « Afficher dans l'Explorateur » reste en option. Piège : un chemin canonicalisé `\\?\…` est refusé par WinRT → préfixe retiré.

## Corrections du même jour (retour de Michée)

- **Plus de « Fil principal ».** La voix (bouton appel, veille réveillée par « Waly », appel vidéo) et le partage d'écran écrivent dans **la conversation ouverte**. Le desktop publie cette conversation au processus voix (`GET /session` du service d'appel), consultée à chaque écriture. La session 1 d'une ancienne base devient « Conversation vocale », supprimable comme les autres ; le stockage ne crée plus de session imposée et ne laisse jamais de messages orphelins. Le journal visuel reste global (clé interne). Limite : si l'on change de conversation pendant un appel, les messages suivent mais la mémoire courte de la voix reste celle du début de l'appel.
- **L'aperçu marche dans toutes les conversations**, pour tout fichier créé (Documents\Waly ou dossier pointé par une mission) : les cartes de fichiers sont mémorisées par conversation (`session_fichiers`) et reprennent leur place à la réouverture ; un fichier créé pendant un tour **s'ouvre tout seul** dans la visionneuse, comme chez Claude.
- **Pages HTML rendues** dans un cadre isolé (sandbox : ni scripts ni formulaires ; rien ne se charge depuis internet). Pour que leurs styles s'appliquent, Tauri ne modifie plus `style-src` (`dangerousDisableAssetCspModification: ["style-src"]`) — sinon ses empreintes annulent `'unsafe-inline'` ; les scripts restent sous empreintes.
- **Télécharger** (conversation) copie le fichier dans Téléchargements et l'y montre ; en **mission**, le bouton devient « Afficher dans le dossier » : la mission agit sur les fichiers en place.

## Conséquences

- Plan des briques neuves à écrire (ordre à choisir par Michée) : réflexion · conversation « créer sans écraser » · projets · sorties autorisées par destination (socle des passerelles, modèles extérieurs, téléchargements) · routeur · partage et appel · aides · utilisation · plugins.
- `popup.html` (pop-up du mode Écran) garde son style ; à aligner lors du chantier écran.
