# Installe et demarre le service scelleur WalySeal (R6a Huis clos).
# AUTO-ELEVATION : lance-le depuis n'importe quelle console PowerShell, il
# declenche l'invite UAC lui-meme (accepter), puis affiche le resultat ici.
# Desinstaller : waly-seal-svc.exe uninstall (eleve).
$ErrorActionPreference = "Continue"
$svc = Join-Path $PSScriptRoot "waly-seal-svc.exe"
$log = Join-Path $PSScriptRoot "svc-install.log"

$id = [Security.Principal.WindowsIdentity]::GetCurrent()
$pr = New-Object Security.Principal.WindowsPrincipal($id)
$eleve = $pr.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)

if (-not $eleve) {
    Write-Host "Elevation requise - invite UAC (accepter)..."
    if (Test-Path $log) { Remove-Item $log }
    Start-Process powershell -Verb RunAs -Wait -WindowStyle Hidden -ArgumentList `
        "-NoProfile","-ExecutionPolicy","Bypass","-File","`"$PSCommandPath`""
    Start-Sleep -Milliseconds 300
    if (Test-Path $log) { Get-Content $log } else { Write-Host "PAS DE LOG (UAC refuse ?)" }
    $s = Get-Service WalySeal -ErrorAction SilentlyContinue
    Write-Host ("WalySeal: " + $(if ($s) { $s.Status } else { "ABSENT" }))
    if ($s -and $s.Status -eq "Running") {
        Write-Host "etat (via pipe, compte standard):"
        & $svc probe '{"cmd":"etat"}'
    }
    exit
}

# --- branche elevee : ecrit tout dans le log, le parent l'affiche ---
"install..."            | Out-File $log -Encoding utf8
& $svc install 2>&1     | Out-File $log -Encoding utf8 -Append
"start..."              | Out-File $log -Encoding utf8 -Append
& $svc start 2>&1       | Out-File $log -Encoding utf8 -Append
