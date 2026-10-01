# Installe le NOUVEAU service scelleur (chantier C) a la place de l'ancien.
# AUTO-ELEVATION : lance-le depuis n'importe quelle console, il declenche l'UAC
# lui-meme (accepter). Il remplace C:\Program Files\Waly\waly-seal-svc.exe par
# la version chantier C (garde admin + scelle d'agent tiers), re-scelle le
# perimetre au demarrage.  (Revenir a l'ancien : reinstaller via l'installeur
# Waly, ou setup avec l'ancien exe.)
$ErrorActionPreference = "Continue"
$rel = Join-Path $PSScriptRoot "waly-seal-svc-c-release.exe"
$log = Join-Path $PSScriptRoot "install-c.log"

$id = [Security.Principal.WindowsIdentity]::GetCurrent()
$pr = New-Object Security.Principal.WindowsPrincipal($id)
$eleve = $pr.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)

if (-not $eleve) {
    Write-Host "Elevation requise - invite UAC (accepter)..."
    if (Test-Path $log) { Remove-Item $log }
    Start-Process powershell -Verb RunAs -Wait -WindowStyle Hidden -ArgumentList `
        "-NoProfile","-ExecutionPolicy","Bypass","-File","`"$PSCommandPath`""
    Start-Sleep -Milliseconds 400
    if (Test-Path $log) { Get-Content $log } else { Write-Host "PAS DE LOG (UAC refuse ?)" }
    $s = Get-Service WalySeal -ErrorAction SilentlyContinue
    Write-Host ("WalySeal: " + $(if ($s) { $s.Status } else { "ABSENT" }))
    exit
}

# --- branche elevee : setup relocalise dans Program Files, AUTO_START, start ---
"setup (remplace l'ancien service)..." | Out-File $log -Encoding utf8
& $rel setup 2>&1 | Out-File $log -Encoding utf8 -Append
Start-Sleep -Milliseconds 500
"---- sc query ----" | Out-File $log -Encoding utf8 -Append
sc.exe query WalySeal 2>&1 | Out-File $log -Encoding utf8 -Append
sc.exe qc WalySeal 2>&1 | Out-File $log -Encoding utf8 -Append
