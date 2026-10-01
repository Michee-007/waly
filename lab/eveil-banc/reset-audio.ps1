# Reset audio capture (elevated) - ASCII pur. Redemarre le graphe audio et
# recycle le peripherique de capture AMD coince (vecu 2026-07-21 : crash de
# process en pleine capture -> Access denied 0x80070005 pour toute nouvelle
# ouverture de flux).
$out = "C:\waly\lab\eveil-banc\reset-audio-result.txt"
$log = @()
try {
  Restart-Service AudioEndpointBuilder -Force -ErrorAction Stop
  $log += "AudioEndpointBuilder redemarre"
} catch { $log += "AudioEndpointBuilder: $_" }
Start-Sleep -Seconds 2
try {
  Start-Service Audiosrv -ErrorAction Stop
  $log += "Audiosrv: " + (Get-Service Audiosrv).Status
} catch { $log += "Audiosrv: $_" }
try {
  $dev = Get-PnpDevice -Class AudioEndpoint -ErrorAction Stop | Where-Object { $_.FriendlyName -like "*Microphone*" }
  foreach ($d in $dev) {
    Disable-PnpDevice -InstanceId $d.InstanceId -Confirm:$false -ErrorAction Stop
    Start-Sleep -Seconds 1
    Enable-PnpDevice -InstanceId $d.InstanceId -Confirm:$false -ErrorAction Stop
    $log += "recycle: " + $d.FriendlyName
  }
} catch { $log += "pnp: $_" }
$log -join "`r`n" | Out-File $out -Encoding utf8
