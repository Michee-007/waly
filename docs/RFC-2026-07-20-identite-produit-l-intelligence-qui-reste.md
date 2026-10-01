# RFC — Identité produit : « Waly, l'intelligence qui reste » (2026-07-20)

> Mandat : le 2026-07-20, Michée a délégué la direction produit (« la direction du
> produit t'appartient ») avec pour critère : une identité pertinente et
> révolutionnaire, à la pointe, alliant meilleure expérience imaginable, puissance
> et utilité — et l'exigence de ne PAS suivre les directions déjà prises par tous
> ni valider de façon biaisée ce qui semble populaire.
> Ce RFC remplace D7 du RFC 2026-07-03 (positionnement) et révise sa feuille de
> route v2 (R6/R7). Tout le reste du RFC 2026-07-03 (D1-D6) demeure.

## I1 — Le constat qui fonde l'identité (preuves, pas intuitions)

Recherche du 2026-07-20 (sources en annexe) :

- **Les gadgets compagnons sont morts** (Humane bradé, Rabbit 95 % d'abandon à
  5 mois, pendentif Friend ridiculisé). Leçon des autopsies : un produit IA doit
  répondre à « qu'est-ce que ça fait que l'appareil existant + le cloud ne fait
  pas mieux ? ». Waly est un logiciel sur le PC existant ✓ — et sa réponse de
  fond tient en trois propriétés que Copilot ne peut PAS copier :
  **rien ne sort, ça marche sans réseau, c'est vérifiable (open source)**.
- **La mémoire d'écran est un puits empoisonné** : Recall un an après = failles
  en série, « presque personne ne l'utilisera » (presse spécialisée), Rewind a
  pivoté. → R7-mémoire-ambiante (style screenpipe) est SUPPRIMÉE de la roadmap.
  Le choix inverse de Waly (voit tout, NE GARDE RIEN) devient un pilier
  d'identité, pas une limitation.
- **Le seul local devenu viral en 2026 : OpenClaw** (60k stars/72 h). Pourquoi :
  local-first assumé, l'IA qui vient à toi, qui AGIT. Deux leçons transférées
  (présence sans clic → wake word ; agir → workflows R-mains), une refusée
  (multi-messageries cloud = la vague déjà passée, CVE et 30k instances exposées).
- **Le marché « compagnon émotionnel » est réel (50 M utilisateurs) mais
  toxique pour nous** : mobile, parasocial, recherche académique négative sur
  l'usage long. → « le compagnon qui te voit » est retiré comme bannière
  publique. La TECHNOLOGIE de présence (éclipse, moments, appel) reste l'âme
  du produit ; elle n'est plus le slogan.
- **La demande solvable et datée est réglementaire, française, locale** :
  guide déontologique CNB mars 2026 (secret professionnel vs IA cloud),
  art. 226-13 Code pénal, HDS pour les médecins, AI Act pleinement applicable
  août 2026. TOUTE l'offre en face est cloud ou rustine (pseudonymisation) ;
  Mistral ne fait du on-premise qu'en datacenter Enterprise. Le poste de
  travail 100 % local est un créneau VIDE avec un vent réglementaire.

## I2 — L'identité

**Nom d'identité : « Waly — l'intelligence qui reste. »**
Double sens gravé : *reste chez toi* (rien ne sort de la machine, jamais) et
*reste avec toi* (une présence qui te connaît, se souvient, t'accompagne —
pas un onglet qu'on ouvre).

**Devise de preuve : « Il voit tout. Rien ne sort. »**
Anti-Recall en cinq mots. Chaque mot est démontrable dans le code (zéro pixel
persisté, zéro OCR brut en base, réseau scellable, sources ouvertes).

**Positionnement en une phrase** : Waly n'est pas un assistant (course perdue
d'avance contre Copilot, gratuit et préinstallé) — c'est **la première présence
locale prouvable** : le premier logiciel avec lequel on peut être en appel —
il te voit, voit ton écran, parle français — **en mode avion**.

**Les trois piliers** (tout arbitrage produit se juge contre eux) :
1. **Présence** — l'Appel, l'éclipse « Marée », les moments proactifs, la
   mémoire vivante, l'overlay flottant, l'éveil à la voix (wake word, à venir).
   Une présence ne se lance pas à la souris.
2. **Preuve** — pas des promesses de confidentialité : des dispositifs
   montrables. Zéro persistance visuelle (livré), « Huis clos » (réseau scellé
   par session, à venir), journal d'audit local, démo avion, open-core.
3. **Langue** — français d'abord (voix, STT, culture du produit) ; l'Europe
   comme marché naturel (RGPD/AI Act = vent porteur, pas contrainte).

**L'expérience définissante** reste l'Appel (R4), étendue d'un geste nouveau :
**le Sceau** — l'utilisateur scelle la session (Huis clos), l'indicateur le
montre, et ce qui se passe dans la session ne peut techniquement pas sortir.
Le moment de démonstration canonique : **la démo avion** — couper le Wi-Fi à
la caméra en pleine conversation ; la conversation continue.

## I3 — Les publics, dans l'ordre stratégique

1. **Self-hosters / r/LocalLLaMA / HN — la distribution.** Ils ne paient pas
   cher mais font le bruit (preuve OpenClaw). On leur donne la démo avion et
   l'open-core. Vitrine NPU : AMD (qui a absorbé FastFlowLM) n'a pas d'app
   phare Ryzen AI grand public — être LA démo = co-marketing gratuit.
2. **Professions à secret (FR d'abord : avocats, experts-comptables, médecins)
   — le revenu.** Mode Huis clos + journal d'audit + one-pager « conformité
   par l'architecture » (CNB/HDS/AI Act). Modèle prouvé par screenpipe :
   open-core MIT + app payante lifetime 200-400 €.
3. **La famille (« Waly chez tes parents ») — le pari différé.** Dépannage
   vocal patient qui voit l'écran, en français, sans cloud. Bloqué par le
   matériel du parc installé (pas de NPU/16 Go) — à réévaluer quand Ryzen AI
   sera le milieu de gamme (~2028). Écrit ici pour ne pas l'oublier, pas pour
   le poursuivre.

**Tests de vérité AVANT tout investissement lourd** (falsifiables, quelques
jours) : (a) 5 conversations réelles avec des professionnels du secret —
« qu'est-ce qui t'empêche d'utiliser ChatGPT au cabinet, paierais-tu X € en
local ? » ; < 3/5 d'étincelle = l'angle pro tombe ; (b) la démo avion postée
(une fois la voix corrigée) ; pas de traction = le récit « local prouvé » ne
porte pas, pivoter tôt.

## I4 — Feuille de route v3 (remplace v2 R6/R7)

| Phase | Contenu | Critère de sortie mesuré |
|---|---|---|
| **R-V « La Voix »** | Pocket TTS `french_24l` (streaming, 1er chunk ~200 ms, voix signature clonée) + banc Kyutai `stt-1b-en_fr-candle` (VAD sémantique) au tribunal RAM | voix jugée « belle » par Michée en A/B vs Piper ; perçu ≤ 1,5 s ; budget 6,5 Go tenu |
| **R6a « Huis clos »** | Scellé réseau par session (WFP), indicateur visible, journal d'audit local, one-pager conformité | démonstration : session scellée, tentative de sortie réseau bloquée et journalisée, verdict d'un professionnel du secret |
| **R6b « L'Éveil »** | Wake word « Waly » (modèle CPU léger, budget < 300 Mo), présence sans clic, overlay comme corps principal | « Waly » à 3 m → éclipse s'éveille < 500 ms, faux positifs < 1/h |
| **R6c « La démo avion »** | Film 30 s (appel + écran + Wi-Fi coupé à l'image), post r/LocalLLaMA + HN, dossier vitrine AMD | publiée ; traction mesurée (test de vérité b) |
| **R7 « Les Mains »** | Web par DOM supervisé + workflows enregistrés/rejoués (inchangé du RFC 2026-07-03, D4-P4) | 3 workflows réels fiables |
| **R8 « BYOK »** | Cloud opt-in, clé de l'utilisateur, jamais nos serveurs (inchangé) | — |

**Supprimé** : mémoire ambiante persistante style screenpipe (ex-R7). La mémoire
de Waly reste ce qu'elle est déjà : des ÉVÉNEMENTS décrits en texte, opt-in,
locaux (journal visuel R4.5) — jamais un enregistrement continu.

**Ordre** : R-V est le GATE de tout (la voix actuelle est sous le seuil — verdict
Michée du 2026-07-10 vs Copilot ; on ne montre rien publiquement avant). R6a-c
peuvent se paralléliser après. La signature de code (décidée par Michée le
2026-07-20) est le prérequis de distribution de R6c.

## I5 — Design/UX : directives (méthode Michée inchangée : A/B nommés, il grave)

- L'éclipse « Marée » est GRAVÉE — on ne la rouvre pas. On étend ses ÉTATS :
  **Sceau** (Huis clos : l'éclipse s'enclot d'un anneau continu, fermé — le
  langage visuel du secret) et **Éveil** (au wake word : frémissement
  d'écoute). Monochrome gravé : le comportement change, jamais la couleur.
- L'overlay flottant (pop-up + cadre de présence, livrés R5) devient à terme
  le CORPS PRINCIPAL du produit ; la fenêtre app = l'atelier (sessions,
  compétences, agents). Une présence vit au bord de l'écran, pas dans un onglet.
- Une planche d'identité (Artifact « planche-identite-waly ») accompagne ce
  RFC pour verdict visuel de Michée sur Sceau/Éveil.

## I6 — Sessions Claude à part (exigence du mandat)

Chantiers dont la profondeur algorithmique/technique impose une session dédiée :

1. **R-V « La Voix » — LA session flagship.** Intégration streaming de Pocket
   TTS (candle/ONNX sous contrainte SAC-DLL, clonage d'une voix signature
   française, raccordement clause-par-clause à `prosody.rs`, préservation du
   barge-in et de l'AEC) + banc Kyutai STT-1B (VAD sémantique à têtes de
   prédiction vs notre `endpoint.rs` spéculatif) + tribunal RAM complet.
   C'est le chantier le plus profond : deux modèles neuronaux streaming
   temps réel dans la cascade Rust, sans casser les 67 tests voix.
2. **R6a « Huis clos ».** WFP (Windows Filtering Platform) par processus/
   session, attestation locale, journal d'audit signé — systems programming
   Windows avancé sous SAC.
3. **R7 « Les Mains ».** Agent DOM/CDP supervisé + trajectory caching (déjà
   cadré au RFC 2026-07-03 avec les chiffres OSWorld).

## Annexe — sources clés (recherche 2026-07-20)

Échecs matériels : digitalapplied.com (Sora/Humane/Rabbit, chiffres d'abandon).
Recall : geekwire.com (un an après), xda-developers.com (« almost nobody »).
OpenClaw : oreilly.com/radar, theadaptavistgroup.com (leçons d'adoption).
Companion : novaedgedigitallabs.tech (50 M), adalovelaceinstitute.org (risques).
Professions : juriadoc.fr (CNB mars 2026, art. 226-13), privacydesk.fr (HDS),
tensoria.fr (souveraineté cabinets). Mistral : mistral.ai/news/le-chat-enterprise.
Voix : kyutai.org (Pocket TTS jan. 2026, `french_24l` ; stt-1b-en_fr-candle).
