# Waly — un système d'intelligence personnelle, sous ta garde

[![CI](https://github.com/Michee-007/waly/actions/workflows/ci.yml/badge.svg)](https://github.com/Michee-007/waly/actions/workflows/ci.yml)
[![Licence : MIT](https://img.shields.io/badge/licence-MIT-black.svg)](LICENSE)

**Un système d'intelligence personnelle pour Windows : un assistant qui
t'entend, te voit, se souvient et agit, et le poste de garde de tous les
agents IA de ta machine.** Tu vois ce que chacun touche (fichiers, mémoire,
écran, caméra, micro, internet) et tu le coupes. Waly est local et scellé par
défaut : voix temps réel (en français aujourd'hui, pensé pour être adapté à
l'anglais), vision (caméra « mode appel » + écran), mémoire persistante, mot
d'éveil, et un scellé réseau au niveau noyau qu'il prouve par un essai avant
de l'afficher.

*Tu vois ce que l'IA touche. Tu coupes quand tu veux.* — [English version](README.md)

![La Garde : le graphe relie Waly et les autres agents de la machine à ce qu'ils touchent ; un Claude Code est dans l'enclos, ses dossiers sont listés avec le résultat de leur essai](docs/images/garde.png)

*La Garde, capture réelle (base vide, vrais agents de la machine de
référence : l'app Claude, deux Claude Code et un Ollama ; quand deux agents
portent le même nom, la Garde dit d'où vient chacun). Chaque lien de Waly se
coupe d'un clic. Le Claude Code choisi est dans l'enclos : un compte Windows à part,
auquel Windows refuse tes dossiers tant que tu ne les donnes pas. Chaque
réglage est suivi d'un essai réel, et c'est son résultat qui est affiché.*

![Au premier lancement, Waly tente une vraie sortie devant toi : « Windows l'a bloqué. »](docs/images/preuve.png)

*Le premier lancement : Waly tente une vraie sortie vers internet sous tes
yeux, et montre ce que Windows en a fait.*

| La réflexion, montrée avant la réponse | Des modèles jugés pour TA machine |
|---|---|
| ![Un modèle local 4B écrit son raisonnement dans un bloc repliable, puis répond](docs/images/reflexion.png) | ![Catalogue de modèles avec les verdicts tient bien / lent / trop gros](docs/images/modeles.png) |

| Un appel vocal, sur ta machine | Un appel vidéo : Waly te voit, la caméra reste locale |
|---|---|
| ![L'écran d'appel : l'éclipse écoute, les sous-titres dessous, la mention « Local » tant qu'un modèle de la machine répond](docs/images/appel-voix.png) | ![Appel vidéo : Waly remarque la présence et l'attention ; l'image de la caméra ne quitte jamais la machine](docs/images/appel-video.png) |

![Le panneau Partage : une conversation reçue attend ton accord ; ton code Waly est ta clé publique](docs/images/partage.png)

*Une conversation envoyée par un contact, chiffrée de bout en bout, en
attente de ton accord. L'appel entre deux utilisateurs de Waly arrive ensuite
(voir plus bas).*

> ⚠️ **Statut : pré-alpha, machine de référence uniquement.** Waly est
> développé et mesuré sur une seule machine (AMD Ryzen AI 5 340 — NPU XDNA 2,
> iGPU Radeon 840M, 15 Go de RAM utile, Windows 11). Il y tourne de bout en
> bout aujourd'hui. La portabilité (autres NPU, GPU seul, plus de RAM) est le
> prochain front — les mesures et retours d'autres machines sont bienvenus.

## Pourquoi un système, et pas un assistant de plus ?

Un assistant répond. Un **système d'intelligence personnelle** tient
ensemble ce qui, autour d'une personne, perçoit, se souvient et agit : la
voix, la vue, la mémoire, les outils, et les autres agents qui tournent sur la
même machine. Michée, qui construit Waly, en donne cette définition : *on doit
pouvoir voir ce que l'IA touche.* Sans cela, ce n'est pas un système, c'est
une boîte.

Un tel système relie toute une vie numérique et agit pour une seule
personne. Ce qu'on peut lui confier ne dépend pas d'abord de son
intelligence, mais de ce qu'on peut **vérifier sans y passer sa vie**. Waly
prend cette vérification pour objet
([la thèse, et où c'est mieux fait ailleurs](docs/RESEARCH-2026-10-06-these-et-etat-de-l-art.md)).

1. **La Garde : voir ce que l'IA touche, et le couper.** Un bouton toujours
   visible ouvre une page : ce que Waly a touché aujourd'hui (le fichier, le
   geste, jamais le contenu), un graphe dont chaque lien se coupe ou se rend
   d'un clic, et le fil des événements. Les autres agents de la machine
   (Ollama, Claude, Codex, OpenClaw, Hermes…) y figurent aussi : tu peux
   leur couper internet, les figer, voir ce qu'ils ouvrent, et les mettre
   dans **l'enclos**, un compte Windows à part où Windows leur refuse tes
   dossiers. Ce qui tient vraiment et ce qui ne tient pas est écrit plus bas
   et dans les ADR
   ([voir](docs/ADR-2026-10-06-garde-voir-ce-que-les-agents-touchent.md),
   [l'enclos](docs/ADR-2026-10-06-garde-l-enclos.md)).
2. **Vie privée prouvable (« huis clos »)** : un scellé WFP (Windows
   Filtering Platform), posé par un service SYSTEM, bloque toute sortie
   réseau des processus de Waly au niveau noyau, garde le loopback (serveur
   de modèles) vivant, et journalise chaque tentative bloquée. Le sceau se
   *teste* depuis l'app, et Waly fait cet essai de lui-même avant d'afficher
   « scellé ». Enfermer le réseau d'un agent n'est pas nouveau :
   Anthropic, OpenAI et NVIDIA ont publié en 2026 des bacs à sable pour
   agents, parfois mieux conçus que le nôtre sur des points précis. Ce que
   nous n'avons pas trouvé ailleurs, c'est la combinaison : un assistant à
   voix et à vision, pour une personne, scellé par défaut, avec la preuve
   entre les mains de l'utilisateur. Voir la
   [comparaison honnête](docs/RESEARCH-2026-10-06-these-et-etat-de-l-art.md).
3. **Un assistant intégré, pas un chatbot** : cascade voix streaming avec
   barge-in, mode appel caméra avec mémoire visuelle, partage d'écran
   OCR-first, mot d'éveil, mémoire SQLite + embeddings locaux, outils
   supervisés avec approbations humaines — une seule app installable.
4. **Pas un assistant réservé à l'anglais — ni au français** : Waly a été
   construit d'abord en français (interface, voix, prompts), dans un
   écosystème presque entièrement anglophone. Il n'a pas vocation à rester
   français seulement : le modèle de langage est multilingue, et ce qui est
   français aujourd'hui est précisément ce qu'un **mode anglais** doit
   adapter. Ce mode n'est pas encore construit ; il est en tête de la
   feuille de route, et c'est un bon endroit où contribuer.

## Structure

- `apps/desktop/` — app Tauri 2 (webview vanilla JS, core Rust in-process),
  installeur NSIS
- `crates/waly-core/` — orchestrateur : chat, outils natifs, mémoire
  (SQLite + sqlite-vec + e5-small local), murs de sûreté, approbations HITL
- `crates/waly-voice/` — cascade voix : VAD Silero, STT Parakeet, fin de tour
  sémantique, TTS Pocket streaming (Piper en secours), mot d'éveil
- `crates/waly-sight/` — perception : caméra (YuNet), capture écran + OCR
  ONNX local, moments VLM
- `crates/waly-relais/` — le petit relais auto-hébergeable qui permet à deux
  Waly de s'échanger une conversation chiffrée de bout en bout (il ne garde
  que des enveloppes fermées)
- `crates/waly-seal/` — le service de la Garde : scellé réseau (filtres WFP
  par session, journal d'audit, fail-closed), suivi de ce que les programmes
  touchent, création du compte de l'enclos
- `engines/` — scripts d'installation/lancement des moteurs (FastFlowLM sur
  NPU, Ollama Vulkan en secours). Binaires et modèles hors git.
- `docs/` — ADR, RFC, mesures par phase (le journal de construction complet)
- `lab/` — bancs et spikes de recherche (ne shippe jamais)

## Démarrer

Prérequis : Windows 11, machine AMD Ryzen AI (XDNA 2) pour la voie NPU *ou*
n'importe quelle machine faisant tourner Ollama avec un modèle 4B, ~7 Go de
RAM libre. Voir le README anglais pour les commandes, et
[CONTRIBUTING.md](CONTRIBUTING.md) pour les deux boucles de build.

**Premier lancement : le bon modèle pour ta machine.** Waly détecte ta
RAM, ta carte graphique, ton NPU et Smart App Control, les compare aux
modèles déjà installés dans Ollama, et recommande un cerveau (plus un modèle
vision si le cerveau ne voit pas) avec les commandes `ollama pull` exactes.
Clique sur le nom du modèle dans la barre d'état, ou lance `waly materiel`.
Waly ne télécharge jamais rien lui-même — il est scellé.

Configuration : un fichier optionnel, `C:\waly\data\waly.toml` (copier
[`waly.toml.example`](waly.toml.example) ; `WALY_CONFIG` pour le déplacer) —
port et modèle du serveur local, ton prénom, chemin de la base. Les
variables d'environnement (`WALY_LLM_PORT`, `WALY_MODEL`, `WALY_USER`,
`WALY_DB`, `WALY_TTS`/`WALY_TTS_SPEAKER`) restent prioritaires.

**Outils de la communauté via MCP.** Déclare n'importe quel serveur MCP
stdio sous `[mcp.<nom>]` dans `waly.toml` (voir le fichier d'exemple). Un
serveur MCP est un processus tiers qui tourne *hors* du sceau réseau :
chaque appel d'outil MCP te demande ton accord, sauf si tu déclares le
serveur `confiance = "lecture"`.

**Vision adaptative.** Waly demande au moteur ce que sait faire le cerveau.
S'il voit, les images lui vont directement. Sinon (modèle texte seul),
déclare `modele_vision` dans `waly.toml` (ex. `gemma3:4b`) : ce modèle
regarde chaque image et la décrit au cerveau. Sans l'un ni l'autre, Waly
dit honnêtement qu'il ne voit pas — et suggère les modèles installés qui
voient.

**Nos promesses, rejouées.** Avant de publier, nous avons vérifié nos
propres garanties : six défauts trouvés et corrigés, six problèmes encore
ouverts, chacun avec la direction prévue et l'endroit où aider. À lire avant
de faire confiance :
[`docs/AUDIT-2026-10-02-promesses-rejouees.md`](docs/AUDIT-2026-10-02-promesses-rejouees.md).
Deux limites à connaître : le scellé vaut par programme (un autre programme
lancé par Waly n'est pas couvert), et un moteur Ollama reste hors du scellé.

**Ce que la Garde tient, et ce qu'elle ne tient pas.** Pour Waly, le registre
est exact et chaque coupure est immédiate. Pour un autre agent : la
surveillance est éteinte par défaut, demande l'accord de Windows, et montre
ce qui ressemble à tes fichiers, pas une trace exhaustive. L'enclos ferme ton
dossier personnel ; hors de ce dossier (un autre disque, `C:\projets`), un
agent de l'enclos lit et écrit tant que tu ne coupes pas. Il ne règle pas le
réseau. Un agent relancé à la main sort de l'enclos, et la Garde le signale.

**Rien ne sort sans ton accord.** Tout ce qui peut sortir est une sortie
que TU ouvres, une par une, visible tant qu'elle est ouverte et inscrite au
journal du scellé : un modèle extérieur avec ta clé, une passerelle de
messagerie, une conversation envoyée à un contact, un téléchargement de
modèle. Les processus de Waly restent scellés : ces sorties passent par un
programme séparé, vers la seule adresse déclarée. Un modèle extérieur ne
reçoit que le texte de la conversation affichée — jamais ta mémoire, tes
instructions, ton écran, tes images ni tes outils ; un fichier joint
seulement si tu l'autorises pour ce message.

**Choisir qui répond.** Le menu des modèles change le cerveau local en
route, ou choisit un modèle extérieur (Claude, Mistral, OpenAI, ton propre
serveur) avec **ta clé**, chiffrée par Windows. *Auto* route chaque message :
local d'abord, seul le travail de fond sur du texte sort, et chaque réponse
dit qui a répondu. La *réflexion approfondie* fait écrire au modèle local
son raisonnement dans un bloc repliable (plus lent ; un petit modèle peut
encore se tromper).

**Depuis ton téléphone, et vers un proche.** Une passerelle Telegram laisse
UN téléphone appairé (code à 8 chiffres) parler au modèle local. Et deux
Waly s'envoient une conversation, n'importe où dans le monde, sans compte :
identités X25519, `crypto_box` de NaCl, par un relais que tu héberges
(`crates/waly-relais`). Ce que tu reçois attend ton accord.

**Ensuite : l'appel entre deux utilisateurs de Waly.** Le but est simple :
deux personnes qui se parlent — à la voix d'abord, en vidéo ensuite — autour
d'une conversation qu'elles partagent, chacune avec son assistant local à
portée de main, et personne au milieu. Même principe que le partage : pas de
compte, chiffrement de bout en bout, un relais qui transporte ce qu'il ne
peut pas lire. Ce n'est **pas encore construit** ; aujourd'hui, seules les
conversations voyagent.

> Ces fonctions tournées vers l'extérieur sont neuves (octobre 2026) : leur
> chemin complet est couvert par des tests de bout en bout hors ligne, elles
> n'ont **pas encore été éprouvées contre les vrais services**.

Les missions rendent Waly meilleur : après une mission réussie, il distille
une **compétence apprise** (étapes, outils, pièges — jamais de données
personnelles) qu'il réapplique aux missions semblables. Panneau
*Compétences*.

## Feuille de route

Voir [ROADMAP.md](ROADMAP.md) — court terme : TTS français distillé,
`waly.toml` + tout backend OpenAI-compatible, client MCP local, mode anglais.

## Contribuer

Pas besoin de la machine de référence : l'essentiel de la logique est du
Rust testé (`cargo test -p waly-core -p waly-relais -p waly-seal` tourne
partout). Trois bonnes portes d'entrée : une
[première tâche](https://github.com/Michee-007/waly/issues?q=is%3Aissue+is%3Aopen+label%3A%22good+first+issue%22),
une mesure sur ta machine (le formulaire d'issue « Measurement on my
machine »), ou le **mode anglais**. Voir [CONTRIBUTING.md](CONTRIBUTING.md) ;
issues et PR en français ou en anglais.

## Licence

[MIT](LICENSE). Les modèles, voix et moteurs tiers gardent leur propre
licence — voir [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) (à noter :
les modèles de features du wake word sont aujourd'hui non commerciaux).
