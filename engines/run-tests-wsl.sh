#!/usr/bin/env bash
# Tests workspace sur l'hote WSL (ELF, hors SAC) avec bilan lisible.
# Usage : wsl.exe -e bash /mnt/c/waly/engines/run-tests-wsl.sh
set -uo pipefail
# bash non-login (wsl.exe -e bash script) : cargo n'est pas dans le PATH.
source "$HOME/.cargo/env" 2>/dev/null || export PATH="$HOME/.cargo/bin:$PATH"
cd /mnt/c/waly
out=$(CARGO_TARGET_DIR="$HOME/waly-target-wsl-host" cargo test --workspace 2>&1) && code=0 || code=$?
echo "$out" | grep -E "test result:" | sort | uniq -c
if [ $code -ne 0 ] || echo "$out" | grep -q "FAILED"; then
  echo "=== ECHECS (code $code) ==="
  echo "$out" | grep -B2 -A8 "FAILED" | head -40
  echo "--- dernieres lignes ---"
  echo "$out" | tail -15
  exit 1
fi
echo "TOUT VERT"
