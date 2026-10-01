# Preuve terrain du sceau (a lancer en COMPTE STANDARD, sans elevation, une
# fois le service installe par install-eleve.ps1). Prouve la chaine complete :
# client user-level -> service SYSTEM -> WFP bloque la sortie, loopback vit,
# tentative journalisee.
#
# Victime = huisclos-banc.exe (reutilise du banc) : on le scelle, puis son
# 'sonder' doit montrer loopback OK + sorties bloquees (os 10013).
#
# Piege PowerShell 5.1 : les guillemets d'un argument JSON passe a un exe natif
# sont manges -> on echappe en \" via .Replace avant l'appel.
$ErrorActionPreference = "Continue"
$svc  = Join-Path $PSScriptRoot "waly-seal-svc.exe"
$banc = Join-Path $PSScriptRoot "huisclos-banc.exe"

function Probe([string]$json) { & $svc probe $json.Replace('"','\"') }

Write-Host "0) etat du service"
Probe '{"cmd":"etat"}'

Write-Host "`n1) sondes AVANT sceau (baseline : tout passe)"
& $banc sonder

Write-Host "`n2) SCELLER huisclos-banc.exe (session 1) via le pipe"
$exeJson = $banc.Replace('\','\\')
$t0 = Get-Date
Probe ('{"cmd":"sceller","session":1,"exes":["' + $exeJson + '"]}')
Write-Host ("   pose demandee en " + [int]((Get-Date)-$t0).TotalMilliseconds + " ms (aller-retour pipe)")

Write-Host "`n3) sondes SOUS sceau (attendu : loopback OK, sorties os 10013)"
& $banc sonder

Write-Host "`n4) JOURNAL d'audit de la session (tentatives bloquees vues par WFP)"
Probe '{"cmd":"journal","session":1}'

Write-Host "`n5) DESCELLER"
Probe '{"cmd":"desceller","session":1}'
Write-Host "   sondes APRES levee (attendu : retour normal)"
& $banc sonder
