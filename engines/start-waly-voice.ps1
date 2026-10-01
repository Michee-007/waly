# Lance Waly voix en une commande : moteur LLM FLM (si absent) + waly-voice talk.
# Usage : powershell -File C:\waly\engines\start-waly-voice.ps1
param(
    # Meme nom que waly_core::llm::modele_par_defaut() : un nom different =
    # hot-swap FLM = serveur MORT (piege R4).
    [string]$Model = "qwen3vl-it:4b",
    # Port de l'app, hors zone dynamique Windows (piege 10 de CLAUDE.md).
    [int]$LlmPort = 42626,
    [int]$CtxLen = 4096,
    # Voix : "homme" (fabien, defaut) ou "femme" (developpeuse) - verdict
    # Michee 2026-07-20. En secours piper : homme=pierre, femme=jessica.
    [string]$Voix = "homme",
    # Moteur TTS : "pocket" (french_24l STREAMING, LA voix de Waly - verdict
    # Michee 2026-07-20) ou "piper" (secours, juge trop robotique).
    [string]$Tts = "pocket",
    # WAV de reference pour le clonage pocket (defaut : selon -Voix).
    [string]$TtsRef = "",
    # Graine pocket : defaut 42 dans le binaire (l'identite jugee a l'A/B).
    # Passer 0 pour un tirage libre.
    [string]$TtsSeed = ""
)
if ($Tts -eq "piper") {
    $env:WALY_TTS = "piper"
} else {
    Remove-Item Env:WALY_TTS -ErrorAction SilentlyContinue
    if ($TtsRef -ne "") { $env:WALY_TTS_REF = $TtsRef }
    else { Remove-Item Env:WALY_TTS_REF -ErrorAction SilentlyContinue }
    if ($TtsSeed -ne "") { $env:WALY_POCKET_SEED = $TtsSeed }
    else { Remove-Item Env:WALY_POCKET_SEED -ErrorAction SilentlyContinue }
}
# WALY_TTS_SPEAKER=0 = feminine, pour les DEUX moteurs (pocket:
# developpeuse, piper: jessica).
if ($Voix -eq "femme") { $env:WALY_TTS_SPEAKER = "0" }
else { Remove-Item Env:WALY_TTS_SPEAKER -ErrorAction SilentlyContinue }
$exe = "C:\waly\bin\waly-voice.exe"
if (-not (Test-Path $exe)) { $exe = "C:\waly\engines\voice-bench\waly-voice-dbg.exe" }
if (-not (Test-Path $exe)) { throw "waly-voice.exe introuvable - compiler d'abord (voir CLAUDE.md)" }

# Le LLM tourne-t-il deja ?
$up = $false
try {
    $r = Invoke-WebRequest -UseBasicParsing -Uri "http://127.0.0.1:$LlmPort/v1/models" -TimeoutSec 2
    $up = ($r.StatusCode -eq 200)
} catch {}

if (-not $up) {
    Write-Host "Demarrage du moteur LLM ($Model, port $LlmPort)..."
    $flmRoot = Get-ChildItem "C:\waly\engines\flm" -Directory | Select-Object -First 1
    $flm = Join-Path $flmRoot.FullName "flm.exe"
    # --pmode turbo : -0,07 s de TTFB mesure (2026-07-04). Ne PAS ajouter
    # --prefill-chunk-len : crash de flm v0.9.43 sur les prompts longs (vecu).
    Start-Process -FilePath $flm -ArgumentList 'serve',$Model,'--ctx-len',"$CtxLen",'--port',"$LlmPort",'--pmode','turbo' -WindowStyle Hidden
    $deadline = (Get-Date).AddSeconds(60)
    while ((Get-Date) -lt $deadline) {
        Start-Sleep -Seconds 2
        try {
            $r = Invoke-WebRequest -UseBasicParsing -Uri "http://127.0.0.1:$LlmPort/v1/models" -TimeoutSec 2
            if ($r.StatusCode -eq 200) { $up = $true; break }
        } catch {}
    }
    if (-not $up) { throw "le moteur LLM n'a pas demarre en 60 s (voir les fenetres flm)" }
}

# waly-voice fait son propre warmup LLM+TTS au demarrage.
& $exe talk
