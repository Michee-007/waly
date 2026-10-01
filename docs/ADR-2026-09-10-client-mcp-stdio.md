# ADR 2026-09-10 — Client MCP, transport stdio, approbation par défaut

**Statut** : accepté (« go MCP », Michée, 2026-09-10).
**Contexte** : comparatif Hermes agent (`RESEARCH-2026-09-10-hermes-agent-vs-waly.md`)
— Hermes branche 100+ outils communautaires ; Waly a un catalogue de 17 outils
codés à la main. Le Model Context Protocol est le standard de fait pour brancher
des outils à un agent. ROADMAP : « Local MCP client (stdio only) ».

## Décision

1. **Client MCP natif en Rust pur** (`waly_core::mcp`), zéro dépendance neuve
   (piège 3 SAC) : JSON-RPC 2.0 ligne à ligne, poignée de main
   `initialize`/`notifications/initialized`, `tools/list` paginé, `tools/call`,
   réponses aux requêtes serveur (`ping` honoré, le reste refusé poliment).
2. **Transport stdio UNIQUEMENT.** Un serveur MCP est un processus enfant ;
   aucun port ouvert, aucune socket. Le transport HTTP (streamable) est
   REFUSÉ : il ferait de Waly un client réseau.
3. **Déclaration dans waly.toml** (`[mcp.<nom>]` : `commande`, `arg1..argN`,
   `confiance`, `outils`, `max_outils`, `actif`). Rien n'est lancé sans
   déclaration explicite de l'utilisateur.
4. **Approbation humaine par défaut.** Le processus serveur (node, python…)
   tourne **hors du sceau WFP** : le sceau filtre par exécutable, et sceller
   `node.exe`/`python.exe` couperait le réseau de toute la machine. Un outil
   MCP peut donc avoir un effet externe → il est SENSIBLE (gate de risque R2,
   carte Approuver/Refuser, attente persistée) sauf `confiance = "lecture"`
   posé par l'utilisateur pour ce serveur.
5. **Mêmes murs que le natif** : un outil MCP est un `Tool` du registre —
   murs financiers (nom + intention), validation des arguments, gate de
   risque, anti-boucle. Aucun contournement.
6. **Budget prompt** : 8 outils max par serveur (`max_outils`), liste blanche
   `outils`, descriptions tronquées à 200 caractères, noms
   `mcp_<serveur>_<outil>` (64 max). Le catalogue est figé au démarrage →
   discipline append-only R4.5 respectée (le bloc d'outils ne change pas
   entre deux tours).
7. **Jamais bloquant** : serveur absent/lent/cassé = ligne de journal
   (`Waly: mcp: …` / `[mcp] …`), Waly démarre quand même (délais : 20 s
   init, 60 s appel ; lecture par thread + `recv_timeout`, jamais de
   `set_read_timeout` — piège 7).

## Périmètre v1

Desktop et CLI. **Pas la voix** : chaque processus lance ses propres
serveurs, et la veille doit rester légère (budget RAM signé) ; la voix
suivra quand le coût sera mesuré. Outils seulement (ni ressources, ni
prompts, ni sampling).

## Conséquences

- La promesse « rien ne sort » devient **conditionnelle aux serveurs MCP
  déclarés** : c'est écrit ici, dans le README et le journal de démarrage
  (« hors du sceau reseau »). Sans serveur déclaré, rien ne change.
- À faire : afficher les serveurs MCP actifs dans le panneau « Vie privée —
  la preuve » ; sélection d'outils contextuelle quand le catalogue grossit ;
  conscience du sceau nuancée si un serveur déclaré a un accès réseau.
- Windows : un lanceur `.cmd` se déclare avec son extension
  (`commande = 'npx.cmd'`).
