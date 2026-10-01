# Plan R3 — Desktop rebranché : l'UI actuelle sur waly-core in-process (2026-07-05)

> ⚠ **AMENDÉ le 2026-07-06 — pivot « desktop neuf » (Michée, 2026-07-05 soir,
> voir JOURNAL).** L'UI React de l'ancien monde n'est PAS portée : desktop
> entièrement neuf (`apps/desktop/`, front vanilla, design éclipse « fin &
> pur »), et le **transport = commandes Tauri `invoke` + Channel** — le shim
> HTTP loopback ci-dessous est **caduc** (il ne se justifiait que pour garder
> le React inchangé). Restent valides : le critère de sortie (installable
> double-clic, RAM < 300 Mo), le GATE build (franchi, piège 9), waly-core en
> lib, la règle d'honnêteté (pas de fonction fantôme), LLM externe, embedder
> lazy. Le contrat HTTP relevé plus bas est conservé comme **matériau de
> référence** de l'ancien monde.

> Phase R3 du RFC : « Desktop rebranché : UI actuelle sur le core Rust
> in-process ». **Critère de sortie mesuré : app installable double-clic,
> RAM à vide < 300 Mo.**
> On porte l'UI React validée (ancien `~/waly-desktop`, 65 tests) SANS la
> réécrire ; on remplace le cerveau (backend Node cloud) par waly-core.

## Le pivot de la phase

L'ancien desktop = **React 19 + Vite + Tauri 2** qui **spawn le backend Node
en sidecar** (`src-tauri/src/sidecar.rs` : `Command::new("node")`), attend le
port, émet l'event `sidecar-ready {port, token, userId}`, puis le React parle
**HTTP/SSE** à `http://127.0.0.1:<port>/core/*` (header `X-Sidecar-Token`).
Toute la surface transport du front tient dans **2 fichiers** :
`src/lib/sidecar.ts` (fetch REST + SSE de `/core/chat`) et
`src/lib/context-sse.ts` (stream `/core/context/stream`).

**R3 = tuer le sidecar Node, garder le contrat HTTP.** Le processus Tauri
monte un **mini-serveur HTTP loopback en Rust in-process** qui expose les
mêmes routes `/core/*` (même token, même `sidecar-ready`) et appelle
**waly-core en direct** (lib). **Le React ne change pas** — c'est le choix
d'architecture acté par Michée (2026-07-05) : parité la plus rapide, UI
validée intacte, la RAM tombe parce que Node + PGlite disparaissent.

Alternative écartée : commandes Tauri `invoke` + `Channel<T>` (100 % IPC, pas
de serveur) — plus propre mais impose de réécrire les 2 fichiers transport et
leurs tests. On y reviendra si le shim HTTP pose problème ; pas avant.

## Contrat HTTP à honorer (relevé sur le front, source de vérité)

| Route | Méthode | Backing waly-core R3 |
|---|---|---|
| `/core/user/bootstrap` | POST | réel (ouvre la base, user courant) |
| `/core/context` | GET | réel partiel (souvenir depuis `user_memory`) + dégradé |
| `/core/context/stream` | GET SSE | réel : `avatar_state` ; `context_update` ; `calendar_connected` **stub** |
| `/core/chat` | POST SSE | **réel : `run_turn_stream`** → event `result {response, artifacts:[]}` |
| `/core/approvals/:id/cancel` | POST | réel (store approbations R2) |
| `/core/missions` | GET | **stub** (feature ancien monde, hors R3) |
| `/core/memory/graph` | GET | **stub** (viz, hors R3) |
| `/core/llm/config` | GET/POST | **stub** dégradé (un seul modèle résident, pas de tier) |
| `/core/auth/google/start` | GET | **stub** (cloud → R7 opt-in BYOK) |

**Règle d'honnêteté UI** : un endpoint stubé renvoie un état vide/`needs_auth`
explicite — jamais une fonction fantôme. Les écrans qui en dépendent
(calendrier, missions, graphe) affichent « pas encore branché » plutôt que du
faux contenu. Ce qui est branché pour de vrai en R3 : **la boucle
conversationnelle** (bootstrap, chat outillé streaming, mémoire, approbations,
souvenir/avatar).

## Risques techniques à lever AU BANC avant d'écrire (méthode R2)

1. **[GATE] Tauri 2 se build-il sous SAC via la boucle WSL→windows-gnu ?**
   Tauri vise surtout msvc ; on ignore si un exe GUI Tauri (WebView2, crates
   `windows`/`webview2-com`) cross-compile en `x86_64-pc-windows-gnu` depuis
   WSL ET passe SAC au lancement (piège 3 : release non signés bloqués, DLL
   OK). C'est LE verrou de la phase.
   - Plan B si gnu coince : `cargo-xwin` (cible msvc cross depuis WSL) ;
   - Plan C : build natif Windows msvc du seul crate desktop (rustup+VS
     BuildTools 18 posés) — SAC bloquera peut-être le release non signé, mais
     l'installeur final sera **signé** de toute façon (cf. CLAUDE.md piège 3).
   - Mesure de sortie du bench : **l'app hello-Tauri s'ouvre en double-clic
     côté Windows.**
2. **waly-core en dépendance lib du crate desktop** : SQLite bundled + ort
   load-dynamic + tokio compilent-ils dans le même target que Tauri ?
   (waly-core est déjà lib+bin — a priori OK ; à confirmer au build.)
3. **RAM à vide < 300 Mo** : WebView2 seul ~120-150 Mo ; waly-core au repos
   (SQLite ouvert, **embedder e5 NON chargé** — lazy) doit tenir le reste.
   L'LLM est FLM externe, jamais résident dans le desktop. À mesurer réel.

## Chantiers

| # | Contenu | Sortie mesurée |
|---|---|---|
| 1 | **[GATE] Faisabilité build Tauri sous SAC** : crate Tauri minimal dans le workspace, cross-build windows-gnu (fallback xwin/msvc), lancement double-clic Windows | l'app vide s'ouvre côté Windows |
| 2 | **Shim HTTP in-process** : remplacer `sidecar.rs` (spawn node) par un serveur loopback Rust (routes `/core/*`, token, event `sidecar-ready`) ; SSE branché sur `run_turn_stream` | `curl` local sur `/core/chat` streame une réponse waly-core |
| 3 | **Branchement waly-core + stubs honnêtes** : bootstrap, context (souvenir réel + dégradé), approvals ; missions/graph/llm/google stubés proprement | le front réel dialogue (chat + mémoire) contre le core, écrans stub explicites |
| 4 | **Portage UI + installeur** : porter le front React tel quel dans le monorepo, `vite build`, bundle Tauri (nsis/msi), tray + Alt+W conservés | **double-clic installe et lance ; RAM à vide < 300 Mo mesurée** |
| 5 | **Parité + ménage** : tests transport, non-régression UI, JOURNAL/ADR | tests verts, phase documentée |

**Sortie de phase R3** : app installable double-clic, RAM à vide < 300 Mo,
conversation réelle (chat + mémoire + outils) dans l'UI — validée par Michée.

## Décisions actées

- **Transport = shim HTTP loopback Rust in-process** (choix Michée 2026-07-05).
  React **inchangé** ; on ne touche pas `lib/sidecar.ts` / `context-sse.ts`.
- **waly-core consommé en LIB** (déjà lib+bin). Base SQLite = **la même**
  `C:\waly\data\waly.db` que le bin `waly` (source unique de mémoire).
- **Emplacement monorepo** : l'app desktop vit sous `apps/desktop/` (front
  React) avec `apps/desktop/src-tauri` (crate Rust ajouté au workspace).
  Layout finalisé au chantier 1. (Règle monorepo unique : plus de dépôt frère.)
- **Endpoints hors périmètre waly-core** (calendrier/Google, missions,
  memory-graph, artifacts, tier LLM) : **stubés/dégradés**, renvoyés à R4+/R7.
  Aucune fonction fantôme dans l'UI (règle d'honnêteté).
- **LLM = FLM externe** (port 52626), jamais résident dans le processus
  desktop. **Embedder e5 lazy** (non chargé au repos, pour tenir < 300 Mo).
- **Cible de build** : `x86_64-pc-windows-gnu` via la boucle WSL (pièges 3/6)
  en priorité ; `CARGO_TARGET_DIR` hors dépôt ; fallback msvc/xwin au
  chantier 1 si Tauri gnu coince. L'installeur final sera **signé**.
- **Dépendance serveur HTTP** : pure-Rust uniquement (axum/tokio déjà au
  workspace, ou hand-rolled std::net comme le client LLM de waly-core) —
  compatible SAC (compilé chez nous, build-scripts ELF).
