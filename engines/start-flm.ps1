# Demarre FastFlowLM (NPU XDNA2) - moteur prioritaire de Waly.
# Prerequis : driver NPU >= 32.0.203.311 (verifier : flm validate).
# API OpenAI-compat sur http://127.0.0.1:42626/v1 (port de l'app, WALY_LLM_PORT).
# ASCII PUR dans ce fichier (piege 2 : PS 5.1 lit l'UTF-8 sans BOM en 1252).
param(
    [string]$Model = "qwen3vl-it:4b",
    # 42626 = port de l'app, HORS zone dynamique Windows (49152-65535) ou
    # WinNAT reserve des plages au demarrage de WSL (vecu 2026-09-10 :
    # 52579-52678 reserve -> bind 10013).
    [int]$Port = 42626,
    [string]$PMode = "performance",
    # CRITIQUE sur 15-16 Go : le ctx par defaut de FLM (32k) fait ~4,5 Go de KV -> le
    # gestionnaire de memoire video echoue a paginer (erreur 0xc01e0200, verifie
    # 2026-07-03). 8192 = ~1,1 Go de KV, charge fiable avec ~7 Go libres.
    [int]$CtxLen = 8192
)
$flmRoot = Get-ChildItem "C:\waly\engines\flm" -Directory | Select-Object -First 1
$flm = Join-Path $flmRoot.FullName "flm.exe"
if (-not (Test-Path $flm)) { throw "flm.exe introuvable sous C:\waly\engines\flm" }
& $flm validate
& $flm serve $Model --port $Port --pmode $PMode --ctx-len $CtxLen
