# Essai 3 (Garde, etape 4) : l'ENCLOS. Un compte Windows a part suffit-il a
# couper un dossier a un programme, et a le lui rendre ?
# Une seule demande UAC : la partie elevee cree le compte d'essai, attend,
# puis le supprime. TOUT le reste (lancer sous ce compte, regler les droits
# des dossiers, sonder) est fait SANS elevation, comme le fera l'app.
# Resultat : resultat-enclos.txt. ASCII pur.
param([switch]$Eleve, [string]$Dossier = 'C:\waly\data\enclos-banc')
$ErrorActionPreference = 'Stop'
$ici = Split-Path -Parent $MyInvocation.MyCommand.Path
$res = Join-Path $ici 'resultat-enclos.txt'
$compte = 'WalyEnclosBanc'
$mdpF = Join-Path $Dossier 'mdp.txt'
$pret = Join-Path $Dossier 'pret.txt'
$fini = Join-Path $Dossier 'fini.txt'
$net = Join-Path $Dossier 'nettoye.txt'

if ($Eleve) {
  # ---- partie elevee : creer, attendre, supprimer ----
  $note = ''
  try {
    $mdp = (Get-Content $mdpF -Raw).Trim()
    Remove-Item $mdpF -Force
    $sec = ConvertTo-SecureString $mdp -AsPlainText -Force
    if (Get-LocalUser -Name $compte -ErrorAction SilentlyContinue) { Remove-LocalUser -Name $compte }
    $t = [Diagnostics.Stopwatch]::StartNew()
    $null = New-LocalUser -Name $compte -Password $sec -PasswordNeverExpires -UserMayNotChangePassword -AccountNeverExpires -Description 'Essai enclos Waly (banc)'
    $note = "compte cree en $($t.ElapsedMilliseconds) ms, sans groupe"
    $sid = (Get-LocalUser -Name $compte).SID.Value
    # Peut-il ouvrir une session sans etre dans un groupe ?
    $cred = New-Object Management.Automation.PSCredential(".\$compte", $sec)
    try {
      $p = Start-Process "$env:SystemRoot\System32\cmd.exe" -Credential $cred -WorkingDirectory "$env:SystemRoot\System32" -ArgumentList '/c exit 0' -PassThru -WindowStyle Hidden
      $null = $p.Handle; $null = $p.WaitForExit(60000)
      $note += ' ; session ouverte sans groupe'
    } catch {
      Add-LocalGroupMember -SID 'S-1-5-32-545' -Member $compte
      $note += ' ; SANS GROUPE REFUSE (' + $_.Exception.Message + ') -> ajoute a Utilisateurs'
    }
    "$sid|$note" | Out-File $pret -Encoding ascii
    $fin = (Get-Date).AddMinutes(8)
    while (-not (Test-Path $fini) -and (Get-Date) -lt $fin) { Start-Sleep -Milliseconds 400 }
  } catch {
    ('ERREUR|' + $_.Exception.Message) | Out-File $pret -Encoding ascii
  } finally {
    $n = @()
    try { Get-Process -IncludeUserName -ErrorAction SilentlyContinue | Where-Object { $_.UserName -like "*\$compte" } | Stop-Process -Force -ErrorAction SilentlyContinue } catch {}
    Start-Sleep -Milliseconds 800
    try {
      $s = (Get-LocalUser -Name $compte -ErrorAction Stop).SID.Value
      $prof = Get-CimInstance Win32_UserProfile | Where-Object { $_.SID -eq $s }
      if ($prof) { $n += "profil $($prof.LocalPath)"; $prof | Remove-CimInstance -ErrorAction SilentlyContinue }
      Remove-LocalUser -Name $compte
      $n += 'compte supprime'
    } catch { $n += 'nettoyage : ' + $_.Exception.Message }
    ($n -join ' ; ') | Out-File $net -Encoding ascii
  }
  exit
}

# ---- partie ordinaire (sans elevation) ----
$out = New-Object System.Collections.Generic.List[string]
$sys = "$env:SystemRoot\System32"
$null = New-Item -ItemType Directory -Force $Dossier
Remove-Item $res, $pret, $fini, $net -ErrorAction SilentlyContinue
$alpha = 'abcdefghijkmnpqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ23456789'
$mdp = 'Wy!9' + (-join (1..28 | ForEach-Object { $alpha[(Get-Random -Maximum $alpha.Length)] }))
$mdp | Out-File $mdpF -Encoding ascii
$dansProfil = Join-Path $env:USERPROFILE 'waly-enclos-essai'
$horsProfil = Join-Path $Dossier 'hors-profil'
try {
  try {
    Start-Process powershell.exe -Verb RunAs -WindowStyle Hidden -ArgumentList @(
      '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', ('"' + $MyInvocation.MyCommand.Path + '"'), '-Eleve', '-Dossier', ('"' + $Dossier + '"'))
  } catch { throw ('REFUS OU ECHEC UAC : ' + $_.Exception.Message) }
  $fin = (Get-Date).AddSeconds(150)
  while (-not (Test-Path $pret) -and (Get-Date) -lt $fin) { Start-Sleep -Milliseconds 300 }
  if (-not (Test-Path $pret)) { throw 'la partie elevee n a rien ecrit en 150 s (UAC non accepte ?)' }
  Start-Sleep -Milliseconds 300
  $l = (Get-Content $pret -Raw).Trim() -split '\|', 2
  if ($l[0] -eq 'ERREUR') { throw ('partie elevee : ' + $l[1]) }
  $sid = $l[0]
  $out.Add("COMPTE : $compte ($sid) ; $($l[1])")
  $out.Add("Appelant : $(whoami) ; eleve = $(([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator))")

  $cred = New-Object Management.Automation.PSCredential(".\$compte", (ConvertTo-SecureString $mdp -AsPlainText -Force))
  function Lance([string]$exe, [string]$arg, [switch]$Profil) {
    try {
      $a = @{ FilePath = $exe; Credential = $cred; WorkingDirectory = $sys; PassThru = $true; WindowStyle = 'Hidden' }
      if ($arg) { $a.ArgumentList = $arg }
      if ($Profil) { $a.LoadUserProfile = $true }
      $p = Start-Process @a
      $null = $p.Handle
      if (-not $p.WaitForExit(60000)) { return 'DELAI' }
      return [string]$p.ExitCode
    } catch { return 'ECHEC ' + $_.Exception.Message }
  }
  # Le code de sortie d'un processus lance sous un autre compte revient vide
  # dans PowerShell 5.1 : la sonde l'ecrit elle-meme dans un fichier public.
  $marque = Join-Path $env:PUBLIC 'waly-enclos-code.txt'
  function Code([string]$interieur) {
    Remove-Item $marque -Force -ErrorAction SilentlyContinue
    $r = Lance "$sys\cmd.exe" ('/v:on /c "' + $interieur + ' & echo !errorlevel!>' + $marque + '"') -Profil
    if ($r -like 'ECHEC*' -or $r -eq 'DELAI') { return $r }
    if (Test-Path $marque) { $c = (Get-Content $marque -Raw).Trim(); Remove-Item $marque -Force -ErrorAction SilentlyContinue; return $c }
    return 'sans-reponse'
  }
  function Lire([string]$d) { $r = Code ('dir /a "' + $d + '" >nul 2>&1'); if ($r -eq '0') { 'LIT' } else { "refus($r)" } }
  function Ecrire([string]$d) {
    $f = Join-Path $d '~waly-essai.tmp'
    $r = Code ('copy /y nul "' + $f + '" >nul 2>&1')
    Remove-Item $f -Force -ErrorAction SilentlyContinue
    if ($r -eq '0') { 'ECRIT' } else { "refus($r)" }
  }
  function Droits([string]$d, [string[]]$a) { $o = & "$sys\icacls.exe" $d @a 2>&1; if ($LASTEXITCODE -ne 0) { throw "icacls $d $a : $o" } }

  # T0 : lancement sans elevation, duree, identite, enfants
  $t = [Diagnostics.Stopwatch]::StartNew(); $r = Lance "$sys\cmd.exe" '/c exit 7'; $r = Code 'cmd /c exit 7'; $out.Add("T0 lancement SANS elevation : code $r en $($t.ElapsedMilliseconds) ms (attendu 7)")
  $t.Restart(); $r = Lance "$sys\cmd.exe" '/c exit 7' -Profil; $out.Add("T0 lancement avec profil (1re fois, profil cree) : code $r en $($t.ElapsedMilliseconds) ms")
  $t.Restart(); $r = Lance "$sys\cmd.exe" '/c exit 7' -Profil; $out.Add("T0 lancement avec profil (2e fois) : code $r en $($t.ElapsedMilliseconds) ms")
  $pub = Join-Path $env:PUBLIC 'waly-enclos-essai.txt'
  Remove-Item $pub -ErrorAction SilentlyContinue
  $null = Lance "$sys\cmd.exe" ('/c whoami > "' + $pub + '" & cmd /c "cmd /c whoami" >> "' + $pub + '" & echo %USERPROFILE% >> "' + $pub + '"') -Profil
  $out.Add('T0 identite, enfant de l enfant, profil : ' + ((Get-Content $pub -ErrorAction SilentlyContinue) -join ' | '))
  Remove-Item $pub -ErrorAction SilentlyContinue

  # T1 : le profil de l'utilisateur, sans rien regler
  foreach ($d in @($env:USERPROFILE, (Join-Path $env:USERPROFILE 'Documents'), (Join-Path $env:USERPROFILE 'Desktop'), (Join-Path $env:USERPROFILE '.ssh'), (Join-Path $env:LOCALAPPDATA 'Programs'))) {
    if (Test-Path $d) { $out.Add("T1 profil, rien de regle : $d -> $(Lire $d)") }
  }
  # T2 : hors du profil, sans rien regler
  $null = New-Item -ItemType Directory -Force $horsProfil; 'x' | Out-File (Join-Path $horsProfil 'f.txt')
  foreach ($d in @('C:\waly\docs', $horsProfil, 'C:\Program Files', $env:PUBLIC)) { $out.Add("T2 hors profil, rien de regle : $d -> $(Lire $d) / $(Ecrire $d)") }

  # T3 : donner puis reprendre un dossier du profil
  $null = New-Item -ItemType Directory -Force (Join-Path $dansProfil 'sous'); 'x' | Out-File (Join-Path $dansProfil 'sous\f.txt')
  $out.Add("T3 dossier du profil, avant : $(Lire $dansProfil) / $(Ecrire $dansProfil)")
  $t.Restart(); Droits $dansProfil @('/grant', "*${sid}:(OI)(CI)RX"); $ms = $t.ElapsedMilliseconds
  $out.Add("T3 donne en LECTURE ($ms ms) : $(Lire $dansProfil) / $(Ecrire $dansProfil) ; sous-dossier $(Lire (Join-Path $dansProfil 'sous'))")
  Droits $dansProfil @('/grant:r', "*${sid}:(OI)(CI)M")
  $out.Add("T3 donne en ECRITURE : $(Lire $dansProfil) / $(Ecrire $dansProfil)")
  Droits $dansProfil @('/remove', "*${sid}")
  $out.Add("T3 repris : $(Lire $dansProfil) / $(Ecrire $dansProfil)")

  # T4 : couper puis rendre un dossier hors profil
  $out.Add("T4 hors profil, avant : $(Lire $horsProfil) / $(Ecrire $horsProfil)")
  $t.Restart(); Droits $horsProfil @('/deny', "*${sid}:(OI)(CI)F"); $ms = $t.ElapsedMilliseconds
  $out.Add("T4 COUPE ($ms ms) : $(Lire $horsProfil) / $(Ecrire $horsProfil)")
  Droits $horsProfil @('/remove:d', "*${sid}")
  $out.Add("T4 rendu : $(Lire $horsProfil) / $(Ecrire $horsProfil)")
  # cout sur un gros dossier (le depot) : poser puis retirer une coupure
  $n = [IO.Directory]::GetFileSystemEntries('C:\waly\crates', '*', 'AllDirectories').Count
  $t.Restart(); Droits 'C:\waly\crates' @('/deny', "*${sid}:(OI)(CI)F"); $a = $t.ElapsedMilliseconds
  $r = Lire 'C:\waly\crates\waly-core\src'
  $t.Restart(); Droits 'C:\waly\crates' @('/remove:d', "*${sid}"); $b = $t.ElapsedMilliseconds
  $out.Add("T4 gros dossier C:\waly\crates ($n entrees) : coupe en $a ms -> sous-dossier $r ; rendu en $b ms -> $(Lire 'C:\waly\crates\waly-core\src')")

  # T5 : un programme installe DANS le profil (cas d'Ollama, de Claude...)
  $prog = Join-Path $dansProfil 'prog'; $null = New-Item -ItemType Directory -Force $prog
  Copy-Item "$sys\whoami.exe" (Join-Path $prog 'agent.exe')
  $out.Add("T5 programme du profil, avant : code $(Code ('"' + (Join-Path $prog 'agent.exe') + '" >nul 2>&1')) (0 = lance)")
  Droits $prog @('/grant', "*${sid}:(OI)(CI)RX")
  $out.Add("T5 programme du profil, dossier donne en lecture : code $(Code ('"' + (Join-Path $prog 'agent.exe') + '" >nul 2>&1'))")
  Droits $prog @('/remove', "*${sid}")

  # T6 : le reseau (rien n'est regle : il doit sortir)
  $out.Add("T6 reseau depuis l'enclos (curl https://1.1.1.1, 0 = sorti) : code $(Code 'curl.exe -s -o nul -m 6 https://1.1.1.1')")
  # T7 : peut-il defaire une coupure lui-meme ?
  Droits $horsProfil @('/deny', "*${sid}:(OI)(CI)F")
  $r = Code ('icacls "' + $horsProfil + '" /remove:d *' + $sid + ' >nul 2>&1')
  $out.Add("T7 l'enclos tente de lever sa propre coupure : icacls code $r ; ensuite $(Lire $horsProfil)")
  Droits $horsProfil @('/remove:d', "*${sid}")
  # T8 : voit-on ses processus depuis le compte ordinaire ?
  $p = Start-Process "$sys\ping.exe" -Credential $cred -WorkingDirectory $sys -ArgumentList '-n 6 127.0.0.1' -PassThru -WindowStyle Hidden
  Start-Sleep -Milliseconds 700
  $vu = Get-CimInstance Win32_Process -Filter "ProcessId=$($p.Id)" -ErrorAction SilentlyContinue
  $chemin = try { (Get-Process -Id $p.Id).Path } catch { 'illisible' }
  $out.Add("T8 processus de l'enclos vu d'ici : nom=$($vu.Name) ; chemin=$chemin ; ligne=$($vu.CommandLine)")
  try { $p.Kill(); $out.Add('T8 arret par la poignee du lanceur : OK') } catch { $out.Add('T8 arret : ' + $_.Exception.Message) }
} catch {
  $out.Add('ERREUR : ' + $_.Exception.Message)
} finally {
  Remove-Item $mdpF -Force -ErrorAction SilentlyContinue
  'fini' | Out-File $fini -Encoding ascii
  $fin = (Get-Date).AddSeconds(40)
  while (-not (Test-Path $net) -and (Get-Date) -lt $fin) { Start-Sleep -Milliseconds 300 }
  $out.Add('NETTOYAGE : ' + $(if (Test-Path $net) { (Get-Content $net -Raw).Trim() } else { 'pas de retour de la partie elevee' }))
  Remove-Item $dansProfil -Recurse -Force -ErrorAction SilentlyContinue
  Remove-Item $Dossier -Recurse -Force -ErrorAction SilentlyContinue
  $out | Out-File $res -Encoding ascii
}
Get-Content $res
