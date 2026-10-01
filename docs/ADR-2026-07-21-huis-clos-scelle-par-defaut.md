# ADR — Huis clos scellé PAR DÉFAUT, sans bouton (silencieux comme Claude)

Date : 2026-07-21 · Statut : accepté · Contexte : R6a « Huis clos »
Décision de Michée (mandat produit). Révise le chantier 1 de R6a et la
directive UX « Sceau » du RFC 2026-07-20 §I5.

## Question
Faut-il un geste « Sceller » (l'utilisateur scelle une session, un indicateur
« Sceau » l'affiche) ou le scellé doit-il être l'état PAR DÉFAUT, permanent et
non affiché ?

## Décision
**Scellé par défaut, permanent, silencieux.** Le service `WalySeal` scelle le
périmètre de Waly dès son démarrage (et au boot). **Pas de bouton, pas de
notion de « session scellée », aucun indicateur d'état normal** (ni anneau
« Sceau » sur l'éclipse, ni liseré, ni badge). La connexion à l'extérieur
sera, plus tard, une CAPACITÉ qu'on ACTIVE explicitement (recherche web, BYOK
R8), bornée et journalisée — jamais l'inverse.

## Raisons, par ordre de dureté

1. **Un bouton contredit l'identité.** La devise est « Il voit tout. *Rien ne
   sort*. » Un bouton « Sceller » admet implicitement que *par défaut, des
   choses peuvent sortir* — l'inversion de charge exacte qui a tué Recall
   (« désactivable »). Le défaut prouvable (« Waly ne PEUT pas sortir, point »)
   est infiniment plus fort que « Waly peut sceller si tu y penses ».

2. **Le modèle Claude.** Claude n'affiche jamais « je suis sécurisé » — il
   l'est, silencieusement, parce que l'infrastructure le garantit. Un
   indicateur permanent de sécurité trahit un doute au lieu d'inspirer
   confiance (comme un antivirus qui clignote). Le silence EST le message.
   L'accès extérieur devient l'exception explicite et visible-pendant-qu'active
   (comme « Claude fait une recherche web ») — pas la règle.

3. **Coût fonctionnel nul aujourd'hui.** Waly n'a AUCUNE fonctionnalité réseau
   livrée (météo/web = « opt-in » notées depuis R2, jamais implémentées). Le
   loopback (voix↔FLM↔desktop) est préservé par construction. Sceller par
   défaut ne casse rien : on interdisait par un bouton ce que le produit ne
   fait pas.

4. **Fail-closed intégral.** Le scellé ne dépend plus de l'app : service
   planté, machine redémarrée, app fermée — le périmètre reste clos. La démo
   avion devient meilleure : *il n'y a rien à cliquer*.

## Conséquences

- Le service scelle un **périmètre** (id réservé 0) à son démarrage, avec les
  chemins connus (moteurs) ; l'app **rejoint** le périmètre en donnant son
  propre chemin d'exe au lancement (IPC add-only, validée `waly*.exe`/`flm.exe`
  — on ne peut pas faire sceller un exe tiers, ni **desceller** le périmètre
  via le pipe : `Desceller` refuse l'id 0, sinon un malware user-level
  couperait le sceau).
- **Bouton « Sceller » supprimé**, styles d'état scellé (anneau, liseré,
  bascule de statut) supprimés. L'éclipse « Sceau » du RFC §I5 est **retirée**
  (l'« Éveil » du wake word R6b demeure).
- **« Journal du sceau » et « Tester le sceau » restent** — la preuve
  *montrable à la demande*, pas affichée en continu.
- **Honnêteté sur l'échec** : si le service est absent/non privilégié, le sceau
  n'est pas tenu — on ne peut pas taire le mensonge « rien ne sort ». Une
  correction DISCRÈTE du texte de statut existant (pas un nouvel indicateur
  anxiogène) le dit, avec l'invite à réparer. Quand tout va bien : rien.
- **Le service devient obligatoire** au setup (élévation unique) → dette
  d'installeur (bundler `waly-seal-svc.exe`) promue au cœur de R6a.

## Contrainte future (R8 BYOK / recherche web)
« Ouvrir » ne devra jamais signifier « tout ouvrir » : ouverture PAR CAPACITÉ,
bornée dans le temps, visible pendant qu'active, journalisée (« fenêtre
ouverte 14:02→14:03, web »). C'est le pendant exact du scellé par défaut.
