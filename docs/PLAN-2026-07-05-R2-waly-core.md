# Plan R2 — waly-core : chat, mémoire, outils, sûreté (2026-07-05)

> Phase R2 du RFC : « waly-core Rust minimal : chat, mémoire SQLite+vec,
> 15-20 outils natifs, sûreté portée ». Critère de sortie : parité
> fonctionnelle desktop (chat), prompt < 1k tokens.
> Instruction faite le 2026-07-05 : inventaire complet de l'ancien backend
> (`~/waly-backend`, WSL) — on porte la LOGIQUE comme specs, jamais le code.

## Risques techniques levés au banc (2026-07-05)

1. **Tool-calling natif FLM v0.9.43 + qwen3-it:4b : OK.** `tools` OpenAI-compat
   → `finish_reason:"tool_calls"` + arguments JSON propres ; réinjection
   `role:"assistant"(tool_calls)` + `role:"tool"` → réponse correcte.
   Fini le bloc de 2 500 tokens d'outils en prompting de l'ancien monde.
2. **rusqlite (bundled) + sqlite-vec : OK dans la boucle WSL→windows-gnu.**
   sqlite3.c et sqlite-vec compilés par mingw (build-scripts ELF, hors SAC),
   liés statiquement (du C, pas du C++ tiers — compatible piège 3). Test natif
   vert : `vec0` répond, KNN correct.

## Ce qu'on porte de l'ancien monde (rapport d'inventaire, session 2026-07-05)

### Socle de sûreté — l'ORDRE est la spec (avant tout dispatch)
1. **Mur financier par nom d'outil** : tokeniser le nom (split non-lettres),
   bloquer si un token ∈ liste interdite (transfer/virement/pay/buy/invest/
   crypto/épargne…). Déterministe, même pour un outil halluciné.
2. **Mur financier par intention** : verbe de mouvement d'argent dans le
   message utilisateur (vire/paie/transfère/rembourse, tolérant STT) ET
   contexte argent (€/euros/balles/3k) ET routage vers un outil financier →
   clarification, jamais d'exécution. (Trou vécu en vocal : « vire 50 € »
   enregistré comme dépense.)
3. **Validation de dispatch** : nom ∈ registre (pas de fuzzy-match), arguments
   JSON objet, **types+enums validés sur les champs PRÉSENTS mais `required`
   NON exigé** (calibré sur corpus réel : l'exiger rejetait 27/36 appels
   vivants ; les outils gèrent leurs défauts). Rejet → erreur renvoyée AU
   MODÈLE ; budget de retente 1, puis arrêt du tour.
4. **Gate de risque** : catégorie par outil — `read` (rien), `write`
   (réversible : mémoire/notes, pas de confirmation), `sensitive`
   (irréversible/effet externe → confirmation humaine avant exécution).
   Table de capacités unique (fusion tools.js + capabilities.js de l'ancien
   monde) avec invariants fail-fast : `paiement ⟹ irréversible` ; outil
   inconnu ⟹ irréversible par défaut.
5. **Anti-boucle** : même appel (hash nom+args) ≥ 3× ou ping-pong A→B→A→B
   sur les 4 derniers → bloqué. Boucle agentique bornée (10 itérations max
   ancien monde ; 4 pour un 4B, à mesurer). Filet final : dernier appel sans
   outils pour forcer du texte.
6. **Taint** (contenu non fiable : emails, webhooks, transactions) : à câbler
   proprement — l'ancien monde avait la machinerie mais PAS le wiring
   ingestion→tour (dette identifiée). Concerne R2 tard (aucun outil
   d'ingestion externe au début) : posé dans le schéma (`tainted` sur les
   embeddings, exclus de la recherche), wiring quand les premiers outils
   externes arrivent.

### Mémoire (SQLite + sqlite-vec, remplace PGlite + pgvector + Voyage cloud)
- `user_memory` : catégorie CHECK(fact|preference|context|event), clé UNIQUE,
  valeur, confiance, source (declared|inferred), `expires_at`.
  **TTL par catégorie** : fact=permanent, preference=90 j, context=30 j,
  event=7 j. **Refresh-on-read** : toute lecture remet TTL et confiance.
- `notes`, `conversations` (historique brut).
- `embeddings` : vec0, `tainted` exclu de la recherche. **Embeddings 100 %
  locaux** (rupture assumée avec voyage-3 cloud 1024d) : candidat
  multilingual-e5-small int8 ONNX (~110 Mo, 384d) sur l'onnxruntime.dll déjà
  en place (ort load-dynamic). À bencher (qualité FR, latence CPU).
- Recherche : ILIKE d'abord (chantier 3), puis **hybride** porté tel quel :
  0,7·cosinus + 0,3·mot-clé, seuil 0,75, top 5, fallback ILIKE (chantier 4).

### Outils cœur R2 (13 natifs livrés 2026-07-05)
`heure` ✅ · mémoire : `memoriser`, `chercher_memoire`, `oublier` ✅ ·
notes : `creer_note`, `chercher_notes`, `lister_notes` ✅ · tâches :
`creer_tache`, `lister_taches`, `maj_tache` ✅ · rappels : `poser_rappel`,
`lister_rappels`, `annuler_rappel` ✅ (déclenchement vocal au repos câblé).
Restent EXTERNES/opt-in (pas R2) : `meteo`, `chercher_web`.
⚠ **Budget mesuré : 13 outils = 1133 tok > 1k** (structure JSON par outil,
~86 tok/outil ; resserrer les descriptions ne rend que ~40 tok). Le core
LEAN (7 outils) = 616 < 1k. Parade pour les deux : **sélection d'outils
contextuelle** (tool-selector de l'ancien monde) → backlog. Reste
très loin des 2 500 tok d'outils en PROMPTING de l'ancien monde.

## Chantiers

| # | Contenu | Sortie mesurée |
|---|---|---|
| 1 | **Fondation** : client LLM outillé (std::net, jamais SO_RCVTIMEO), registre+dispatch, boucle agentique bornée, store SQLite+vec prouvé, bin `waly` (turn/chat) | ✅ fait 2026-07-05 — tour réel outillé 4,6 s, 9 tests |
| 2 | **Sûreté portée** : murs financiers ×2, table capacités+invariants, gate de risque, anti-boucle, validation calibrée, budget retente | tests exhaustifs sur les règles portées ; « vire 50 € » refusé au banc |
| 3 | **Mémoire v1** : schéma+migrations, TTL+refresh-on-read, outils mémoire/notes, historique persisté | souvenirs survivent au restart ; rappel correct en conversation réelle |
| 4 | **Embeddings locaux + hybride** : bench e5-small int8, vec0, 0,7/0,3 | ✅ fait 2026-07-05 — « plat préféré » → ndolé sans mot commun ; charge 1,7 s, 2-3 ms/texte ; ⚠ seuil 0,75 non transposable (cosinus e5 compressés) → plancher 0,78 + top 3 classé ; bonus ch. 6 anticipé : souvenirs injectés au prompt chaque tour |
| 5 | **Approbations** : gate sensitive → confirmation conversationnelle, table pending (24 h), résolution au tour suivant | ✅ fait 2026-07-05 — claim atomique (double oui inoffensif), E2E confirmation ET refus validés ; leçon 4B : imposer LA forme exacte de l'appel dans le prompt, sinon il « annule » avec le mauvais outil et affirme l'avoir fait ; reste pour plus tard : auto-approbation apprise, garde taint→HITL |
| 6 | **Parité + prompt** : prompt système unifié < 1k tokens mesurés, brancher waly-voice sur waly-core (le prompt voix lève sa garde « aucun outil ») | ✅ fait 2026-07-05 — **prompt mesuré 616 tok < 1k ✓** ; voix branchée sur `run_turn_stream` (mémoire+outils par la voix, disciplines du durcissement conservées) ; coût constaté : TTFT 1,16→2,2-2,4 s (bloc d'outils re-préfillé, pas de cache FLM) ; rappels schedulés reportés au backlog R2+ |

**Sortie de phase R2** : validation réelle par Michée (conversation vocale
avec mémoire et outils). Backlog R2+ : auto-approbation apprise
(`permission_memory`), garde taint→HITL, rappels/tâches schedulés, outils
externes (météo, web) en opt-in.

## Décisions actées
- **Un seul crate** `waly-core`, la voix consommera `chat::run_turn` (R2 ch. 6).
- **SQLite fichier unique** `C:\waly\data\waly.db` (WAL), migrations
  idempotentes dans le code.
- **Pas de BullMQ/Redis** : rappels = table + boucle de tick dans le service.
- **Pas de cloud** : embeddings locaux, météo/web = intégrations opt-in plus
  tard (BYOK pour le web éventuellement).
- Le champ `writesMemory`/`irreversible` vit dans UNE table Rust au registre.
