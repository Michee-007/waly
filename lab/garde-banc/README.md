# Banc « Garde » — voir ce qu'un autre agent touche, lui fermer un dossier

Essais de faisabilité pour la Garde (jamais livrés). Verdicts mesurés sur la
machine de référence.

## Essai 1 (2026-10-06) : le suivi d'événements de Windows suffit-il ?

**Question.** Peut-on voir, de l'extérieur, les fichiers qu'un programme
ouvre ou écrit, les programmes qu'il lance et ses connexions, sans rien
installer dans ce programme ? Et à quel coût ?

**Méthode** (`essai-etw.ps1`, une demande UAC). Une session de suivi (ETW)
sur trois sources du noyau : `Microsoft-Windows-Kernel-File` (noms,
créations, écritures, suppressions, renommages), `Kernel-Process`
(lancements), `Kernel-Network` (IPv4 et IPv6). Un faux agent fait quatre
gestes connus ; on les cherche ensuite dans la trace.

**Verdict : faisable.** Les quatre gestes sont retrouvés.

| Geste du faux agent | Retrouvé |
|---|---|
| Lire `README.md` | oui (événement 12, avec le chemin complet) |
| Écrire un nouveau fichier | oui (événements 12, 30 « nouveau fichier », 10) |
| Lancer `ping.exe` | oui (lancement, avec le parent) |
| Se connecter à `1.1.1.1:443` | oui (événements 12 et 13, avec le PID) |

**Coût mesuré.** 5,1 s de capture sur TOUT le système : 4 377 événements,
soit ~865 par seconde, 3,3 Mo de trace. C'est peu : filtrer côté lecteur sur
les processus de l'agent est abordable, sans filtre côté noyau.

**Ce qu'il faudra traiter.**

- **Le bruit.** Le faux agent (un PowerShell) a produit 1 551 événements de
  fichiers et 560 noms distincts en 5 s, presque tous dus à son propre
  démarrage (bibliothèques, profil). Il faut écarter Windows, le dossier
  d'installation du programme et son cache, et regrouper par dossier.
- **Les chemins** arrivent sous forme de périphérique
  (`\Device\HarddiskVolume3\…`) : à traduire en lettre de lecteur.
- **Le réseau** : l'adresse est un entier (`16843009` = `1.1.1.1`) et le port
  est en ordre réseau (`47873` = 443 octets inversés).
- **Les écritures** (événement 16) portent un identifiant d'objet, pas un
  nom : il faut le relier à l'ouverture (événement 12).
- **Les droits.** Ouvrir la session exige un administrateur : c'est le rôle
  du service `waly-seal`, déjà en SYSTEM. L'app lirait le résultat par le
  canal du service.

**Ce que cet essai ne dit pas.** Il a été fait avec `logman` et relu après
coup ; un lecteur en direct dans le service reste à écrire. Et il montre ce
qu'un programme touche, il n'empêche rien : c'est l'objet de l'« enclos ».

## Essai 2 (2026-10-06) : le lecteur en direct du service

**Question.** Le lecteur écrit pour le service (`crates/waly-seal/src/regard.rs`)
retrouve-t-il les mêmes gestes, en direct, après tri ?

**Méthode** (`essai-regard.ps1`, une demande UAC). Le service bâti en version
d'essai est lancé dans une console élevée avec `essai-regard 9` : il écoute
neuf secondes et imprime son carnet. Deux passes : portée « agents » (le
faux agent désigné par son programme), puis portée « tout ».

**Verdict : quatre gestes sur quatre, dans les deux portées.**

| | Portée « agents » | Portée « tout » |
|---|---|---|
| Lire `README.md` | vu | vu |
| Créer et écrire un fichier | vu | vu |
| Lancer `ping.exe` | vu | vu |
| Se connecter à `1.1.1.1:443` | vu | vu |
| Lignes gardées en 9 s | 18 | 27 |

**Ce que le premier passage a appris** (corrigé avant le second) :

- le port tient sur deux octets : lu sur quatre, il valait toujours 0 ;
- des dossiers parcourus arrivaient comme des fichiers ouverts : on écarte
  d'après les options d'ouverture, puis, pour un nom sans extension, en
  regardant sur le disque ;
- le processus « System » vide les caches à la place des programmes : lui
  attribuer leurs écritures serait faux, on l'écarte pour les fichiers ;
- hors SYSTEM, le nom de beaucoup de processus est illisible (« processus
  39748 ») : en portée « tout », on l'apprend au lancement.

**Reste à voir** sur le service installé (en SYSTEM) : le comportement sur
un vrai agent (Ollama), et la tenue dans la durée.

## Essai 3 (2026-10-06) : un compte Windows à part ferme-t-il un dossier ?

**Question.** Pour empêcher un autre agent d'ouvrir un dossier (étape 4,
« l'enclos »), suffit-il de le lancer sous un compte Windows à part ? Et que
faut-il d'élevé ?

**Méthode** (`essai-enclos.ps1`, une demande UAC). La partie élevée crée un
compte d'essai, attend, puis le supprime avec son profil. Tout le reste est
fait **sans élévation**, comme le fera l'app : lancer sous ce compte, régler
les droits de dossiers, sonder. La sonde est un `cmd` lancé sous le compte,
qui tente de lister le dossier puis d'y créer un fichier.

**Verdict : oui, et une seule élévation suffit (créer le compte).**

| Question | Mesure |
|---|---|
| Créer le compte | 40 ms ; sans groupe, il ouvre quand même une session |
| Lancer sous le compte, sans élévation | oui (252 ms) ; l'enfant d'un enfant garde le compte |
| Profil de l'utilisateur, rien de réglé | **fermé** : le dossier, Documents, Bureau, `.ssh`, `AppData\Local\Programs` |
| Hors du profil, rien de réglé | **lit et écrit** `C:\waly\docs`, `C:\Users\Public` ; lit `C:\Program Files` |
| Dossier du profil : donner en lecture | lit, n'écrit pas ; sous-dossier compris (24 ms) |
| … donner en écriture, puis reprendre | lit et écrit ; puis fermé de nouveau |
| Dossier hors profil : couper, puis rendre | fermé (14 ms) ; puis lit et écrit de nouveau |
| Dossier de 100 entrées | coupé en 25 ms, rendu en 23 ms |
| Programme installé dans le profil | refusé (code 5) ; se lance une fois son dossier donné en lecture |
| Réseau depuis le compte | sort (`curl https://1.1.1.1` : code 0) |
| L'enclos lève-t-il sa propre coupure ? | non (`icacls` : accès refusé) |
| Ses processus, vus du compte ordinaire | nom, chemin et ligne de commande lisibles ; arrêt possible par le lanceur |

**Ce que le premier passage a appris.** Dans PowerShell 5.1, le code de sortie
d'un processus lancé avec `-Credential` revient vide : toutes les sondes
disaient « refus ». Corrigé en faisant écrire son code par la sonde dans un
fichier public.

## Essai 4 (2026-10-06) : le code livré (`waly_core::enclos`)

**Question.** Le module de l'app fait-il la même chose, et ses propres essais
disent-ils vrai ?

**Méthode** (`essai-enclos-code.ps1`). Le CLI `waly enclos`, sans élévation ;
une demande UAC pour créer le vrai compte `WalyEnclos` avec le programme du
service bâti (pas installé). Sortie : `resultat-enclos-code.txt`.

**Verdict.** Douze réglages enchaînés, tous confirmés par la sonde
(« TENU ») : lecture, écriture, coupure d'un sous-dossier d'un dossier donné,
lecture seule hors profil (l'écriture y est refusée), coupure puis écriture.
Trois refus attendus : tout le profil, la mémoire de Waly, un lecteur entier.
Un programme système puis un programme du profil lancés sous le compte,
suivis (2 processus : lui et sa console), arrêtés. Un réglage avec sa sonde :
~190 ms.

**Deux défauts trouvés par cet essai, corrigés.**

- « Reprendre » un dossier laissait un refus en place : `REVOKE_ACCESS`
  retire les permissions, pas les refus. Rendre un dossier coupé le laissait
  coupé. Le code retire maintenant lui-même les entrées du compte.
- Le nom d'un « job » Windows disparaît quand plus personne n'en tient de
  poignée, même si des processus y tournent : l'agent lancé n'était plus
  retrouvable. Une poignée est maintenant déposée dans l'agent lui-même.

**Reste à voir** sur l'app installée : un vrai agent (Ollama, Claude) mis dans
l'enclos depuis la Garde ; un agent à fenêtre ; un agent Node ou Python.

**État laissé sur la machine.** Le compte `WalyEnclos` existe, son secret est
dans la base de Waly, `C:\waly\data` lui est coupé. Pour le retirer :
`waly-seal-svc enclos supprimer` dans une console élevée.
