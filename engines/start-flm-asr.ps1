# Demarre FastFlowLM en mode ASR STANDALONE (Whisper-V3-Turbo sur NPU).
# PIEGE v0.9.43 (mesure 2026-07-03) : /v1/audio/transcriptions ne fonctionne
# QUE dans ce mode. Si un LLM est co-charge (flm serve <llm> --asr 1), le
# endpoint repond "null" HTTP 200 sans rien faire.
# Le LLM tourne donc dans un SECOND processus flm (start-flm.ps1, autre port).
# Cohabitation NPU mesuree : OK, serialisation ~additive en simultane.
# API : POST /v1/audio/transcriptions (multipart: file=@x.wav, model=whisper-v3:turbo)
param(
    [int]$Port = 52625,
    [string]$PMode = "performance"
)
$flmRoot = Get-ChildItem "C:\waly\engines\flm" -Directory | Select-Object -First 1
$flm = Join-Path $flmRoot.FullName "flm.exe"
if (-not (Test-Path $flm)) { throw "flm.exe introuvable sous C:\waly\engines\flm" }
& $flm serve --asr 1 --port $Port --pmode $PMode
