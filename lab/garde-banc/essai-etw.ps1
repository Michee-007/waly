# Essai de faisabilite (Garde, etape 3) : peut-on VOIR ce qu'un autre agent
# touche (fichiers, programmes lances, connexions) avec le suivi d'evenements
# de Windows (ETW), et a quel cout ?
#
# Le script s'eleve (une demande UAC), ouvre une session de suivi sur trois
# sources du noyau, lance un faux agent qui fait quatre gestes CONNUS, arrete,
# puis cherche ces quatre gestes dans la trace. Resultat : resultat.txt.
# ASCII pur (piege 2). Ne laisse rien derriere lui.
param([switch]$Eleve)
$ErrorActionPreference = 'Stop'
$ici = Split-Path -Parent $MyInvocation.MyCommand.Path
$res = Join-Path $ici 'resultat.txt'

if (-not $Eleve) {
  Remove-Item $res -ErrorAction SilentlyContinue
  try {
    Start-Process powershell.exe -Verb RunAs -Wait -WindowStyle Hidden -ArgumentList @(
      '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', ('"' + $MyInvocation.MyCommand.Path + '"'), '-Eleve')
  } catch {
    ('REFUS OU ECHEC UAC : ' + $_.Exception.Message) | Out-File $res -Encoding ascii
  }
  if (Test-Path $res) { Get-Content $res } else { 'aucun resultat ecrit' }
  exit
}

$out = New-Object System.Collections.Generic.List[string]
function Dire($t) { $out.Add($t) }
try {
  $nom = 'waly-garde-essai'
  $etl = 'C:\waly\data\garde-essai.etl'
  $cible = 'C:\waly\data\garde-essai-ecrit.txt'
  cmd /c "logman stop $nom -ets >nul 2>&1"
  Remove-Item $etl, $cible -ErrorAction SilentlyContinue

  # Fichiers : noms + creations + ecritures + suppressions + renommages + nouveaux fichiers.
  cmd /c "logman start $nom -p Microsoft-Windows-Kernel-File 0x1E90 0x4 -o `"$etl`" -ets -bs 256 -nb 64 256 >nul 2>&1"
  if ($LASTEXITCODE -ne 0) { throw "logman start a echoue (code $LASTEXITCODE)" }
  cmd /c "logman update trace $nom -p Microsoft-Windows-Kernel-Process 0x10 0x4 -ets >nul 2>&1"
  $okProc = ($LASTEXITCODE -eq 0)
  cmd /c "logman update trace $nom -p Microsoft-Windows-Kernel-Network 0x30 0x4 -ets >nul 2>&1"
  $okNet = ($LASTEXITCODE -eq 0)

  # Le faux agent : quatre gestes connus.
  $gestes = "Start-Sleep -Milliseconds 1500; `$null = Get-Content 'C:\waly\README.md'; Set-Content '$cible' 'essai'; " +
            "Start-Process ping.exe -ArgumentList '-n','1','127.0.0.1' -WindowStyle Hidden -Wait; " +
            "try { `$c = New-Object Net.Sockets.TcpClient; `$c.Connect('1.1.1.1',443); `$c.Close() } catch {}; Start-Sleep -Milliseconds 800"
  $t0 = Get-Date
  $p = Start-Process powershell.exe -PassThru -WindowStyle Hidden -ArgumentList @('-NoProfile', '-Command', $gestes)
  $agent = $p.Id
  $null = $p.WaitForExit(25000)
  Start-Sleep -Milliseconds 800
  $duree = ((Get-Date) - $t0).TotalSeconds
  cmd /c "logman stop $nom -ets >nul 2>&1"

  $taille = (Get-Item $etl).Length
  $t1 = Get-Date
  $ev = @(Get-WinEvent -Path $etl -Oldest -ErrorAction SilentlyContinue)
  $lecture = ((Get-Date) - $t1).TotalSeconds

  Dire ("session : fichiers=oui processus=$okProc reseau=$okNet")
  Dire ("duree de capture : {0:N1} s ; trace : {1:N0} Ko ; evenements : {2}" -f $duree, ($taille / 1KB), $ev.Count)
  Dire ("debit systeme entier : {0:N0} evenements/s ; relecture PowerShell : {1:N1} s" -f ($ev.Count / [Math]::Max($duree, 0.1)), $lecture)
  foreach ($g in ($ev | Group-Object ProviderName)) { Dire ("  {0} : {1}" -f $g.Name, $g.Count) }
  $fic = $ev | Where-Object { $_.ProviderName -eq 'Microsoft-Windows-Kernel-File' }
  Dire ("  fichiers par type : " + (($fic | Group-Object Id | Sort-Object Count -Descending | ForEach-Object { "id$($_.Name)=$($_.Count)" }) -join ' '))

  function Champs($e) {
    $h = @{}
    try { $x = [xml]$e.ToXml(); foreach ($d in $x.Event.EventData.Data) { if ($d.Name) { $h[$d.Name] = $d.'#text' } } } catch {}
    $h
  }

  # 1. fichiers touches par l'agent (l'en-tete de l'evenement porte son PID)
  $deAgent = @($fic | Where-Object { $_.ProcessId -eq $agent })
  Dire ("agent PID $agent : $($deAgent.Count) evenements de fichiers")
  $noms = @{}
  foreach ($e in ($deAgent | Select-Object -First 4000)) {
    $c = Champs $e
    $n = $c['FileName']
    if ($n) { if (-not $noms.ContainsKey($n)) { $noms[$n] = New-Object System.Collections.Generic.List[int] }; if (-not $noms[$n].Contains($e.Id)) { $noms[$n].Add($e.Id) } }
  }
  Dire ("  fichiers distincts nommes : $($noms.Count)")
  $vuLecture = @($noms.Keys | Where-Object { $_ -like '*\waly\README.md' })
  $vuEcrit = @($noms.Keys | Where-Object { $_ -like '*garde-essai-ecrit.txt' })
  Dire ("  GESTE 1 lecture de README.md : " + $(if ($vuLecture.Count) { 'VU (' + $vuLecture[0] + ' ; types ' + ($noms[$vuLecture[0]] -join ',') + ')' } else { 'PAS VU' }))
  Dire ("  GESTE 2 ecriture d'un fichier : " + $(if ($vuEcrit.Count) { 'VU (' + $vuEcrit[0] + ' ; types ' + ($noms[$vuEcrit[0]] -join ',') + ')' } else { 'PAS VU' }))
  $hors = @($noms.Keys | Where-Object { $_ -notlike '*\Windows\*' -and $_ -notlike '*\Program Files*' } | Select-Object -First 8)
  Dire ("  exemples hors Windows : " + ($hors -join ' | '))

  # 2. programmes lances par l'agent
  $lances = @()
  foreach ($e in ($ev | Where-Object { $_.ProviderName -eq 'Microsoft-Windows-Kernel-Process' -and $_.Id -eq 1 })) {
    $c = Champs $e
    if ("$($c['ParentProcessID'])" -eq "$agent") { $lances += $c['ImageName'] }
  }
  Dire ("  GESTE 3 lancement de ping.exe : " + $(if ($lances | Where-Object { $_ -like '*ping.exe' }) { 'VU' } else { 'PAS VU' }) + " (lances : " + ($lances -join ', ') + ")")

  # 3. connexions de l'agent
  $conn = @()
  $champsNet = ''
  foreach ($e in ($ev | Where-Object { $_.ProviderName -eq 'Microsoft-Windows-Kernel-Network' })) {
    $c = Champs $e
    if (-not $champsNet) { $champsNet = ($c.Keys -join ',') }
    if ("$($c['PID'])" -eq "$agent") { $conn += ("id{0} {1}:{2}" -f $e.Id, $c['daddr'], $c['dport']) }
  }
  $uniq = @($conn | Select-Object -Unique)
  Dire ("  GESTE 4 connexion vers 1.1.1.1:443 : " + $(if ($uniq | Where-Object { $_ -match '1\.1\.1\.1|16843009' }) { 'VU' } else { 'PAS VU' }) + " (connexions : " + (($uniq | Select-Object -First 6) -join ' ; ') + ")")
  Dire ("  champs reseau : $champsNet")
} catch {
  Dire ('ERREUR : ' + $_.Exception.Message)
} finally {
  cmd /c "logman stop waly-garde-essai -ets >nul 2>&1"
  Remove-Item 'C:\waly\data\garde-essai.etl', 'C:\waly\data\garde-essai-ecrit.txt' -ErrorAction SilentlyContinue
  $out | Out-File $res -Encoding ascii
}
