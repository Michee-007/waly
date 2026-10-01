# Lance banc-c.ps1 en ELEVE (UAC) et capture toute sa sortie dans banc-c.log,
# que le parent (console standard) affiche ensuite.
$here = $PSScriptRoot
$log  = Join-Path $here "banc-c.log"
$banc = Join-Path $here "banc-c.ps1"

$id = [Security.Principal.WindowsIdentity]::GetCurrent()
$pr = New-Object Security.Principal.WindowsPrincipal($id)
$eleve = $pr.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)

if (-not $eleve) {
    if (Test-Path $log) { Remove-Item $log }
    Write-Host "Invite UAC (accepter) pour la sequence complete du banc..."
    Start-Process powershell -Verb RunAs -Wait -WindowStyle Hidden -ArgumentList `
        "-NoProfile","-ExecutionPolicy","Bypass","-Command","& '$banc' *>&1 | Tee-Object -FilePath '$log'"
    Start-Sleep -Milliseconds 400
    if (Test-Path $log) { Get-Content $log } else { Write-Host "PAS DE LOG (UAC refuse ?)" }
    exit
}
& $banc
