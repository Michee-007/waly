# ADR 2026-10-06 — La Garde : voir ce que les programmes touchent

## Contexte

La Garde montre ce que Waly touche (registre exact, tenu par notre code) et
permet de couper ses accès. Pour les AUTRES agents de la machine, Waly ne
savait que leur couper internet ou les figer. Michée : « l'option qui permet
de voir ce que l'agent touche, c'est la bonne définition d'un système », puis :
« on doit avoir l'option de surveillance de tout ce qui se trouve sur la
machine, vraiment tout, et on peut aussi se limiter agent par agent ; éteinte
par défaut ».

## Décision

Le service `waly-seal` (SYSTEM) reçoit un **regard** : une session de suivi
d'événements du noyau Windows (ETW), en temps réel, sur trois sources —
fichiers, lancements de programmes, connexions.

1. **Éteinte par défaut**, et à chaque démarrage du service. Rien n'est
   observé tant que l'utilisateur ne l'a pas demandé.
2. **Deux portées** : « agents » (les programmes désignés, et tout ce qu'ils
   lancent, attribué à l'agent) ou « tout » (toute la machine).
3. **Allumer exige une élévation** (accord Windows), comme sceller un
   programme tiers (ADR 2026-09-16). Un programme ordinaire ne peut donc pas
   se servir du service pour espionner les autres. **Éteindre est permis à
   tous** : cela ne fait que réduire ce qui est exposé.
4. **Des chemins, jamais des contenus.** On retient le nom d'un fichier, un
   programme lancé, une adresse et un port. Aucun octet lu ou écrit.
5. **Tout reste local.** Le carnet vit dans la mémoire du service (4 000
   lignes au plus, regroupées) ; l'app le relève par le canal du service et
   l'inscrit dans sa base, au registre de la Garde.
6. **Le tri est pur et testé** (`tri.rs`) : le système, les bibliothèques,
   les caches et les dossiers parcourus sont écartés, sinon un programme qui
   démarre produit des centaines de lignes.

## Ce que cela ne fait pas

- **Voir n'est pas empêcher.** Couper un dossier à un autre agent demande de
  le lancer sous une identité à part (l'« enclos », étape suivante).
- **Le tri peut cacher un geste.** Un fichier d'un dossier écarté (un cache,
  `AppData\Local\Temp`) n'apparaît pas. Le fil montre ce qui ressemble aux
  fichiers de l'utilisateur, pas une trace exhaustive.
- **Les lectures ne sont pas distinguées des simples ouvertures.** « A
  ouvert » veut dire que le programme a ouvert le fichier ; « a écrit » et
  « a créé » sont, eux, des gestes constatés.
- **Les programmes de Waly sont écartés du regard** : son propre registre est
  exact, le doubler par l'observation ferait deux lignes par geste.

## Mesures (`lab/garde-banc`)

- Faisabilité (`logman`) : ~865 événements par seconde pour tout le système,
  3,3 Mo pour cinq secondes ; les quatre gestes d'un faux agent retrouvés.
- Lecteur en direct (`regard.rs`, console élevée) : quatre gestes sur quatre
  retrouvés, en portée « agents » comme en portée « tout » ; après tri, 18
  lignes pour un PowerShell qui démarre, contre 560 noms bruts.

## Essai sur le service installé (2026-10-06, nuit)

`apps/desktop/e2e/e2e-garde-surveillance.js`, sur la vraie app et le service
installé (SYSTEM) : « Surveiller toute la machine » allumé par le bouton
(accord Windows), un PowerShell fait quatre gestes, les quatre arrivent dans
le fil de la Garde (lecture de `README.md`, création et écriture d'un
fichier, lancement de `ping`, connexion à `1.1.1.1:443`), au milieu de ceux
d'une douzaine d'autres programmes ; extinction par le bouton, sans accord.

Deux défauts trouvés par cet essai et corrigés :

- **« Surveiller » échouait depuis l'app, jamais depuis une console.** Le
  programme élevé du service ne réessayait pas quand le canal était occupé,
  et la Garde ouverte l'interroge en continu. Il réessaie maintenant, comme
  l'app. Le même défaut pouvait faire échouer « Couper internet ».
- **Un geste refait dans la journée n'apparaissait plus.** Sa ligne n'était
  que redatée ; le fil, lu par numéro, la laissait enfouie sous les lignes
  plus récentes. Elle est maintenant retirée puis réinscrite.

Vu aussi : en portée « tout », le fil se remplit vite (plus de mille gestes
en quelques minutes sur une machine de développeur) et montre des chemins du
profil. Il reste local ; une capture de ce fil ne se publie pas telle quelle.

## Conséquences

- Le service doit être **réinstallé** pour recevoir le regard.
- `Requete::Regarder` et `Requete::Observations` s'ajoutent au protocole,
  gardées par `ipc::autoriser`.
- La surveillance de toute la machine est un pouvoir réel. Elle est annoncée
  par un dialogue, visible en permanence en tête de la Garde, et s'arrête
  d'un clic.
