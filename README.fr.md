# Waly — l'intelligence qui reste

**Assistant personnel IA 100 % local pour Windows, à la vie privée prouvable.**
Voix française temps réel, vision (caméra « mode appel » + écran), mémoire
persistante, mot d'éveil — et un scellé réseau au niveau noyau qui fait de
« rien ne sort de ta machine » un *fait vérifiable*, pas une promesse.

*Il voit tout. Rien ne sort.* — [English version](README.md)

![Le panneau Vie privée : le réseau est scellé, une vraie tentative de sortie est bloquée par Windows et inscrite au journal, à côté de la seule sortie ouverte par l'utilisateur](docs/images/vie-privee.png)

*« Rien ne sort », vérifiable : le test intégré tente une vraie connexion
sortante depuis le processus de Waly — le noyau la bloque, le journal la
garde, à côté des sorties que tu as ouvertes toi-même.*

| La réflexion, montrée avant la réponse | Des modèles jugés pour TA machine |
|---|---|
| ![Un modèle local 4B écrit son raisonnement dans un bloc repliable, puis répond](docs/images/reflexion.png) | ![Catalogue de modèles avec les verdicts tient bien / lent / trop gros](docs/images/modeles.png) |

| Un appel vocal, sur ta machine | Un appel vidéo : Waly te voit, la caméra reste locale |
|---|---|
| ![L'écran d'appel : l'éclipse écoute, les sous-titres dessous, « Local · rien ne sort »](docs/images/appel-voix.png) | ![Appel vidéo : Waly remarque la présence et l'attention ; l'image de la caméra ne quitte jamais la machine](docs/images/appel-video.png) |

![Le panneau Partage : une conversation reçue attend ton accord ; ton code Waly est ta clé publique](docs/images/partage.png)

*Une conversation envoyée par un contact, chiffrée de bout en bout, en
attente de ton accord. L'appel entre deux utilisateurs de Waly arrive ensuite
(voir plus bas).*

> ⚠️ **Statut : pré-alpha, machine de référence uniquement.** Waly est
> développé et mesuré sur une seule machine (AMD Ryzen AI 5 340 — NPU XDNA 2,
> iGPU Radeon 840M, 15 Go de RAM utile, Windows 11). Il y tourne de bout en
> bout aujourd'hui. La portabilité (autres NPU, GPU seul, plus de RAM) est le
> prochain front — les mesures et retours d'autres machines sont bienvenus.

## Pourquoi encore un assistant local ?

1. **Vie privée prouvable (« huis clos »)** : un scellé WFP (Windows
   Filtering Platform), posé par un service SYSTEM, bloque toute sortie
   réseau des processus de Waly au niveau noyau, garde le loopback (serveur
   de modèles) vivant, et journalise chaque tentative bloquée. Le sceau se
   *teste* depuis l'app.
2. **Un assistant intégré, pas un chatbot** : cascade voix streaming avec
   barge-in, mode appel caméra avec mémoire visuelle, partage d'écran
   OCR-first, mot d'éveil, mémoire SQLite + embeddings locaux, outils
   supervisés avec approbations humaines — une seule app installable.
3. **Français d'abord** : voix, prosodie, prompts et produit pensés en
   français (anglais prévu).

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
- `crates/waly-seal/` — le scellé réseau : filtres WFP par session, service
  Windows, journal d'audit, fail-closed
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

## Licence

[MIT](LICENSE). Les modèles, voix et moteurs tiers gardent leur propre
licence — voir [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) (à noter :
les modèles de features du wake word sont aujourd'hui non commerciaux).
