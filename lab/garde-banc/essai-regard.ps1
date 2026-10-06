# Essai 2 (Garde, etape 3) : le lecteur EN DIRECT du service (regard.rs),
# lance ici dans une console elevee, sans installer le service.
# Deux passes : « agents » (un faux agent designe par son programme), puis
# « tout » (toute la machine). Resultat : resultat-regard.txt. ASCII pur.
param([switch]$Eleve, [string]$Svc = 'C:\waly\data\garde-essai\waly-seal-svc.exe')
$ErrorActionPreference = 'Stop'
$ici = Split-Path -Parent $MyInvocation.MyCommand.Path
$res = Join-Path $ici 'resultat-regard.txt'

if (-not $Eleve) {
  Remove-Item $res -ErrorAction SilentlyContinue
  try {
    Start-Process powershell.exe -Verb RunAs -Wait -WindowStyle Hidden -ArgumentList @(
      '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', ('"' + $MyInvocation.MyCommand.Path + '"'), '-Eleve', '-Svc', ('"' + $Svc + '"'))
  } catch {
    ('REFUS OU ECHEC UAC : ' + $_.Exception.Message) | Out-File $res -Encoding ascii
  }
  if (Test-Path $res) { Get-Content $res } else { 'aucun resultat ecrit' }
  exit
}

$out = New-Object System.Collections.Generic.List[string]
try {
  $ps = "$env:SystemRoot\System32\WindowsPowerShell\v1.0\powershell.exe"
  $cible = 'C:\waly\data\garde-essai-ecrit.txt'
  $gestes = "Start-Sleep -Milliseconds 800; `$null = Get-Content 'C:\waly\README.md'; Set-Content '$cible' 'essai'; " +
            "Start-Process ping.exe -ArgumentList '-n','1','127.0.0.1' -WindowStyle Hidden -Wait; " +
            "try { `$c = New-Object Net.Sockets.TcpClient; `$c.Connect('1.1.1.1',443); `$c.Close() } catch {}; Start-Sleep -Milliseconds 500"

  foreach ($passe in @('agents', 'tout')) {
    $sortie = "C:\waly\data\garde-essai\sortie-$passe.txt"
    $erreur = "C:\waly\data\garde-essai\erreur-$passe.txt"
    Remove-Item $sortie, $erreur, $cible -ErrorAction SilentlyContinue
    $args = if ($passe -eq 'agents') { @('essai-regard', '9', $ps) } else { @('essai-regard', '9') }
    $e = Start-Process $Svc -ArgumentList $args -PassThru -WindowStyle Hidden -RedirectStandardOutput $sortie -RedirectStandardError $erreur
    Start-Sleep -Milliseconds 2500
    $a = Start-Process $ps -PassThru -WindowStyle Hidden -ArgumentList @('-NoProfile', '-Command', $gestes)
    $null = $a.WaitForExit(15000)
    $null = $e.WaitForExit(20000)
    $l = @(Get-Content $sortie -ErrorAction SilentlyContinue)
    $err = (Get-Content $erreur -ErrorAction SilentlyContinue) -join ' '
    $out.Add("=== passe $passe : code $($e.ExitCode) ; faux agent PID $($a.Id) ; $($l.Count) lignes $err")
    $obs = @($l | Where-Object { $_ -match "`t" })
    $out.Add("  " + ($l | Select-Object -First 2 | ForEach-Object { $_ }) -join ' | ')
    $out.Add("  par genre : " + (($obs | ForEach-Object { ($_ -split "`t")[0] } | Group-Object | ForEach-Object { "$($_.Name)=$($_.Count)" }) -join ' '))
    $out.Add("  par programme : " + (($obs | ForEach-Object { ($_ -split "`t")[2] } | Group-Object | Sort-Object Count -Descending | Select-Object -First 8 | ForEach-Object { "$($_.Name)=$($_.Count)" }) -join ' '))
    $deLui = @($obs | Where-Object { ($_ -split "`t")[1] -eq "$($a.Id)" -or $passe -eq 'agents' })
    $out.Add("  GESTE 1 lecture README.md : " + $(if ($deLui | Where-Object { $_ -match 'ouvert.*\\waly\\README\.md' }) { 'VU' } else { 'PAS VU' }))
    $out.Add("  GESTE 2 fichier ecrit    : " + $(if ($deLui | Where-Object { $_ -match '(cree|ecrit).*garde-essai-ecrit\.txt' }) { 'VU' } else { 'PAS VU' }))
    $out.Add("  GESTE 3 ping lance       : " + $(if ($deLui | Where-Object { $_ -match '^lance.*PING\.EXE' }) { 'VU' } else { 'PAS VU' }))
    $out.Add("  GESTE 4 connexion 1.1.1.1: " + $(if ($deLui | Where-Object { $_ -match '^connecte.*1\.1\.1\.1:443' }) { 'VU' } else { 'PAS VU' }))
    if ($passe -eq 'agents') { foreach ($x in ($obs | Select-Object -First 25)) { $out.Add("    " + ($x -replace "`t", ' | ')) } }
    else { foreach ($x in ($obs | Where-Object { ($_ -split "`t")[0] -ne 'ouvert' } | Select-Object -First 14)) { $out.Add("    " + ($x -replace "`t", ' | ')) } }
  }
} catch {
  $out.Add('ERREUR : ' + $_.Exception.Message)
} finally {
  cmd /c "logman stop WalyGardeRegard -ets >nul 2>&1"
  Remove-Item 'C:\waly\data\garde-essai-ecrit.txt' -ErrorAction SilentlyContinue
  $out | Out-File $res -Encoding ascii
}
