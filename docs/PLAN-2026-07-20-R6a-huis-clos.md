# PLAN R6a — « Huis clos » : le scellé réseau prouvable par session (2026-07-20)

> Brief : RFC 2026-07-20 §I4 (roadmap v3) et §I6.2. Pilier « Preuve » de
> l'identité « l'intelligence qui reste » : *« Il voit tout. Rien ne sort. »*
> Critère de sortie RFC : **démonstration session scellée + tentative de sortie
> réseau bloquée ET journalisée + verdict d'un professionnel du secret.**

## Ce qu'on construit

Quand Michée scelle une session (le geste « Sceau »), Waly **bloque
techniquement** — au niveau du noyau Windows, pas par promesse de code —
toute sortie réseau de ses propres processus. L'éclipse le montre (état
« Sceau » : l'anneau continu fermé, planche d'identité du 20/07). Toute
tentative de sortie pendant le scellé est bloquée **et** inscrite dans un
journal d'audit local consultable (« le 20/07 à 21:14, waly-voice.exe a été
empêché de joindre 142.250.74.36:443 »). Le journal des tentatives EST la
preuve : un cabinet peut l'ouvrir et constater.

**Mécanisme choisi : Windows Filtering Platform (WFP)** — le moteur de filtrage
du noyau, celui du pare-feu Windows lui-même. API user-mode dans
`fwpuclnt.dll` (DLL système signée Microsoft → SAC-safe, piège n°3 : jamais
d'exe tiers). Accès depuis Rust windows-gnu via `windows-sys`
(`Win32_NetworkManagement_WindowsFilteringPlatform`) : link d'import d'une DLL
système, aucun build-script C++.

## Architecture du scellé

- **Session WFP DYNAMIQUE** (`FWPM_SESSION_FLAG_DYNAMIC`) : les filtres vivent
  tant que le processus scelleur vit, et sont détruits par le noyau à sa mort.
  C'est un CHOIX et pas un pis-aller : le sceau est *par session* (sceller =
  ouvrir la session WFP, dessceller = la fermer), et un crash de Waly ne laisse
  jamais de filtres orphelins casser le réseau de la machine. Le journal
  d'audit enregistre pose et levée : un trou dans la couverture se VOIT.
- **Sous-couche (sublayer) dédiée Waly**, poids fort. Par exe scellé (app-id
  WFP = chemin NT via `FwpmGetAppIdFromFileName0`), aux couches
  `FWPM_LAYER_ALE_AUTH_CONNECT_V4/V6` (toute connexion sortante TCP + premier
  envoi UDP) :
  1. **PERMIT poids 15** : app-id == exe scellé ET
     `FWP_CONDITION_FLAG_IS_LOOPBACK` — la machinerie interne (voix↔FLM↔
     desktop) continue de vivre ;
  2. **BLOCK poids 0** : app-id == exe scellé — tout le reste.
  Le BLOCK d'une sous-couche n'est pas annulable par le PERMIT d'une autre :
  le pare-feu Windows ne peut pas ré-ouvrir ce qu'on ferme.
- **Journal d'audit** : `FWPM_ENGINE_COLLECT_NET_EVENTS` + énumération/
  souscription des événements `CLASSIFY_DROP` filtrés sur nos app-ids →
  table SQLite locale `audit_sceau` (horodatage, exe, adresse:port, protocole,
  scellé posé/levé). Consultable dans l'app (et exportable pour le one-pager
  conformité). Aucune adresse n'est résolue en ligne (évidemment).

## Périmètre des processus (GATE 2 — inventaire du code, 2026-07-20)

| Processus | Rôle | Flux loopback requis | Scellé ? |
|---|---|---|---|
| `waly.exe` (installé : `%LOCALAPPDATA%\Programs\Waly\waly.exe` ; dev : `C:\waly\bin\waly.exe`) | desktop, waly-core in-process | → FLM 52626 ; écoute 52710 (service d'appel) | **OUI** |
| `waly-voice.exe` (spawné par l'appel/écran) | cascade voix | → FLM 52626 ; → desktop 52710 (`/cliche`, `/etat`, `/pouls`) | **OUI** |
| `flm.exe` (parent + worker, même exe) | le cerveau — c'est LUI qui tient la conversation | écoute 52626 (et 52625 ASR) | **OUI** — sceller l'app sans sceller le moteur serait du théâtre |
| `ollama.exe` (fallback iGPU) | moteur B | écoute 11434 | OUI si présent (app-id ajouté au sceau, inoffensif s'il ne tourne pas) |
| `msedgewebview2.exe` | rendu UI Tauri | aucun (UI = fichiers locaux) | **NON par WFP** — exe PARTAGÉ avec d'autres apps (un filtre app-id les casserait toutes). Traitement au spawn : `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS` = proxy mort (`--proxy-server=127.0.0.1:1`) + `--host-resolver-rules="MAP * ~NOTFOUND"` pendant le scellé — le navigateur ne peut plus ni résoudre ni sortir. Honnêteté du one-pager : cette part-là est « au spawn », pas noyau. |

Remarques de périmètre :
- Les app-ids couvrent **les chemins d'exe**, pas des PID (WFP ne filtre pas
  par PID) — pour nos exes c'est équivalent (une seule instance de chaque).
- DNS sortant = une connexion sortante comme une autre → bloqué par le BLOCK.
- Les outils opt-in (météo, web) échouent en session scellée : c'est le
  comportement VOULU ; le message d'erreur outil doit dire « session scellée »
  (chantier 2), et la tentative apparaît au journal.

## Les 3 GATES au banc — AVANT tout code produit

Banc : `lab/huisclos-banc` (crate autonome hors workspace, comme
`stt1b-banc`), buildé depuis WSL en windows-gnu, exécuté sous SAC.

- **GATE 1 — WFP depuis Rust windows-gnu sous SAC, et le modèle de droits.**
  Le banc ouvre le moteur, pose la paire PERMIT-loopback/BLOCK sur son propre
  app-id, sonde (loopback OK ? 1.1.1.1:443 bloqué ?), lit les net events.
  Question centrale : **que peut un utilisateur STANDARD ?** Hypothèse
  documentée : `FwpmFilterAdd0` exige un accès accordé par défaut aux seuls
  Administrators → mesurer précisément ce qui échoue (codes FWP_E_*), puis
  trancher entre les voies « sans admin permanent » :
  (a) **ACL one-time à l'installation** : un `FwpmEngineSetSecurityInfo0`
  élévé UNE FOIS accorde à l'utilisateur le droit d'ajouter des filtres —
  ensuite le scellé se pose sans UAC, à vie ; (b) service élevé installé
  une fois (plus de surface, dernier recours) ; (c) UAC à chaque scellement
  (fallback UX acceptable mais frottant). L'alternative AppContainer sans
  capacité réseau est DISQUALIFIÉE d'avance : elle bloque aussi le loopback
  (l'exemption `checknetisolation` exige admin) → casserait voix↔FLM.
- **GATE 2 — périmètre** : inventaire ci-dessus, à confirmer au banc (poser le
  sceau sur flm.exe pendant qu'il sert : la conversation loopback survit-elle
  intacte ?).
- **GATE 3 — coût/latence** : temps de pose/levée du sceau (cible < 100 ms,
  geste instantané à l'œil), surcoût par connexion loopback voix↔FLM (cible :
  indistinguable — WFP classifie déjà chaque connexion de la machine), RAM
  (cible ~0 : pas de processus neuf, pas de modèle).

## VERDICT DES GATES — VERTS (2026-07-20, banc `lab/huisclos-banc`)

Mesuré, log brut `gate-eleve.log`, détail dans le README du banc :
- **GATE 1** : exe *debug* passe SAC et pilote `fwpuclnt.dll`. Droits mesurés —
  utilisateur **standard** : `FwpmEngineOpen0` OK mais `FwpmSubLayerAdd0` =
  `ACCESS_DENIED 0x5` (ne peut PAS sceller) ; **élevé** : chaîne complète OK.
  → **le scellé exige une élévation**.
- **GATE 2** : sous sceau, loopback `127.0.0.1` **CONNECTE 0,2 ms**,
  `1.1.1.1:443`/`9.9.9.9:443` **`os error 10013`** (bloqués). Le contrat
  loopback survit, tout le reste tombe. (UDP dropé+journalisé mais `send_to`
  renvoie OK localement → sonder l'UDP par le journal.)
- **GATE 3** : pose **5,4 ms**, levée **0,5 ms**, médiane loopback **0,10 ms ==
  baseline** (surcoût nul), **0** process/modèle résident.
- **Journal** : `FwpmNetEventEnum0` sur nos app-ids → `CLASSIFY_DROP` horodaté
  (exe, adresse:port, protocole). C'est la table `audit_sceau`.
- **Hygiène** : session dynamique → `FwpmEngineClose0` détruit tout ; aucun
  filtre orphelin après levée (connectivité restaurée, vérifié).

## DÉCISION D'ARCHITECTURE — voie d'élévation : SERVICE ÉLEVÉ (corrigée le 2026-07-20)

Le scellé exige une élévation (GATE 1). Trois voies pour éviter un UAC à chaque
sceau. **Reco initiale = ACL one-time ; CORRIGÉE après challenge de Michée
(« ce qui est meilleur, robuste et pas vulnérable, pas ce qui est facile »).**
Le bon critère pour un *sceau* n'est pas le nombre de clics, c'est **qui peut le
manipuler et un compte compromis peut-il le percer en silence** :

- **(a) ACL one-time — DISQUALIFIÉE pour la robustesse.** Accorder au SID
  utilisateur le droit de poser des filtres délègue *à vie* une capacité
  privilégiée au token le plus exposé (le cas courant de compromission est du
  code malveillant **user-level**, pas SYSTEM). Ce code pourrait alors ajouter
  un PERMIT de poids supérieur dans la sous-couche Waly et **percer un trou
  silencieux** : l'UI affiche « Sceau » mais ça fuit. Un sceau contournable
  par un attaquant user-level pendant qu'il s'affiche fermé n'est pas une
  preuve. Optimisait la facilité UX, au prix de la garantie.
- **(b) SERVICE ÉLEVÉ — RETENUE.** La capacité privilégiée **reste dans
  SYSTEM**, jamais dans le compte utilisateur. Filtres **possédés par SYSTEM**
  → un attaquant user-level ne peut ni les retirer ni les percer :
  l'intégrité du sceau est protégée par la frontière de privilège de l'OS.
  IPC app→service **étroit et non-générique** : commandes `sceller`/`desceller`
  *ma session* sur des chemins d'exe connus, **jamais un filtre brut du
  client** → l'app ne peut pas demander un trou. Permet le **fail-closed** en
  sûreté (service mort = filtres persistent, rien ne fuit, seul SYSTEM nettoie
  au redémarrage — vs session dynamique app-liée où un crash retire les
  filtres = fuite ; fail-closed est le bon défaut pour le secret). « Plus
  complexe » ≠ « plus vulnérable » : moins de surface exploitable car rien de
  privilégié ne descend dans le compte utilisateur. Coût principal = **service
  signé** — prérequis distribution déjà acté le 20/07, les décisions
  convergent.
- **(c) UAC par sceau — FALLBACK.** Codé pour qui n'installe pas le service
  (self-hosters en mode léger) : aucune install privilégiée, mais un prompt
  admin à chaque geste (frottement pour une présence).

⚠ Bascule à entériner par Michée avant d'écrire le service (revirement de la
reco initiale). Une fois entérinée, le chantier 0 vise le service, pas l'ACL.

## Chantiers (après gates verts)

0. **Le service scelleur + le client** (voie retenue) :
   - **`waly-seal-svc.exe`** (nouveau binaire, service Windows SYSTEM, signé) :
     détient le moteur WFP, expose un IPC **étroit et non-générique** (named
     pipe ACLé, verbes `sceller {session, exes[]}` / `desceller {session}` /
     `journal {session}` uniquement — jamais de définition de filtre venue du
     client). Filtres **possédés par SYSTEM**, sous-couche Waly persistante,
     **fail-closed** (survie des filtres au crash, nettoyage au redémarrage).
     Le service surveille les PID scellés et desscelle à leur mort (garde le
     no-orphan sans session dynamique app-liée).
   - **`waly-core` client** (`sceau.rs`) : parle au pipe (`poser`/`lever`/
     `tentatives`), écrit `audit_sceau` en SQLite. **Fallback** intégré : si le
     service est absent, voie (c) UAC-par-sceau (relance élevée ponctuelle).
   - Réutiliser tout le code FFI WFP **validé au banc** (`lab/huisclos-banc`).
   - Tests logiques côté WSL (mock IPC), validation binaire + service côté
     Windows. Prototype à mesurer d'abord : install/désinstall du service,
     latence IPC (cible : sceau perçu instantané), et **sceller les VRAIS
     process** (flm + voix en appel) pour confirmer que le tour voix loopback
     survit intact (intégration).
1. **Desktop : le geste** — bouton « Sceller » par session (frère de ◐ Appel et
   ▣ Écran), état visible : éclipse « Sceau » (anneau continu fermé — étendre
   `eclipse` sans rouvrir « Marée », monochrome gravé), badge dans la barre de
   session, WebView2 re-spawné bras morts pendant le scellé.
2. **Refus parlants** : outils réseau opt-in → erreur « session scellée » (et
   ligne au journal) ; la voix le dit avec naturel.
3. **Journal d'audit consultable** : vue dans l'app (par session, exportable
   texte) ; pose/levée du sceau journalisées avec horodatage.
4. **One-pager « conformité par l'architecture »** (CNB/HDS/AI Act) : ce que le
   sceau garantit (noyau), ce qu'il ne garantit pas (WebView2 au spawn,
   processus hors périmètre), comment le vérifier soi-même. Honnêteté = la
   marque.

## Ce que le sceau ne prétend PAS (gravé pour le one-pager)

- Il scelle **les processus de Waly**, pas la machine (le navigateur de
  l'utilisateur, le cloud d'un autre logiciel continuent). C'est le contrat :
  « ce qui entre dans la session ne sort pas par Waly ».
- Un admin local peut évidemment retirer les filtres — le journal, lui, en
  garderait la trace de pose/levée (attestation locale, cf. RFC §I6 ; la
  signature du journal viendra avec la signature de code).

## Critère de sortie

Démonstration filmable : session scellée (éclipse « Sceau »), un outil web
tente de sortir → refus parlant + ligne au journal d'audit datée avec
l'adresse bloquée, pendant que la voix continue de converser (loopback vivant).
Puis verdict d'un professionnel du secret (test de vérité (a) du RFC §I3).
