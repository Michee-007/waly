#!/usr/bin/env bash
# Fabrique Waly-Setup.exe depuis WSL (boucle officielle windows-gnu).
# Prerequis one-shot (sans root) :
#   cd /tmp && apt-get download nsis nsis-common
#   mkdir -p ~/nsis-local && dpkg -x /tmp/nsis-common_*.deb ~/nsis-local \
#     && dpkg -x /tmp/nsis_*.deb ~/nsis-local
# Usage : bash apps/desktop/installer/build-installer.sh [debug|release]
set -euo pipefail

PROFILE="${1:-release}"
TDIR="$HOME/waly-target-wsl-tauri/x86_64-pc-windows-gnu/$PROFILE"
REPO="$(cd "$(dirname "$0")/../../.." && pwd)"
OUT="$REPO/bin/Waly-Setup.exe"

EXE="$TDIR/waly-desktop.exe"
[ -f "$EXE" ] || { echo "exe absent: $EXE (builder d'abord)"; exit 1; }
LOADER="$(find "$TDIR/build" -name WebView2Loader.dll -path '*x64*' | head -1)"
[ -n "$LOADER" ] || { echo "WebView2Loader.dll introuvable sous $TDIR/build"; exit 1; }

# Huis clos (ADR 2026-07-21) : le service scelleur est bundle. On le construit
# dans le MEME target puis on le passe au NSIS. Sans lui, pas de scelle-par-
# defaut a l'install (l'app afficherait "reseau non scelle - reparer").
RELFLAG=""
[ "$PROFILE" = release ] && RELFLAG="--release"
CARGO_TARGET_DIR="$HOME/waly-target-wsl-tauri" cargo build $RELFLAG \
  -p waly-seal --bin waly-seal-svc --target x86_64-pc-windows-gnu
SVC="$TDIR/waly-seal-svc.exe"
[ -f "$SVC" ] || { echo "service absent: $SVC"; exit 1; }

NSISDIR="$HOME/nsis-local/usr/share/nsis" "$HOME/nsis-local/usr/bin/makensis" \
  -DEXE_PATH="$EXE" \
  -DLOADER_PATH="$LOADER" \
  -DSVC_PATH="$SVC" \
  -DICON_PATH="$REPO/apps/desktop/src-tauri/icons/icon.ico" \
  -DOUT_PATH="$OUT" \
  "$REPO/apps/desktop/installer/installer.nsi"

ls -la "$OUT"
echo "OK: $OUT"
