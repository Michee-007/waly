# ADR 2026-10-01 — Lot 3 des « bientôt » : réflexion, modèles, sorties ouvertes

## Contexte

Lot 3 décidé par Michée le 29/09 : réflexion approfondie, téléchargement de
modèles, modèles extérieurs avec la clé de l'utilisateur + routeur,
messageries + partage. Trois de ces quatre points font SORTIR quelque chose de
la machine, alors que Waly est scellé par défaut (ADR 2026-07-21) et que l'ADR
du 14/09 a posé les règles : « rien ne sort sans ton accord », un modèle
extérieur ne reçoit ni mémoire ni fichiers ni écran, le routeur choisit un
modèle mais pas ce qui sort.

## Décisions

### 1. Réflexion approfondie = réflexion DEMANDÉE, séparée au fil de l'eau

Le cerveau par défaut (qwen3 instruct) ne raisonne pas nativement
(`reasoning_effort` ignoré — banc du jour). La réflexion est donc demandée par
une consigne en queue du message : le modèle écrit entre
`<reflexion>…</reflexion>` puis répond ; `waly_core::reflexion::Filtre` sépare
le flux (balises coupées entre deux deltas comprises). Un modèle qui raisonne
nativement passe par le même filtre (`<think>` brut, ou champ `reasoning` du
serveur ré-emballé par `LlmClient::reflexion`).

- Un seul appel, mêmes outils, consigne dans le message : ni le système ni le
  bloc d'outils ne bougent (append-only R4.5). Après le tour, la fenêtre ne
  garde ni la consigne ni la réflexion (budget 8k ; et un modèle qui les
  relirait les imiterait une fois l'option coupée) — mutation en queue,
  re-préfill court sur Ollama, plein tarif sur un cache une-case type FLM.
- Conversation écrite seulement : ni voix, ni mission, ni écran, ni image.
- La réflexion affichée est gardée (`session_reflexions`) et revient, repliée,
  à la réouverture. Si le modèle réfléchit sans conclure, une relance courte
  sans outils produit la réponse.
- Le délai de 120 s d'un flux devient un délai d'INACTIVITÉ : une réponse
  longue qui coule n'est pas une panne.
- Honnêteté : un 4B qui réfléchit reste un 4B. L'UI le dit (« un petit modèle
  peut encore se tromper »).

### 2. Télécharger un modèle = le DEMANDER au moteur local

Waly ne télécharge rien : il demande à Ollama (`POST /api/pull`, loopback).
Ollama est hors du périmètre scellé ; c'est lui qui sort vers son registre.
La sortie est visible pendant qu'elle dure et inscrite au journal du sceau.
Catalogue court, noms et tailles relus à la source le jour même ; verdict
« tient bien / lent / trop gros » = règle de mémoire calée sur la machine de
référence (`modeles::verdict`) ; champ libre pour tout autre nom Ollama.

### 3. Modèles extérieurs : passerelle séparée, clé au coffre, filtre dans le code

- **Waly reste scellé.** L'appel sort par un processus passerelle séparé : le
  `curl` du système (signé, TLS du système, chemin absolu `System32`), hors du
  périmètre, vers la seule adresse déclarée. Précédent : les connecteurs MCP
  (ADR 2026-09-10), eux aussi « hors du scellé » et déclarés par l'utilisateur.
  Aucune dépendance TLS ajoutée, aucun binaire neuf à signer.
- **La clé** est chiffrée par Windows pour le compte de l'utilisateur (DPAPI,
  `crypt32.dll`) avant d'entrer en base ; elle ne voyage que par l'entrée
  standard de la passerelle (jamais en argument, jamais sur disque en clair,
  jamais dans un journal ni une erreur). Adresse, modèle et clé sont validés
  par jeu de caractères : rien ne peut s'échapper de la configuration.
- **Ce qui sort** (`exterieur::conversation_sortante`) : le texte de la
  conversation affichée, rien d'autre. Pas de système local (mémoire, projet,
  instructions, journal visuel, nom de l'utilisateur), pas d'outils, pas
  d'image. Les fichiers joints sont retirés (seul leur nom reste) sauf accord
  explicite pour CE message. Missions, écran et images restent locaux.
- **Protocoles** : API Messages native pour Claude (`x-api-key`, pas de
  température, réflexion adaptative par défaut, repli serveur sur refus) ;
  format OpenAI pour Mistral, OpenAI et un serveur personnel (`http://` admis
  seulement vers la machine ou le réseau local).
- **Routeur « Auto »** (`exterieur::router`, pur et testé) : local d'abord ;
  seul le travail de fond sur du texte sort ; tout ce qui touche aux outils,
  à la mémoire, aux fichiers, à l'écran, aux images ou aux missions reste
  local. La raison est affichée sous chaque réponse.
- **Visible** : sous chaque réponse (« via X · sorti vers hôte »), sur le
  bouton du modèle, au pied de la barre latérale (« Scellé · sortie ouverte :
  hôte ») et au journal du sceau (genre `sortie`, une ligne par tour).
- Le cerveau LOCAL se change en route depuis le même menu (`Cmd::Modele`) ;
  le choix persiste et vaut pour la voix.

### 4. Passerelle de messagerie : Telegram d'abord, un seul interlocuteur

- Relève et envoi par la même passerelle séparée ; jeton chiffré comme une clé.
- **Appairage par code** (8 chiffres, affiché dans l'app) depuis une
  conversation PRIVÉE ; après 5 codes faux l'appairage se ferme. Une fois
  appairée, la passerelle ne répond qu'à cet interlocuteur ; inconnus et
  groupes sont ignorés sans réponse, et inscrits au journal.
- Le modèle LOCAL répond, dans une conversation dédiée ; les actions qui
  demandent un accord restent en attente dans l'app, jamais approuvées à
  distance.
- Honnêteté dans l'UI : un message Telegram transite par les serveurs de
  Telegram ; la passerelle vit tant que l'app est ouverte.

## Ce qui n'est PAS fait (et pourquoi)

- **Sceller la passerelle PAR DESTINATION** (filtres WFP adresse/port) : c'est
  le vrai socle annoncé par l'ADR du 14/09. Il demande une mise à jour du
  service élevé (installation par Michée, UAC). Aujourd'hui la passerelle est
  un programme système non scellé qui ne va QUE là où le code l'envoie.
- **Matrix, Signal, WhatsApp, Discord/Slack** : restent « bientôt ». Matrix
  est le suivant naturel (même moteur relève/envoi), à faire contre un vrai
  serveur.
- **Appel entre deux Waly** : reste « bientôt » (audio temps réel à travers
  un relais : un chantier à part). Le PARTAGE d'une conversation est fait —
  voir l'addendum.
- **Aides en parallèle** : non tranché (hors lot).
- **Vérification contre les vrais services** : Claude, Mistral, OpenAI et
  Telegram n'ont PAS été appelés (aucune clé, aucun jeton n'a été saisi). Le
  chemin complet est prouvé contre des serveurs locaux (Ollama en « serveur
  personnel », faux service Telegram) ; les formats réels sont couverts par des
  tests unitaires écrits d'après la documentation. Premier usage réel = terrain
  de Michée.
- Coffre de clés : Windows seulement (DPAPI) ; Linux à faire.

## Conséquences

- `audit_sceau` accepte le genre `sortie` (table rebâtie à l'ouverture d'une
  ancienne base) : le journal du sceau devient le registre des sorties
  bloquées ET ouvertes.
- « Local · rien ne sort » n'est plus affiché quand une sortie est ouverte.
- Trois e2e neufs (`apps/desktop/e2e/e2e-{reflexion,exterieur,passerelle}.js`),
  tous jouables hors ligne.

## Addendum (même jour) — partage d'une conversation entre deux Waly

**Décision de Michée** : pas de « réseau local seulement » — un contact peut
être à l'autre bout du monde. Donc un **relais**, comme l'annonçait l'ADR du
14/09 (« relais de mise en relation, auto-hébergeable, une sortie de plus »).

- **Relais = boîtes d'enveloppes chiffrées** (`crates/waly-relais`, Rust pur,
  bibliothèque standard, tout en mémoire). Les deux Waly ne font que des
  connexions SORTANTES vers lui : aucun port entrant, aucune règle de
  pare-feu. On dépose toujours sur le relais du destinataire.
- **Identité sans compte** : une paire de clés X25519 par installation. Le
  « code Waly » donné à un contact = clé publique + boîte (128 bits au hasard)
  + adresse du relais, avec un contrôle contre les fautes de recopie.
- **Chiffrement** : `crypto_box` de NaCl (X25519 + XSalsa20-Poly1305, crate
  RustCrypto, Rust pur), authentifié dans les deux sens. Rien d'inventé. Le
  relais ne voit que du chiffré ; une enveloppe qui ne vient pas d'un contact
  connu est jetée avant tout déchiffrement ; une enveloppe modifiée ou forgée
  échoue au déchiffrement.
- **Le destinataire décide** : une conversation reçue attend son accord.
- **Boîte** : ouverte à la première relève, qui lui attache un jeton ; seul ce
  jeton la relève ensuite. Clé secrète et jeton sont chiffrés par Windows.
- Waly reste scellé (dépôt et relève par la passerelle séparée) ; envoi,
  réception et enveloppes jetées s'inscrivent au journal du sceau.

**Limites assumées** : pas de confidentialité persistante (v1, clés
statiques) ; le relais voit les métadonnées (quelle boîte, quand, quelle
taille, quelle clé publique expéditrice) ; un relais hostile peut retenir ou
effacer, jamais lire ni falsifier ; la conversation part « telle qu'affichée »
(fichiers joints compris, l'utilisateur le confirme). **Il n'existe pas de
relais public** : pour un usage réel il faut en héberger un
(`crates/waly-relais/README.md`) — décision et geste de Michée.

Vérifié de bout en bout avec deux instances de l'app et un relais local
(`e2e-partage.js`, 17 vérifications) ; jamais à travers internet.
