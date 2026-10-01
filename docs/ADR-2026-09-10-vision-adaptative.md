# ADR 2026-09-10 — Vision adaptative : Waly s'ajuste à ce que le modèle sait faire

**Statut** : accepté (« go point 2 », Michée, 2026-09-10).
**Contexte** : stratégie open source (parité Hermes agent) — Waly ne doit
dépendre d'aucune machine. Sur la machine de référence, le cerveau unique
texte+vision (`qwen3vl-it:4b` sur NPU/FastFlowLM) est bloqué par Smart App
Control ; sur Ollama/Vulkan AMD, `qwen3-vl` plante (0.33.3 ET 0.34.0) ;
`gemma3:4b` voit mais refuse les outils (HTTP 400). Jusqu'ici, Waly envoyait
l'image au cerveau quoi qu'il sache faire.

## Décision

Un module `waly_core::vision` résout UNE fois par processus :

1. **Déclaration** (`WALY_MODEL_VISION` › waly.toml `[llm] modele_vision`) :
   égale au cerveau → **Directe** ; sinon → **Déléguée** : ce modèle
   REGARDE l'image et la décrit (2-3 phrases factuelles, la demande de
   l'utilisateur en contexte) ; le cerveau reçoit la description en texte
   (« la vision comme un sens »).
2. **Sinon, sonde des capacités** du cerveau (`/api/show` d'Ollama, sans
   charger le modèle) : il déclare `vision` → **Directe** ; il ne la déclare
   pas → **Absente** : aucune image n'est envoyée, le modèle reçoit une
   consigne d'honnêteté (« dis-le, n'invente pas »), le journal suggère les
   modèles installés qui voient.
3. **Moteur inconnu** (FastFlowLM, autre serveur) : **Directe** —
   comportement historique, aucune route inconnue envoyée à FLM.

**Point d'application UNIQUE** : `vision::rendre_visible(messages)` au début
de chaque round de `run_turn` / `run_turn_stream_with`. Tous les chemins
d'image y passent (outils `regarder` / `regarder_ecran`, raccourcis
d'intention desktop et voix, tours écran). Les moments proactifs du desktop
passent par `vision::client_vision` (le modèle qui voit ; aucun → pas de
moment). Aucune sonde sans image.

## Conséquences

- Waly fonctionne avec un cerveau texte seul (vision absente, honnête) ou
  avec un couple cerveau + modèle vision, sur n'importe quel serveur
  compatible — c'est le socle de la portabilité.
- **Coût du mode délégué sur une petite machine** : Ollama ne garde qu'un
  modèle en mémoire avec peu de RAM → chaque tour visuel recharge le modèle
  vision puis le cerveau, et le cache de préfixe du cerveau est perdu. C'est
  LENT (dizaines de secondes) mais ça VOIT ; sur une machine avec plus de
  mémoire, les deux modèles restent chargés. Le cerveau unique qui voit
  (NPU) reste la voie rapide.
- Budget signé : le mode délégué ajoute un modèle, chargé À LA DEMANDE,
  jamais deux résidents imposés.
- Le mode est figé par processus : changer `modele_vision` = relancer Waly.
