# Essai 4 (Garde, etape 4) : le CODE de l'enclos (waly_core::enclos), par le
# CLI `waly enclos`, sans elevation. Une seule demande UAC : la creation du
# compte WalyEnclos par le programme du service (bati, pas installe).
# Le compte RESTE apres l'essai : c'est celui de l'app. Pour le retirer :
#   waly-seal-svc enclos supprimer   (console elevee)
# Resultat : resultat-enclos-code.txt. ASCII pur.
param([string]$Bin = 'C:\waly\data\enclos-essai', [switch]$SansCreer)
$ErrorActionPreference = 'Continue'
$ici = Split-Path -Parent $MyInvocation.MyCommand.Path
$res = Join-Path $ici 'resultat-enclos-code.txt'
$waly = Join-Path $Bin 'waly.exe'
$svc = Join-Path $Bin 'waly-seal-svc.exe'
$out = New-Object System.Collections.Generic.List[string]
function W([string]$titre, [string[]]$a) {
  $t = [Diagnostics.Stopwatch]::StartNew()
  $r = & $waly enclos @a 2>&1 | ForEach-Object { "$_" }
  $out.Add("--- $titre ($($t.ElapsedMilliseconds) ms)")
  foreach ($l in $r) { $out.Add("    $l") }
}
$dans = Join-Path $env:USERPROFILE 'waly-enclos-essai'
$hors = 'C:\waly\lab\garde-banc\tmp-enclos'
try {
  $null = New-Item -ItemType Directory -Force (Join-Path $dans 'prive'), (Join-Path $dans 'prog'), $hors
  'x' | Out-File (Join-Path $dans 'f.txt'); 'x' | Out-File (Join-Path $hors 'f.txt')
  Copy-Item "$env:SystemRoot\System32\PING.EXE" (Join-Path $dans 'prog\agent.exe') -Force
  if (-not $SansCreer) { W 'creer (UAC)' @('creer', $svc) }
  W 'etat apres creation' @()
  W 'essai general (profil + dossiers)' @('essai')
  W 'dans le profil : donner en lecture' @('donner', $dans)
  W 'dans le profil : passer en ecriture (le refus d ecrire doit partir)' @('donner', $dans, 'ecriture')
  W 'sous-dossier d un dossier donne : couper' @('couper', (Join-Path $dans 'prive'))
  W 'dans le profil : repasser en lecture' @('donner', $dans)
  W 'hors profil : donner en lecture (doit REFUSER l ecriture)' @('donner', $hors)
  W 'hors profil : couper' @('couper', $hors)
  W 'hors profil : ecriture apres coupure (la coupure doit partir)' @('donner', $hors, 'ecriture')
  W 'refus attendu : tout le profil' @('donner', $env:USERPROFILE)
  W 'refus attendu : la memoire de Waly' @('donner', 'C:\waly\data')
  W 'refus attendu : un lecteur' @('couper', 'C:\')
  W 'reprendre le sous-dossier' @('reprendre', (Join-Path $dans 'prive'))
  W 'reprendre le dossier du profil' @('reprendre', $dans)
  W 'reprendre hors profil' @('reprendre', $hors)
  $acl = (& icacls $dans 2>&1 | Out-String) + (& icacls $hors 2>&1 | Out-String)
  $out.Add('--- apres reprise, reste-t-il une entree WalyEnclos sur les dossiers ? ' + $(if ($acl -match 'WalyEnclos') { 'OUI (defaut)' } else { 'non' }))
  W 'lancer un programme systeme (ping 40 s)' @('lancer', "$env:SystemRoot\System32\PING.EXE", '-n', '40', '127.0.0.1')
  Start-Sleep -Milliseconds 1500
  W 'etat : ses processus' @()
  $p = Get-CimInstance Win32_Process -Filter "Name='PING.EXE'" | Select-Object -First 1
  $proprio = if ($p) { (Invoke-CimMethod -InputObject $p -MethodName GetOwner -ErrorAction SilentlyContinue).User } else { '' }
  $out.Add("--- le ping tourne sous le compte : '$proprio' (vide = illisible sans elevation) ; PID $($p.ProcessId)")
  W 'arreter' @('arreter', "$env:SystemRoot\System32\PING.EXE")
  Start-Sleep -Milliseconds 600
  $out.Add('--- ping encore la apres arret ? ' + $(if (Get-Process -Name PING -ErrorAction SilentlyContinue) { 'OUI (defaut)' } else { 'non' }))
  W 'lancer un programme DU PROFIL (son dossier doit etre donne d office)' @('lancer', (Join-Path $dans 'prog\agent.exe'), '-n', '20', '127.0.0.1')
  Start-Sleep -Milliseconds 1200
  W 'etat' @()
  W 'arreter' @('arreter', (Join-Path $dans 'prog\agent.exe'))
  W 'reprendre le dossier du programme' @('reprendre', (Join-Path $dans 'prog'))
  W 'etat final' @()
} catch {
  $out.Add('ERREUR : ' + $_.Exception.Message)
} finally {
  Remove-Item $dans -Recurse -Force -ErrorAction SilentlyContinue
  Remove-Item $hors -Recurse -Force -ErrorAction SilentlyContinue
  $out | Out-File $res -Encoding utf8
}
Get-Content $res -Encoding utf8
