# Banc chantier C - « Huis clos universel » : sceller n'importe quel agent.
# Prouve : (1) scelle d'un agent TIERS temoin (copie de curl.exe) -> sortie
# reseau bloquee, loopback intact ; (2) garde ADMIN (seal tiers refuse en
# compte standard) ; (3) latence pose/levee ; (4) journal d'audit.
#
# PREREQUIS : le NOUVEAU service (chantier C) doit tourner. Installer d'abord :
#   .\install-c-eleve.ps1   (UAC une fois)
#
# Lancer ce banc depuis une console ELEVEE pour la partie seal/unseal, et il
# relance lui-meme un seal en STANDARD pour prouver le refus.
$ErrorActionPreference = "Continue"
$here = $PSScriptRoot
$svc  = Join-Path $here "waly-seal-svc-c.exe"   # client CLI (nouveau)
$agent = Join-Path $here "agent-temoin.exe"      # agent tiers temoin

# Agent temoin = copie de curl.exe (exe tiers, signe MS -> passe SAC). Chemin
# distinct de System32 -> app-id WFP distinct : on ne scelle QUE ce chemin.
if (-not (Test-Path $agent)) {
    Copy-Item "$env:WINDIR\System32\curl.exe" $agent -Force
}

function Test-Sortie($exe, $label) {
    # Sortie reseau reelle (TCP 443 vers une IP publique neutre), timeout court.
    & $exe -s -m 6 -o NUL https://1.1.1.1 2>$null
    $ok = ($LASTEXITCODE -eq 0)
    Write-Host ("  {0}: sortie 1.1.1.1:443 = {1}" -f $label, $(if ($ok) {"REUSSIE (exit 0)"} else {"BLOQUEE (exit $LASTEXITCODE)"}))
    return $ok
}

$id = [Security.Principal.WindowsIdentity]::GetCurrent()
$pr = New-Object Security.Principal.WindowsPrincipal($id)
$eleve = $pr.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
Write-Host ("== Banc C == console elevee: {0}" -f $eleve)
Write-Host ("etat service:")
& $svc list

Write-Host "`n[0] Reference : l'agent temoin sort-il AVANT scelle ?"
$avant = Test-Sortie $agent "avant"

if ($eleve) {
    Write-Host "`n[1] Scelle de l'agent tiers (console elevee) :"
    $t = Measure-Command { & $svc seal $agent } | Select-Object -ExpandProperty TotalMilliseconds
    & $svc seal $agent | Out-Null
    Write-Host ("  pose ~ {0:N1} ms" -f $t)

    Write-Host "`n[2] L'agent sort-il APRES scelle ? (doit etre BLOQUEE)"
    $apres = Test-Sortie $agent "apres"

    Write-Host "`n[3] Le vrai curl (System32, NON scelle) sort-il toujours ? (loopback/tiers intact)"
    Test-Sortie "$env:WINDIR\System32\curl.exe" "curl-systeme" | Out-Null

    Write-Host "`n[4] Journal d'audit de l'agent :"
    & $svc journal $agent

    Write-Host "`n[5] Levee du scelle :"
    $t2 = Measure-Command { & $svc unseal $agent } | Select-Object -ExpandProperty TotalMilliseconds
    Write-Host ("  levee ~ {0:N1} ms" -f $t2)
    Start-Sleep -Milliseconds 300
    Write-Host "`n[6] L'agent re-sort-il APRES levee ? (doit etre REUSSIE)"
    Test-Sortie $agent "apres-levee" | Out-Null

    Write-Host "`n== VERDICT =="
    Write-Host ("  avant={0} apres-scelle={1} (attendu: True puis False)" -f $avant, $apres)
} else {
    Write-Host "`n[1] Garde ADMIN : tenter de sceller un agent tiers en compte STANDARD (doit etre REFUSE) :"
    & $svc seal $agent
    Write-Host "`n(relance ce banc dans une console ELEVEE pour la sequence complete)"
}
