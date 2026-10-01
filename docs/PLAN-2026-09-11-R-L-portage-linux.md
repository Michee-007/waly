# PLAN R-L — Portage Linux (puis macOS)

> Lancé le 2026-09-11 (« go point 4 », Michée) — point 4 de la stratégie
> open source « comme Hermes : ne dépendre d'aucune machine ». Précédents :
> `waly.toml` + moteur OpenAI-compat (pt 1), vision adaptative (pt 2),
> recommandation matérielle (pt 3). Windows reste la plateforme de
> référence ; Linux devient la deuxième, macOS la troisième.

## Inventaire des couplages Windows (2026-09-11)

| # | Couplage | Où | Voie Linux |
|---|---|---|---|
| A | ~45 chemins `C:\waly\…` en dur | core, voix, desktop, sight | `waly_core::chemins` (ch. 0) ✅ |
| B | noms `*.dll` (onnxruntime, sherpa) | core, sight, voix | `DLL_PREFIX/SUFFIX` (ch. 0) ✅ |
| C | caméra Media Foundation (nokhwa `input-msmf`) | waly-sight | nokhwa `input-v4l` (bindgen → libclang à vérifier) |
| D | capture d'écran GDI | waly-sight/screen.rs | X11 (xcb) puis portail Wayland (PipeWire) |
| E | **sceau WFP** + service SCM + pipe nommé | waly-seal, core/sceau | cgroup v2 + nftables `socket cgroupv2`, service systemd, socket Unix — ADR à écrire |
| F | spawn `waly-voice.exe`, installeur NSIS | desktop, installer | `chemins::exe` (ch. 0) ✅ ; AppImage/.deb |
| G | détection matériel par le registre | materiel.rs | `/sys/class/drm`, `/sys/class/accel` (ch. 0) ✅ |
| H | audio | voix (cpal) | ALSA/PulseAudio via cpal — exige `libasound2-dev` au build |

## Chantiers et gates (critères MESURÉS, pas de passage sans eux)

- **Ch. 0 — Socle portable (2026-09-11)** : chemins, noms de bibliothèques,
  détection matérielle Linux. GATE 0 : workspace + tests verts sur hôte
  Linux (WSL) ; Windows INCHANGÉ (mêmes chemins, e2e `waly materiel` +
  `waly turn` sur la machine de référence).
- **Ch. 1 — Waly en ligne de commande sur Linux** (`waly turn/chat/materiel`)
  contre Ollama Linux. GATE 1 : tour complet avec outil sur un vrai Linux
  (VM, machine communautaire ou CI avec un petit modèle) — PAS sur la
  machine de référence (règle : aucune inférence dans WSL, RAM ÷2).
- **Ch. 2 — Voix Linux** : cpal/ALSA, `libonnxruntime.so`,
  `libsherpa-onnx-c-api.so`. GATE 2 : `waly-voice text` puis `talk` sur un
  vrai Linux ; premier son ≤ 2 s à chaud (même critère que Windows).
  2026-09-11 : le build `--features service` **compile** sur l'hôte Linux
  (ELF 84 Mo, debug) avec `libasound2-dev` extrait sans droits dans
  `~/waly-sysroot` (paquet officiel, SHA-256 vérifié — recette au JOURNAL).
  Reste l'exécution réelle (audio + IA) sur un vrai Linux.
- **Ch. 3 — Desktop Linux** (Tauri/webkit2gtk) + paquet AppImage. GATE 3 :
  installation et lancement sur Ubuntu 24.04, RAM au repos < 300 Mo.
- **Ch. 4 — Vision Linux** : caméra V4L2, capture X11. GATE 4 : mode Appel
  et mode Écran de bout en bout.
- **Ch. 5 — Huis clos Linux** : ADR + banc (cgroup v2 + nftables, service
  systemd root, IPC socket Unix étroit, fail-closed). GATE 5 : les trois
  gates du banc R6a rejoués (sortie bloquée + journal, loopback vivant,
  coût de pose). Sans lui, la promesse « rien ne sort » n'est PAS tenue sur
  Linux — l'app doit le dire (statut « réseau non scellé »).
- **macOS (après)** : sceau = Network Extension (entitlements Apple
  Developer) — décision et coût à trancher avec Michée.

## Prérequis ouverts

- Un Linux de test (VM ou machine) : la machine de référence ne peut pas
  héberger d'inférence Linux (WSL). Idéalement une CI publique, liée à la
  décision de publication (dépôt neuf).
