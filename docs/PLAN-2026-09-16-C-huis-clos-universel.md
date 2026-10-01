# PLAN — Chantier C « Huis clos universel » (2026-09-16)

> Suite de R6a (`docs/PLAN-2026-07-20-R6a-huis-clos.md`,
> `docs/ADR-2026-07-21-huis-clos-scelle-par-defaut.md`). Séquence B → **C** →
> démo → publier (`lab/publication/README.md`, mémoire
> `sequence-b-c-demo-publication`). B est livré ; C ouvre ici.

## Ce qu'on construit

Faire du sceau réseau une **brique indépendante capable de sceller n'importe
quel agent local** (Hermes compris), **sans délaisser Waly** (son périmètre
reste auto-scellé par défaut, ADR 2026-07-21). C'est le seul fossé réellement
unique face à Hermes (`docs/RESEARCH-2026-09-10-hermes-agent-vs-waly.md`).

Aujourd'hui le service `waly-seal-svc` (SYSTEM) ne scelle QUE le périmètre Waly
(`waly*.exe`, `flm.exe`) ; l'IPC est volontairement étroit. C généralise le
scellé à un exe arbitraire, **derrière une autorisation admin**.

## Décisions (tranchées avec Michée le 2026-09-16)

1. **Forme = les deux, CLI d'abord.** Cœur = commande générique dans le
   service + sous-commandes CLI (`seal`/`unseal`/`list`/`journal`) faisant du
   `waly-seal-svc` une brique utilisable seule (distribuable, testable,
   mesurable). L'UI (panneau « Sceller un autre agent ») vient **après**, par
   dessus le même cœur.
2. **Sceller un agent TIERS = action ADMIN obligatoire** (ADR
   `docs/ADR-2026-09-16-sceller-un-tiers-est-admin.md`). Raison : sceller un
   exe qui n'est pas à nous *bloque le réseau d'un processus tiers* → vecteur
   de déni de service si un malware user-level pouvait le demander. Le service
   **vérifie l'élévation du client du pipe** (impersonation + `TokenElevation`)
   et refuse tout scellé hors-périmètre-Waly à un appelant non élevé.
   Cohérent avec la voie « service élevé, robuste pas facile » entérinée en
   R6a. Le périmètre Waly (`Rejoindre`, add-only `waly*`) reste user-level.

## Modèle de sécurité (le point central)

- **Le sceau Waly** (périmètre id 0) : inchangé. Auto-scellé au boot, l'app
  `Rejoindre` en user-level (add-only, `exe_admissible`). Jamais descellable
  via le pipe.
- **Le sceau d'un agent tiers** : nouveau. Requiert un appelant **élevé**.
  - Le serveur de pipe identifie le client (`ImpersonateNamedPipeClient` →
    `OpenThreadToken` → `GetTokenInformation(TokenElevation)` → `RevertToSelf`).
  - `Sceller`/`Desceller` d'un exe **non admissible Waly** : refusé si non
    élevé (`ACCESS_DENIED` clair). Sceller le périmètre Waly reste permis.
  - Fermeture d'un trou latent : aujourd'hui `Sceller { exes:[n'importe quoi] }`
    est accepté de tout utilisateur authentifié (DoS possible). C le **durcit**.
- Un agent tiers scellé a sa **propre session** (id dérivé du chemin d'exe,
  négatif, stable → `seal`/`unseal`/`journal` par exe cohérents et idempotents).
  Distinct du périmètre (0) et des sessions Waly (positives).

## Chantiers

- **C0 — cœur service générique + admin gate** (ce jour) :
  - `wfp.rs` : détection d'élévation du client, `seal`/`unseal` par session
    tiers ; garde admin dans `seal` pour exe non-Waly. `session_pour_exe(exe)`
    (hash stable → id négatif).
  - `ipc.rs` : le protocole accepte déjà `Sceller`/`Desceller`/`Journal` — on
    ajoute le passage de l'élévation à `traiter` (le serveur, pas le client).
  - `service.rs` : `servir` capture l'élévation du client et la passe à
    `traiter`.
  - `bin/waly-seal-svc.rs` : sous-commandes `seal <exe…>`, `unseal <exe>`,
    `list`, `journal <exe>` (client pipe ; message clair si refus = « relance
    en administrateur »).
- **C1 — banc/mesure** : sceller un vrai agent tiers (ex. un exe témoin qui
  sort sur le réseau), prouver blocage + journal, mesurer pose/levée. Refus
  net en compte standard. Verdicts chiffrés dans `lab/huisclos-banc/README`.
- **C2 — UI « Sceller un autre agent »** (après C0/C1) : panneau desktop,
  choisir un exe, geste explicite → seal admin (UAC une fois), journal par
  agent, liste des agents scellés. Réutilise l'éclipse/anneau « Marée ».
- **C3 — docs** : ADR (fait), one-pager étendu « pour n'importe quel agent »,
  README brique, JOURNAL, mémoire.

## Critères de sortie

- CLI : `seal <exe-tiers>` élevé → exe bloqué en sortie, loopback intact,
  journal exact ; `unseal <exe>` → restauré ; compte standard → refus clair.
- Waly inchangé : périmètre toujours auto-scellé, app rejoint sans UAC.
- Mesure de pose/levée < 100 ms (comme R6a).
- Verdict terrain Michée (et, pour la promesse « Hermes compris », sceller un
  agent tiers réel dès qu'on en a un sous la main).

## Hors périmètre / risques

- WFP filtre par **chemin d'exe**, pas par PID : sceller un agent tiers bloque
  toutes les instances de cet exe (voulu pour un agent ; à documenter).
- Un exe partagé (ex. `python.exe`, `node.exe`) scellé couperait d'AUTRES
  logiciels : la CLI **avertit** si l'exe a l'air d'un interpréteur partagé
  (heuristique de nom), sans l'interdire (l'admin décide).
- UDP : dropé + journalisé mais `send_to` renvoie OK localement (piège R6a
  gravé) → la preuve passe par le journal.
