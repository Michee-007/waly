# Installeur Waly (R3 ch. 4)

`Waly-Setup.exe` : NSIS, **per-user** (`%LOCALAPPDATA%\Programs\Waly`),
raccourcis Menu Démarrer + Bureau, désinstalleur enregistré (HKCU,
« Applications installées »). Double-clic = installe ET lance. La
désinstallation **ne touche jamais** `C:\waly\data` (mémoire/conversations).

## Huis clos : le service scelleur (bundlé, ADR 2026-07-21)

Le service `waly-seal-svc.exe` est **bundlé** et installé au setup. L'app
elle-même reste per-user (zéro UAC), mais le service exige une élévation :
**une seule UAC** à l'installation (et à la désinstallation). Le NSIS lance
`waly-seal-svc.exe setup` en `runas` — la commande **relocalise l'exe dans
`%ProgramFiles%\Waly`** (dossier non modifiable par un utilisateur standard :
un service SYSTEM ne doit jamais tourner depuis un chemin où l'utilisateur peut
écrire, sinon escalade de privilège), l'enregistre en **AUTO_START** et le
démarre → réseau **scellé par défaut, dès le boot**. Si l'UAC est refusée, le
service manque et l'app affiche « réseau non scellé — réparer » (dégradé
honnête). `build-installer.sh` construit le service dans le même target et le
passe au NSIS (`-DSVC_PATH`).

## Fabriquer (depuis WSL, boucle officielle)

```bash
# one-shot : makensis SANS root (paquet Ubuntu extrait localement)
cd /tmp && apt-get download nsis nsis-common
mkdir -p ~/nsis-local && dpkg -x /tmp/nsis-common_*.deb ~/nsis-local \
  && dpkg -x /tmp/nsis_*.deb ~/nsis-local

# build release + installeur
cd /mnt/c/waly
CARGO_TARGET_DIR=~/waly-target-wsl-tauri cargo build --release -p waly-desktop \
  --target x86_64-pc-windows-gnu
bash apps/desktop/installer/build-installer.sh release
# -> C:\waly\bin\Waly-Setup.exe
```

## SAC (Smart App Control)

Mesuré 2026-07-07 : l'installeur non signé ET le Waly.exe release installé
sont **passés du premier coup** (binaires compilés localement, pas de
Mark-of-the-Web). Les verdicts ISG restent PAR binaire et imprévisibles
(piège 3 d'AGENTS.md) : si un build est bloqué, boucle toucher-rebuilder.
⚠ Ça ne vaut QUE sur cette machine : un Waly-Setup.exe **téléchargé** chez
quelqu'un d'autre aura le Mark-of-the-Web → il FAUT signer pour distribuer.

## Signature (requis pour distribuer — décision/achat Michée)

Deux chemins :
1. **Azure Trusted Signing** (~10 $/mois) : identité à faire valider,
   signature cloud via `signtool` + dlib — le plus simple en 2026 pour un
   indé, réputation SmartScreen/SAC rapide.
2. **Certificat OV classique** (Sectigo/GlobalSign, ~200-400 €/an, clé sur
   token USB FIPS obligatoire depuis 2023).

Une fois le certificat en main, signer DANS CET ORDRE :
`signtool sign … Waly.exe` (avant makensis) → `makensis` →
`signtool sign … Waly-Setup.exe`. Le script `build-installer.sh` prendra un
crochet à ce moment-là. Prérequis machine cible : WebView2 Runtime (présent
sur tout Windows 11 ; bootstrapper à ajouter le jour de la distribution).
