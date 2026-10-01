# ADR — Sceller un agent tiers est une action administrateur (2026-09-16)

- **Statut** : accepté (décision Michée, 2026-09-16).
- **Contexte** : chantier C « Huis clos universel »
  (`docs/PLAN-2026-09-16-C-huis-clos-universel.md`). Le sceau réseau WFP
  (service SYSTEM `waly-seal-svc`, R6a) devient une brique capable de sceller
  n'importe quel agent local, pas seulement le périmètre Waly.

## Décision

Le service **refuse de sceller un exe hors du périmètre Waly** (tout ce qui
n'est pas `waly*.exe` / `flm.exe`) **si le client du pipe n'est pas élevé**
(token élevé / groupe Administrateurs). Sceller le périmètre Wali et le
`Rejoindre` add-only restent user-level, comme aujourd'hui.

## Pourquoi

Sceller un exe, c'est **bloquer sa sortie réseau au niveau du noyau**. Pour un
exe tiers, c'est un effet sur un processus qui n'est pas le nôtre : si un
appelant user-level quelconque pouvait le demander, un logiciel malveillant
tournant sous le compte de l'utilisateur pourrait **couper le réseau de
n'importe quel logiciel** (navigateur, antivirus, client VPN…) = déni de
service silencieux. L'IPC de R6a était étroit précisément pour éviter cette
surface ; C l'élargit, donc l'autorisation doit monter d'un cran.

L'élévation est le bon niveau : elle est déjà requise pour toute modification
durable du pare-feu Windows ; elle est cohérente avec la voie « service élevé,
robuste pas facile » entérinée en R6a (l'ACL one-time avait été **disqualifiée**
car elle mettait la capacité de manipuler le sceau dans un token user-level).

## Comment

Le serveur de pipe identifie le client avant de traiter une requête de scellé
générique : `ImpersonateNamedPipeClient` → `OpenThreadToken(TOKEN_QUERY)` →
`GetTokenInformation(TokenElevation)` → `RevertToSelf`. Le résultat (`élevé`)
est passé à `traiter`, qui applique la garde uniquement au scellé d'un exe non
admissible Waly. En pratique, l'utilisateur lance `waly-seal-svc seal <exe>`
depuis une console élevée (UAC une fois) ; sinon message clair.

## Conséquences

- Ferme un trou latent : `Sceller { exes:[arbitraire] }` n'est plus accepté de
  tout utilisateur authentifié.
- La CLI/UI qui scelle un tiers doit être lancée élevée (UAC). Frottement
  assumé, borné à l'action tierce.
- Le périmètre Waly et l'expérience « scellé par défaut sans bouton »
  (ADR 2026-07-21) sont **inchangés**.
- WFP filtrant par chemin d'exe (pas PID), sceller un agent tiers bloque toutes
  ses instances — voulu pour « sceller un agent », documenté.

## Alternatives écartées

- **Liste blanche déclarée** (config admin posée une fois, puis seal/unseal
  user-level des exes listés) : moins de frottement mais surface plus large et
  état à gérer/protéger ; on préfère l'admin per-action, quitte à l'assouplir
  plus tard si le terrain le réclame.
- **Ouvrir le pipe à tout exe pour tout utilisateur** : rejeté (le vecteur DoS
  ci-dessus).
