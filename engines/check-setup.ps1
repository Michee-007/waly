# Verifie l'environnement complet Waly cote Windows. Usage (depuis WSL) :
#   powershell.exe -File C:\waly\engines\check-setup.ps1
$ErrorActionPreference = 'SilentlyContinue'
Write-Host "=== Waly check-setup ==="

# 1. NPU + driver
$npu = Get-CimInstance Win32_PnPSignedDriver | Where-Object { $_.DeviceName -like '*NPU Compute*' } | Select-Object -First 1
if ($npu) { Write-Host ("[OK] NPU driver " + $npu.DriverVersion + " (requis >= 32.0.203.311)") }
else { Write-Host "[!!] NPU introuvable" }

# 2. FastFlowLM
$flmRoot = Get-ChildItem 'C:\waly\engines\flm' -Directory | Select-Object -First 1
$flm = if ($flmRoot) { Join-Path $flmRoot.FullName 'flm.exe' } else { $null }
if ($flm -and (Test-Path $flm)) { Write-Host ("[OK] flm.exe : " + (& $flm --version 2>&1 | Select-Object -First 1)) }
else { Write-Host "[!!] flm.exe absent - reinstaller: zip portable github.com/FastFlowLM/FastFlowLM -> C:\waly\engines\flm" }

# 3. Serveur NPU
try {
    $m = Invoke-RestMethod 'http://127.0.0.1:52625/v1/models' -TimeoutSec 4
    Write-Host ("[OK] Moteur A (NPU) up sur :52625 - " + $m.data.Count + " modeles au catalogue")
} catch { Write-Host "[--] Moteur A down. Lancer : powershell -File start-flm.ps1  (qwen3:4b, ctx 8192)" }

# 4. Ollama Vulkan (moteur B)
try {
    $v = Invoke-RestMethod 'http://127.0.0.1:11434/api/version' -TimeoutSec 4
    Write-Host ("[OK] Moteur B (Ollama " + $v.version + ") up sur :11434")
} catch { Write-Host "[--] Ollama down (fallback seulement, non bloquant)" }
foreach ($k in 'OLLAMA_VULKAN','OLLAMA_IGPU_ENABLE','OLLAMA_KV_CACHE_TYPE','OLLAMA_FLASH_ATTENTION') {
    $val = [Environment]::GetEnvironmentVariable($k, 'User')
    if ($val) { Write-Host ("[OK] env $k=$val") } else { Write-Host ("[!!] env $k manquante (voir engines/README.md)") }
}

# 5. RAM - le NPU a besoin de marge pour paginer (viser >= 7 Go libres au chargement)
$os = Get-CimInstance Win32_OperatingSystem
Write-Host ("[i ] RAM libre : " + [math]::Round($os.FreePhysicalMemory/1MB,1) + " Go / " + [math]::Round($os.TotalVisibleMemorySize/1MB,1))

# 6. Modeles GGUF (moteur B signe, plus tard)
if (Test-Path 'C:\waly\engines\models\Qwen3VL-4B-Instruct-Q4_K_M.gguf') { Write-Host "[OK] Qwen3-VL-4B GGUF present" }
else { Write-Host "[--] Qwen3-VL-4B GGUF absent (necessaire seulement pour llama-server signe)" }

Write-Host "=== fin check-setup ==="
